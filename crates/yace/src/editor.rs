//! Editor groups, views and the split layout (spec §10).
//!
//! Documents live in a [`DocumentStore`] and are shared by every view that
//! shows them; each [`View`] has its own selection and scroll position.

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
};

use ratatui::layout::Rect;
use yace_buffer::{ChangeSet, Document, Selection};
use yace_workspace::session::SplitAxis;

pub type DocId = u32;
pub type GroupId = u32;

#[derive(Debug)]
pub struct DocEntry {
    pub doc: Document,
    /// `Untitled-N` number for documents without a path.
    pub untitled: Option<u32>,
    /// Key used for lookup by path (canonical when possible).
    pub key: Option<PathBuf>,
    /// Being loaded in the background (large file); read-only until done.
    pub loading: bool,
    /// Cursor (line, column) to apply once loading finishes.
    pub pending_cursor: Option<(usize, usize)>,
}

#[derive(Debug, Default)]
pub struct DocumentStore {
    docs: HashMap<DocId, DocEntry>,
    by_key: HashMap<PathBuf, DocId>,
    next_id: DocId,
    next_untitled: u32,
}

/// Lookup key for a path: canonical if it exists, else absolute.
pub fn path_key(p: &Path) -> PathBuf {
    std::fs::canonicalize(p).unwrap_or_else(|_| yace_workspace::paths::absolutize(p))
}

impl DocumentStore {
    pub fn insert(&mut self, doc: Document) -> DocId {
        self.next_id += 1;
        let id = self.next_id;
        let key = doc.path().map(path_key);
        let untitled = if doc.path().is_none() {
            self.next_untitled += 1;
            Some(self.next_untitled)
        } else {
            None
        };
        if let Some(k) = &key {
            self.by_key.insert(k.clone(), id);
        }
        self.docs.insert(id, DocEntry { doc, untitled, key, loading: false, pending_cursor: None });
        id
    }
    pub fn get(&self, id: DocId) -> Option<&DocEntry> {
        self.docs.get(&id)
    }
    pub fn get_mut(&mut self, id: DocId) -> Option<&mut DocEntry> {
        self.docs.get_mut(&id)
    }
    pub fn doc(&self, id: DocId) -> Option<&Document> {
        self.docs.get(&id).map(|e| &e.doc)
    }
    pub fn doc_mut(&mut self, id: DocId) -> Option<&mut Document> {
        self.docs.get_mut(&id).filter(|e| !e.loading).map(|e| &mut e.doc)
    }
    pub fn is_loading(&self, id: DocId) -> bool {
        self.docs.get(&id).is_some_and(|e| e.loading)
    }
    pub fn find_by_path(&self, p: &Path) -> Option<DocId> {
        self.by_key.get(&path_key(p)).copied()
    }
    pub fn remove(&mut self, id: DocId) -> Option<DocEntry> {
        let e = self.docs.remove(&id)?;
        if let Some(k) = &e.key {
            self.by_key.remove(k);
        }
        Some(e)
    }
    /// Re-key after Save As / rename.
    pub fn rekey(&mut self, id: DocId) {
        if let Some(e) = self.docs.get_mut(&id) {
            if let Some(k) = e.key.take() {
                self.by_key.remove(&k);
            }
            e.key = e.doc.path().map(path_key);
            if let Some(k) = &e.key {
                self.by_key.insert(k.clone(), id);
                e.untitled = None;
            }
        }
    }
    pub fn ids(&self) -> Vec<DocId> {
        let mut v: Vec<DocId> = self.docs.keys().copied().collect();
        v.sort_unstable();
        v
    }
    pub fn iter(&self) -> impl Iterator<Item = (DocId, &DocEntry)> {
        self.docs.iter().map(|(k, v)| (*k, v))
    }
    pub fn len(&self) -> usize {
        self.docs.len()
    }
    pub fn is_empty(&self) -> bool {
        self.docs.is_empty()
    }
    /// Display name: file name or `Untitled-N`.
    pub fn name(&self, id: DocId) -> String {
        match self.docs.get(&id) {
            Some(e) => match e.doc.path() {
                Some(p) => p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
                None => format!("Untitled-{}", e.untitled.unwrap_or(1)),
            },
            None => String::new(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct View {
    pub doc: DocId,
    pub sel: Selection,
    pub scroll_line: usize,
    /// Wrapped row of `scroll_line` shown at the top (word wrap).
    pub scroll_sub: usize,
    /// Horizontal scroll in display cells.
    pub scroll_col: usize,
    pub preview: bool,
    /// Collapsed regions (per view, as in VS Code).
    pub folds: Vec<crate::display::Fold>,
}

impl View {
    pub fn new(doc: DocId) -> Self {
        Self {
            doc,
            sel: Selection::point(0),
            scroll_line: 0,
            scroll_sub: 0,
            scroll_col: 0,
            preview: false,
            folds: Vec::new(),
        }
    }

    pub fn top(&self) -> crate::display::Row {
        crate::display::Row { line: self.scroll_line, sub: self.scroll_sub }
    }

    pub fn set_top(&mut self, r: crate::display::Row) {
        self.scroll_line = r.line;
        self.scroll_sub = r.sub;
    }
}

#[derive(Debug, Clone)]
pub struct Group {
    pub id: GroupId,
    pub views: Vec<View>,
    pub active: usize,
    /// Most recently used documents in this group (front = most recent).
    pub mru: Vec<DocId>,
    /// Tab-bar horizontal scroll (first visible tab index).
    pub tab_scroll: usize,
}

impl Group {
    fn new(id: GroupId) -> Self {
        Self { id, views: Vec::new(), active: 0, mru: Vec::new(), tab_scroll: 0 }
    }
    pub fn active_view(&self) -> Option<&View> {
        self.views.get(self.active)
    }
    pub fn active_view_mut(&mut self) -> Option<&mut View> {
        self.views.get_mut(self.active)
    }
    pub fn find(&self, doc: DocId) -> Option<usize> {
        self.views.iter().position(|v| v.doc == doc)
    }
    pub fn activate(&mut self, idx: usize) {
        if idx < self.views.len() {
            self.active = idx;
            let d = self.views[idx].doc;
            self.mru.retain(|x| *x != d);
            self.mru.insert(0, d);
        }
    }
    /// Remove a view; returns it. Activates the most recently used remaining view.
    pub fn remove(&mut self, idx: usize) -> Option<View> {
        if idx >= self.views.len() {
            return None;
        }
        let v = self.views.remove(idx);
        self.mru.retain(|d| *d != v.doc);
        if self.views.is_empty() {
            self.active = 0;
        } else if let Some(next) = self.mru.first().and_then(|d| self.find(*d)) {
            self.active = next;
        } else {
            self.active = idx.min(self.views.len() - 1);
        }
        Some(v)
    }
    pub fn preview_index(&self) -> Option<usize> {
        self.views.iter().position(|v| v.preview)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Node {
    Leaf(GroupId),
    Split { axis: SplitAxis, children: Vec<Node>, ratios: Vec<f32> },
}

/// A draggable border between two split children.
#[derive(Debug, Clone, PartialEq)]
pub struct SplitHandle {
    /// Path of child indices from the root to the split node.
    pub path: Vec<usize>,
    /// The border lies after child `index`.
    pub index: usize,
    pub axis: SplitAxis,
    /// The whole split area (for converting mouse positions to ratios).
    pub area: Rect,
    /// The border cells.
    pub rect: Rect,
}

#[derive(Debug, Clone)]
pub struct Layout {
    pub root: Node,
    groups: Vec<Group>,
    pub active: GroupId,
    pub maximized: bool,
    next_group: GroupId,
}

impl Default for Layout {
    fn default() -> Self {
        Self { root: Node::Leaf(1), groups: vec![Group::new(1)], active: 1, maximized: false, next_group: 2 }
    }
}

impl Layout {
    pub fn groups_mut(&mut self) -> &mut [Group] {
        &mut self.groups
    }
    pub fn groups(&self) -> &[Group] {
        &self.groups
    }
    pub fn group(&self, id: GroupId) -> Option<&Group> {
        self.groups.iter().find(|g| g.id == id)
    }
    pub fn group_mut(&mut self, id: GroupId) -> Option<&mut Group> {
        self.groups.iter_mut().find(|g| g.id == id)
    }
    pub fn active_group(&self) -> &Group {
        self.group(self.active).unwrap_or(&self.groups[0])
    }
    pub fn active_group_mut(&mut self) -> &mut Group {
        let id = self.active;
        let idx = self.groups.iter().position(|g| g.id == id).unwrap_or(0);
        &mut self.groups[idx]
    }
    /// Groups in visual (left-to-right, top-to-bottom) order.
    pub fn order(&self) -> Vec<GroupId> {
        fn walk(n: &Node, out: &mut Vec<GroupId>) {
            match n {
                Node::Leaf(g) => out.push(*g),
                Node::Split { children, .. } => children.iter().for_each(|c| walk(c, out)),
            }
        }
        let mut out = Vec::new();
        walk(&self.root, &mut out);
        out
    }

    /// All views of `doc` in all groups (group id, view index).
    pub fn views_of(&self, doc: DocId) -> Vec<(GroupId, usize)> {
        self.groups
            .iter()
            .flat_map(|g| g.views.iter().enumerate().filter(|(_, v)| v.doc == doc).map(move |(i, _)| (g.id, i)))
            .collect()
    }

    pub fn all_views_mut(&mut self) -> impl Iterator<Item = &mut View> {
        self.groups.iter_mut().flat_map(|g| g.views.iter_mut())
    }

    /// Map every view of `doc` except `(skip_group, skip_view)` through `cs`.
    pub fn sync_views(&mut self, doc: DocId, cs: &ChangeSet, skip: Option<(GroupId, usize)>) {
        for g in &mut self.groups {
            for (i, v) in g.views.iter_mut().enumerate() {
                if v.doc != doc {
                    continue;
                }
                if skip != Some((g.id, i)) {
                    v.sel = v.sel.map(cs);
                }
                if !v.folds.is_empty() {
                    crate::display::map_folds(&mut v.folds, cs);
                }
            }
        }
    }

    /// Split the active group; the new group shows the same document.
    pub fn split(&mut self, axis: SplitAxis) -> GroupId {
        let id = self.next_group;
        self.next_group += 1;
        let mut g = Group::new(id);
        if let Some(v) = self.active_group().active_view().cloned() {
            g.views.push(View { preview: false, ..v });
            g.activate(0);
        }
        let target = self.active;
        insert_split(&mut self.root, target, id, axis);
        self.groups.push(g);
        self.active = id;
        self.maximized = false;
        id
    }

    /// Ensure a group exists to the right/below for "move to next group".
    pub fn next_group_or_split(&mut self, forward: bool) -> GroupId {
        let order = self.order();
        let pos = order.iter().position(|g| *g == self.active).unwrap_or(0);
        if forward {
            if let Some(g) = order.get(pos + 1) {
                return *g;
            }
            let current = self.active;
            let id = self.next_group;
            self.next_group += 1;
            insert_split(&mut self.root, current, id, SplitAxis::Horizontal);
            self.groups.push(Group::new(id));
            id
        } else if pos > 0 {
            order[pos - 1]
        } else {
            self.active
        }
    }

    /// Remove a group (its views are discarded by the caller first).
    pub fn close_group(&mut self, id: GroupId) {
        if self.groups.len() <= 1 {
            return;
        }
        remove_leaf(&mut self.root, id);
        collapse(&mut self.root);
        self.groups.retain(|g| g.id != id);
        if self.active == id {
            self.active = self.order().first().copied().unwrap_or(self.groups[0].id);
        }
        self.maximized = false;
    }

    /// Give every split's children equal sizes (View: Reset Layout,
    /// Even Editor Group Sizes).
    pub fn even_sizes(&mut self) {
        fn walk(n: &mut Node) {
            if let Node::Split { children, ratios, .. } = n {
                let k = children.len().max(1) as f32;
                ratios.iter_mut().for_each(|r| *r = 1.0 / k);
                children.iter_mut().for_each(walk);
            }
        }
        walk(&mut self.root);
        self.maximized = false;
    }

    /// Reset to a single group containing every open view (deduplicated).
    pub fn reset(&mut self) {
        let mut merged = Group::new(self.order()[0]);
        for gid in self.order() {
            if let Some(g) = self.group(gid) {
                for v in &g.views {
                    if merged.find(v.doc).is_none() {
                        merged.views.push(v.clone());
                    }
                }
            }
        }
        merged.activate(0);
        self.root = Node::Leaf(merged.id);
        self.active = merged.id;
        self.groups = vec![merged];
        self.maximized = false;
    }

    /// Compute group rectangles and split handles for `area`.
    pub fn rects(&self, area: Rect) -> (Vec<(GroupId, Rect)>, Vec<SplitHandle>) {
        let mut out = Vec::new();
        let mut handles = Vec::new();
        if self.maximized {
            out.push((self.active, area));
            return (out, handles);
        }
        layout_node(&self.root, area, &mut Vec::new(), &mut out, &mut handles);
        (out, handles)
    }

    /// Set the ratio for a handle from a mouse position.
    pub fn drag_handle(&mut self, h: &SplitHandle, col: u16, row: u16) {
        let Some(Node::Split { ratios, .. }) = node_at_mut(&mut self.root, &h.path) else { return };
        let (pos, start, len) = match h.axis {
            SplitAxis::Horizontal => (col, h.area.x, h.area.width),
            SplitAxis::Vertical => (row, h.area.y, h.area.height),
        };
        if len < 4 {
            return;
        }
        let before: f32 = ratios[..h.index].iter().sum();
        let pair = ratios[h.index] + ratios[h.index + 1];
        let frac = (f32::from(pos.saturating_sub(start)) / f32::from(len)).clamp(0.0, 1.0);
        let min = 0.08_f32.min(pair / 2.0);
        let first = (frac - before).clamp(min, pair - min);
        ratios[h.index] = first;
        ratios[h.index + 1] = pair - first;
    }

    /// Grow/shrink the active group by `delta` (fraction of its parent).
    pub fn resize_active(&mut self, delta: f32) {
        let target = self.active;
        resize_in(&mut self.root, target, delta);
    }

    /// Rebuild from saved state; `make_group` returns (views, active).
    pub fn from_state(
        state: &yace_workspace::session::LayoutState,
        mut make_group: impl FnMut(&yace_workspace::session::GroupState) -> (Vec<View>, usize),
    ) -> Self {
        use yace_workspace::session::LayoutState as LS;
        let mut layout = Layout { root: Node::Leaf(0), groups: Vec::new(), active: 1, maximized: false, next_group: 1 };
        fn build(
            s: &LS,
            l: &mut Layout,
            f: &mut dyn FnMut(&yace_workspace::session::GroupState) -> (Vec<View>, usize),
        ) -> Node {
            match s {
                LS::Group(gs) => {
                    let id = l.next_group;
                    l.next_group += 1;
                    let (views, active) = f(gs);
                    let mut g = Group::new(id);
                    g.views = views;
                    if !g.views.is_empty() {
                        // Seed MRU so closing tabs activates sensibly.
                        for i in (0..g.views.len()).rev() {
                            g.activate(i);
                        }
                        g.activate(active.min(g.views.len() - 1));
                    }
                    l.groups.push(g);
                    Node::Leaf(id)
                }
                LS::Split { axis, ratios, children } => {
                    let kids: Vec<Node> = children.iter().map(|c| build(c, l, f)).collect();
                    let mut r = ratios.clone();
                    if r.len() != kids.len() || r.iter().any(|x| !x.is_finite() || *x <= 0.0) {
                        r = vec![1.0 / kids.len().max(1) as f32; kids.len()];
                    }
                    let sum: f32 = r.iter().sum();
                    Node::Split { axis: *axis, children: kids, ratios: r.iter().map(|x| x / sum).collect() }
                }
            }
        }
        layout.root = build(state, &mut layout, &mut make_group);
        collapse(&mut layout.root);
        if layout.groups.is_empty() {
            return Layout::default();
        }
        layout.active = layout.order()[0];
        layout
    }

    pub fn to_state(
        &self,
        mut tab: impl FnMut(&View) -> Option<yace_workspace::session::TabState>,
    ) -> yace_workspace::session::LayoutState {
        use yace_workspace::session::{GroupState, LayoutState as LS};
        fn walk(n: &Node, l: &Layout, tab: &mut dyn FnMut(&View) -> Option<yace_workspace::session::TabState>) -> LS {
            match n {
                Node::Leaf(id) => {
                    let g = l.group(*id);
                    let (tabs, active) = g
                        .map(|g| {
                            let mut tabs = Vec::new();
                            let mut active = 0;
                            for (i, v) in g.views.iter().enumerate() {
                                if let Some(t) = tab(v) {
                                    if i == g.active {
                                        active = tabs.len();
                                    }
                                    tabs.push(t);
                                }
                            }
                            (tabs, active)
                        })
                        .unwrap_or_default();
                    LS::Group(GroupState { tabs, active })
                }
                Node::Split { axis, children, ratios } => LS::Split {
                    axis: *axis,
                    ratios: ratios.clone(),
                    children: children.iter().map(|c| walk(c, l, tab)).collect(),
                },
            }
        }
        walk(&self.root, self, &mut tab)
    }
}

fn insert_split(node: &mut Node, target: GroupId, new: GroupId, axis: SplitAxis) -> bool {
    match node {
        Node::Leaf(g) if *g == target => {
            *node = Node::Split { axis, children: vec![Node::Leaf(target), Node::Leaf(new)], ratios: vec![0.5, 0.5] };
            true
        }
        Node::Leaf(_) => false,
        Node::Split { axis: a, children, ratios } => {
            if let Some(i) = children.iter().position(|c| *c == Node::Leaf(target))
                && *a == axis
            {
                // Same direction: add a sibling and rebalance evenly.
                children.insert(i + 1, Node::Leaf(new));
                let n = children.len() as f32;
                *ratios = vec![1.0 / n; children.len()];
                return true;
            }
            children.iter_mut().any(|c| insert_split(c, target, new, axis))
        }
    }
}

fn remove_leaf(node: &mut Node, id: GroupId) -> bool {
    if let Node::Split { children, ratios, .. } = node {
        if let Some(i) = children.iter().position(|c| *c == Node::Leaf(id)) {
            children.remove(i);
            let r = ratios.remove(i);
            let n = ratios.len().max(1);
            for x in ratios.iter_mut() {
                *x += r / n as f32;
            }
            return true;
        }
        return children.iter_mut().any(|c| remove_leaf(c, id));
    }
    false
}

fn collapse(node: &mut Node) {
    if let Node::Split { children, .. } = node {
        for c in children.iter_mut() {
            collapse(c);
        }
        if children.len() == 1 {
            *node = children.remove(0);
        }
    }
}

fn node_at_mut<'a>(node: &'a mut Node, path: &[usize]) -> Option<&'a mut Node> {
    match path.split_first() {
        None => Some(node),
        Some((i, rest)) => match node {
            Node::Split { children, .. } => node_at_mut(children.get_mut(*i)?, rest),
            Node::Leaf(_) => None,
        },
    }
}

fn resize_in(node: &mut Node, target: GroupId, delta: f32) -> bool {
    if let Node::Split { children, ratios, .. } = node {
        if let Some(i) = children.iter().position(|c| *c == Node::Leaf(target)) {
            if children.len() < 2 {
                return false;
            }
            let j = if i + 1 < children.len() { i + 1 } else { i - 1 };
            let pair = ratios[i] + ratios[j];
            let new = (ratios[i] + delta).clamp(0.1 * pair, 0.9 * pair);
            ratios[j] = pair - new;
            ratios[i] = new;
            return true;
        }
        return children.iter_mut().any(|c| resize_in(c, target, delta));
    }
    false
}

fn layout_node(
    node: &Node,
    area: Rect,
    path: &mut Vec<usize>,
    out: &mut Vec<(GroupId, Rect)>,
    handles: &mut Vec<SplitHandle>,
) {
    match node {
        Node::Leaf(g) => out.push((*g, area)),
        Node::Split { axis, children, ratios } => {
            let n = children.len() as u16;
            let total = match axis {
                SplitAxis::Horizontal => area.width,
                SplitAxis::Vertical => area.height,
            };
            // One cell of border between children.
            let usable = total.saturating_sub(n.saturating_sub(1));
            let mut offset = 0u16;
            for (i, child) in children.iter().enumerate() {
                let size = if i + 1 == children.len() {
                    usable.saturating_sub(offset.saturating_sub(i as u16))
                } else {
                    ((f32::from(usable) * ratios[i]).round() as u16).max(1)
                };
                let start = offset;
                let rect = match axis {
                    SplitAxis::Horizontal => Rect {
                        x: area.x + start,
                        y: area.y,
                        width: size.min(total.saturating_sub(start)),
                        height: area.height,
                    },
                    SplitAxis::Vertical => Rect {
                        x: area.x,
                        y: area.y + start,
                        width: area.width,
                        height: size.min(total.saturating_sub(start)),
                    },
                };
                path.push(i);
                layout_node(child, rect, path, out, handles);
                path.pop();
                offset = start + size;
                if i + 1 < children.len() {
                    let border = match axis {
                        SplitAxis::Horizontal => Rect { x: area.x + offset, y: area.y, width: 1, height: area.height },
                        SplitAxis::Vertical => Rect { x: area.x, y: area.y + offset, width: area.width, height: 1 },
                    };
                    handles.push(SplitHandle { path: path.clone(), index: i, axis: *axis, area, rect: border });
                    offset += 1;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_close_and_rects() {
        let mut l = Layout::default();
        l.active_group_mut().views.push(View::new(1));
        l.active_group_mut().activate(0);
        let g2 = l.split(SplitAxis::Horizontal);
        assert_eq!(l.order(), vec![1, g2]);
        assert_eq!(l.group(g2).unwrap().views[0].doc, 1, "split shows the same document");
        let g3 = l.split(SplitAxis::Vertical);
        assert_eq!(l.order(), vec![1, g2, g3]);
        let (rects, handles) = l.rects(Rect::new(0, 0, 81, 40));
        assert_eq!(rects.len(), 3);
        assert_eq!(handles.len(), 2);
        assert_eq!(rects[0].1.width + 1 + rects[1].1.width, 81);
        l.close_group(g2);
        assert_eq!(l.order(), vec![1, g3]);
        l.close_group(g3);
        assert_eq!(l.order(), vec![1]);
        assert_eq!(l.root, Node::Leaf(1));
    }

    #[test]
    fn drag_handle_changes_ratio() {
        let mut l = Layout::default();
        l.split(SplitAxis::Horizontal);
        let (_, handles) = l.rects(Rect::new(0, 0, 100, 10));
        l.drag_handle(&handles[0], 25, 5);
        let (rects, _) = l.rects(Rect::new(0, 0, 100, 10));
        assert!(rects[0].1.width < 30, "{rects:?}");
    }

    #[test]
    fn state_roundtrip() {
        let mut l = Layout::default();
        l.active_group_mut().views.push(View::new(7));
        l.split(SplitAxis::Vertical);
        let st = l.to_state(|v| {
            Some(yace_workspace::session::TabState { path: format!("/f{}", v.doc).into(), ..Default::default() })
        });
        let back = Layout::from_state(&st, |gs| (gs.tabs.iter().map(|_| View::new(9)).collect(), gs.active));
        assert_eq!(back.order().len(), 2);
        assert!(matches!(back.root, Node::Split { axis: SplitAxis::Vertical, .. }));
    }

    #[test]
    fn group_mru_activation_on_close() {
        let mut g = Group::new(1);
        for d in 1..=3 {
            g.views.push(View::new(d));
        }
        g.activate(0);
        g.activate(2);
        g.activate(1);
        g.remove(1);
        assert_eq!(g.active_view().unwrap().doc, 3, "most recently used remaining");
    }
}
