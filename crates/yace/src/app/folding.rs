//! View geometry (folds + word wrap) and the folding commands.

use yace_buffer::{Range, Rope};
use yace_config::WordWrap;

use super::App;
use crate::{
    display::{DisplayMap, Fold},
    editor::{DocId, GroupId},
};

/// Everything needed to build a view's display map, owned so callers can
/// mutate the view afterwards (rope clones are O(1)).
pub(crate) struct ViewGeo {
    pub rope: Rope,
    pub folds: Vec<Fold>,
    pub wrap: Option<usize>,
    pub tab: usize,
    pub height: usize,
}

impl ViewGeo {
    pub fn map(&self) -> DisplayMap<'_> {
        DisplayMap::new(&self.rope, &self.folds, self.wrap, self.tab)
    }
}

/// Vertical movement by display rows (folds and wrapped lines aware).
pub(crate) fn display_vertical(map: &DisplayMap, rope: &Rope, r: Range, delta: isize, extend: bool) -> Range {
    let (row, col) = map.row_of(r.head);
    let goal = r.goal.unwrap_or(col);
    let target = map.advance(row, delta);
    let mut nr = if target == row {
        let edge = if delta < 0 { 0 } else { rope.len_chars() };
        if delta < 0 && !extend && !r.is_empty() { Range::point(r.from()) } else { r.with_head(edge, extend) }
    } else {
        r.with_head(map.char_at(target, goal), extend)
    };
    nr.goal = Some(goal);
    nr
}

impl App {
    /// Wrap width for a document shown `text_width` cells wide.
    pub fn wrap_width(&self, doc: DocId, text_width: usize) -> Option<usize> {
        let d = self.docs.doc(doc)?;
        if d.large_file {
            return None;
        }
        let e = &self.settings_for(d.path()).editor;
        let mode = match self.word_wrap_override {
            Some(false) => WordWrap::Off,
            Some(true) if e.word_wrap == WordWrap::Off => WordWrap::On,
            _ => e.word_wrap,
        };
        // Keep the last column for the scrollbar.
        let avail = text_width.saturating_sub(1).max(1);
        match mode {
            WordWrap::Off => None,
            WordWrap::On => Some(avail),
            WordWrap::WordWrapColumn => Some(e.word_wrap_column.max(1)),
            WordWrap::Bounded => Some(avail.min(e.word_wrap_column.max(1))),
        }
    }

    pub(crate) fn view_geo(&self, g: GroupId, v: usize) -> Option<ViewGeo> {
        let group = self.layout.group(g)?;
        let view = group.views.get(v)?;
        let doc = self.docs.doc(view.doc)?;
        let wl = crate::ui::workbench_layout(self);
        let gl = wl.groups.iter().find(|x| x.id == g);
        let (height, width) = gl.map(|gl| (gl.content.height as usize, gl.text_width as usize)).unwrap_or((20, 80));
        Some(ViewGeo {
            rope: doc.rope().clone(),
            folds: view.folds.clone(),
            wrap: self.wrap_width(view.doc, width),
            tab: self.settings_for(doc.path()).editor.tab_size.max(1),
            height,
        })
    }

    pub(crate) fn active_geo(&self) -> Option<(GroupId, usize, DocId, ViewGeo)> {
        let (g, v, d) = self.active()?;
        Some((g, v, d, self.view_geo(g, v)?))
    }

    /// Unfold regions that hide a cursor of view `(g, v)` (after a jump).
    pub(crate) fn reveal_cursors(&mut self, g: GroupId, v: usize) {
        let Some(view) = self.layout.group_mut(g).and_then(|gr| gr.views.get_mut(v)) else { return };
        if view.folds.is_empty() {
            return;
        }
        let heads: Vec<usize> = view.sel.ranges().iter().map(|r| r.head).collect();
        view.folds.retain(|f| !heads.iter().any(|h| *h > f.start && *h <= f.end));
    }

    /// The fold range with header `line` (syntax tree, else indentation).
    pub(crate) fn fold_range(&self, d: DocId, rope: &Rope, line: usize, tab: usize) -> Option<(usize, usize)> {
        yace_syntax::fold::range_at(self.syntax.tree(d), rope, line, tab)
    }

    fn folding_enabled(&self, d: DocId) -> bool {
        self.docs.doc(d).is_some_and(|doc| self.settings_for(doc.path()).editor.folding)
    }

    /// Fold the innermost unfolded region containing the cursor line.
    pub(crate) fn fold_at_cursor(&mut self) {
        let Some((g, v, d, geo)) = self.active_geo() else { return };
        if !self.folding_enabled(d) {
            return;
        }
        let rope = &geo.rope;
        let Some(view) = self.layout.group(g).and_then(|gr| gr.views.get(v)) else { return };
        let cl = rope.char_to_line(view.sel.primary().head.min(rope.len_chars()));
        let folded: Vec<(usize, usize)> = view.folds.iter().map(|f| f.lines(rope)).collect();
        let lo = cl.saturating_sub(20_000);
        let found = (lo..=cl).rev().find_map(|l| {
            let r = self.fold_range(d, rope, l, geo.tab)?;
            (r.0 <= cl && cl <= r.1.max(r.0) && !folded.contains(&r)).then_some(r)
        });
        if let Some((h, last)) = found
            && let Some(f) = Fold::from_lines(rope, h, last)
        {
            self.add_fold(g, v, f);
        }
    }

    fn add_fold(&mut self, g: GroupId, v: usize, f: Fold) {
        let Some(view) = self.layout.group_mut(g).and_then(|gr| gr.views.get_mut(v)) else { return };
        view.folds.push(f);
        // Cursors inside move to the end of the header line.
        let sel = view.sel.transform(|r| if r.head > f.start && r.head <= f.end { Range::point(f.start) } else { r });
        view.sel = sel;
        self.reveal_cursor = true;
    }

    pub(crate) fn unfold_at_cursor(&mut self) {
        let Some((g, v, _, geo)) = self.active_geo() else { return };
        let rope = geo.rope;
        let Some(view) = self.layout.group_mut(g).and_then(|gr| gr.views.get_mut(v)) else { return };
        let cl = rope.char_to_line(view.sel.primary().head.min(rope.len_chars()));
        view.folds.retain(|f| f.lines(&rope).0 != cl);
        self.reveal_cursor = true;
    }

    pub(crate) fn toggle_fold_at_cursor(&mut self) {
        let Some((g, v, _, geo)) = self.active_geo() else { return };
        let rope = &geo.rope;
        let Some(view) = self.layout.group(g).and_then(|gr| gr.views.get(v)) else { return };
        let cl = rope.char_to_line(view.sel.primary().head.min(rope.len_chars()));
        if view.folds.iter().any(|f| f.lines(rope).0 == cl) {
            self.unfold_at_cursor();
        } else {
            self.fold_at_cursor();
        }
    }

    /// Toggle the fold whose header is `line` (gutter click).
    pub(crate) fn toggle_fold_line(&mut self, g: GroupId, line: usize) -> bool {
        let Some(group) = self.layout.group(g) else { return false };
        let v = group.active;
        let Some(view) = group.views.get(v) else { return false };
        let d = view.doc;
        let Some(rope) = self.docs.doc(d).map(|x| x.rope().clone()) else { return false };
        if let Some(i) = view.folds.iter().position(|f| f.lines(&rope).0 == line) {
            if let Some(view) = self.layout.group_mut(g).and_then(|gr| gr.views.get_mut(v)) {
                view.folds.remove(i);
            }
            return true;
        }
        if !self.folding_enabled(d) {
            return false;
        }
        let tab = self.view_geo(g, v).map(|x| x.tab).unwrap_or(4);
        match self.fold_range(d, &rope, line, tab).and_then(|(h, l)| Fold::from_lines(&rope, h, l)) {
            Some(f) => {
                self.add_fold(g, v, f);
                true
            }
            None => false,
        }
    }

    pub(crate) fn fold_all(&mut self) {
        let Some((g, v, d, geo)) = self.active_geo() else { return };
        if !self.folding_enabled(d) {
            return;
        }
        let ranges = yace_syntax::fold::all_ranges(self.syntax.tree(d), &geo.rope, geo.tab);
        let folds: Vec<Fold> = ranges.into_iter().filter_map(|(h, l)| Fold::from_lines(&geo.rope, h, l)).collect();
        let Some(view) = self.layout.group_mut(g).and_then(|gr| gr.views.get_mut(v)) else { return };
        view.folds = folds;
        self.move_hidden_cursors(g, v, &geo);
    }

    /// Add folds (keeping the existing ones), then move hidden cursors.
    fn add_folds(&mut self, g: GroupId, v: usize, geo: &ViewGeo, ranges: impl IntoIterator<Item = (usize, usize)>) {
        let rope = &geo.rope;
        let Some(view) = self.layout.group_mut(g).and_then(|gr| gr.views.get_mut(v)) else { return };
        let have: Vec<(usize, usize)> = view.folds.iter().map(|f| f.lines(rope)).collect();
        for (h, l) in ranges {
            if !have.contains(&(h, l))
                && let Some(f) = Fold::from_lines(rope, h, l)
            {
                view.folds.push(f);
            }
        }
        view.folds.sort_by_key(|f| f.start);
        self.move_hidden_cursors(g, v, geo);
    }

    /// The fold range around the cursor line: the one whose header it is,
    /// else the innermost containing it (looking up to 20,000 lines back).
    fn range_around_cursor(&self, g: GroupId, v: usize, d: DocId, geo: &ViewGeo) -> Option<(usize, usize)> {
        let rope = &geo.rope;
        let view = self.layout.group(g)?.views.get(v)?;
        let cl = rope.char_to_line(view.sel.primary().head.min(rope.len_chars()));
        (cl.saturating_sub(20_000)..=cl).rev().find_map(|l| {
            let r = self.fold_range(d, rope, l, geo.tab)?;
            (r.0 <= cl && cl <= r.1).then_some(r)
        })
    }

    /// Fold Recursively: the region at the cursor and every region in it.
    pub(crate) fn fold_recursively(&mut self) {
        let Some((g, v, d, geo)) = self.active_geo() else { return };
        if !self.folding_enabled(d) {
            return;
        }
        let Some((h, l)) = self.range_around_cursor(g, v, d, &geo) else { return };
        let ranges: Vec<(usize, usize)> =
            (h..=l).filter_map(|line| self.fold_range(d, &geo.rope, line, geo.tab)).collect();
        self.add_folds(g, v, &geo, ranges);
    }

    /// Unfold Recursively: the region at the cursor and every region in it.
    pub(crate) fn unfold_recursively(&mut self) {
        let Some((g, v, d, geo)) = self.active_geo() else { return };
        let rope = &geo.rope;
        let Some(view) = self.layout.group(g).and_then(|gr| gr.views.get(v)) else { return };
        let cl = rope.char_to_line(view.sel.primary().head.min(rope.len_chars()));
        // A folded region whose header is the cursor line, else the region
        // around the cursor.
        let target = view
            .folds
            .iter()
            .map(|f| f.lines(rope))
            .find(|(h, _)| *h == cl)
            .or_else(|| self.range_around_cursor(g, v, d, &geo));
        let Some((h, l)) = target else { return };
        if let Some(view) = self.layout.group_mut(g).and_then(|gr| gr.views.get_mut(v)) {
            view.folds.retain(|f| {
                let (fh, _) = f.lines(rope);
                fh < h || fh > l
            });
        }
        self.reveal_cursor = true;
    }

    /// Fold Level N: every region nested `level` deep, except the ones
    /// holding a cursor (VS Code).
    pub(crate) fn fold_level(&mut self, level: usize) {
        let Some((g, v, d, geo)) = self.active_geo() else { return };
        if !self.folding_enabled(d) {
            return;
        }
        let rope = &geo.rope;
        let ranges = yace_syntax::fold::all_ranges(self.syntax.tree(d), rope, geo.tab);
        let levels = yace_syntax::fold::nesting_levels(&ranges);
        let cursors: Vec<usize> = self
            .layout
            .group(g)
            .and_then(|gr| gr.views.get(v))
            .map(|view| view.sel.ranges().iter().map(|r| rope.char_to_line(r.head.min(rope.len_chars()))).collect())
            .unwrap_or_default();
        let pick: Vec<(usize, usize)> = ranges
            .into_iter()
            .zip(levels)
            .filter(|((h, l), lv)| *lv == level && !cursors.iter().any(|c| h <= c && c <= l))
            .map(|(r, _)| r)
            .collect();
        self.add_folds(g, v, &geo, pick);
    }

    /// Fold All Block Comments.
    pub(crate) fn fold_all_block_comments(&mut self) {
        let Some((g, v, d, geo)) = self.active_geo() else { return };
        if !self.folding_enabled(d) {
            return;
        }
        let rope = &geo.rope;
        let tree = self.syntax.tree(d);
        let open = self.syntax.language(d).rules.block_comment.map(|(o, _)| o);
        let pick: Vec<(usize, usize)> = yace_syntax::fold::all_ranges(tree, rope, geo.tab)
            .into_iter()
            .filter(|(h, _)| yace_syntax::fold::is_comment_at(tree, rope, *h, open))
            .collect();
        self.add_folds(g, v, &geo, pick);
    }

    /// Move cursors hidden by folds to the end of their visible header.
    fn move_hidden_cursors(&mut self, g: GroupId, v: usize, geo: &ViewGeo) {
        let Some(view) = self.layout.group_mut(g).and_then(|gr| gr.views.get_mut(v)) else { return };
        let map = DisplayMap::new(&geo.rope, &view.folds, geo.wrap, geo.tab);
        let rope = &geo.rope;
        let sel = view.sel.transform(|r| {
            let line = rope.char_to_line(r.head.min(rope.len_chars()));
            if map.is_hidden(line) {
                let h = map.visible_line(line);
                Range::point(rope.line_to_char(h) + yace_buffer::text::line_without_ending(rope, h).len_chars())
            } else {
                r
            }
        });
        view.sel = sel;
        self.reveal_cursor = true;
    }

    pub(crate) fn unfold_all(&mut self) {
        let Some((g, v, _)) = self.active() else { return };
        if let Some(view) = self.layout.group_mut(g).and_then(|gr| gr.views.get_mut(v)) {
            view.folds.clear();
        }
        self.reveal_cursor = true;
    }

    pub(crate) fn toggle_word_wrap(&mut self) {
        let currently = self.active().is_some_and(|(_, _, d)| self.wrap_width(d, 80).is_some());
        self.word_wrap_override = Some(!currently);
        // Horizontal scroll makes no sense while wrapping.
        for g in self.layout.groups_mut() {
            for v in &mut g.views {
                v.scroll_col = 0;
                v.scroll_sub = 0;
            }
        }
        self.reveal_cursor = true;
    }

    /// Scroll a view by `delta` display rows.
    pub(crate) fn scroll_view(&mut self, g: GroupId, delta: isize) {
        let Some(v) = self.layout.group(g).map(|gr| gr.active) else { return };
        let Some(geo) = self.view_geo(g, v) else { return };
        let map = geo.map();
        let Some(view) = self.layout.group_mut(g).and_then(|gr| gr.views.get_mut(v)) else { return };
        let top = map.advance(view.top(), delta);
        view.set_top(top);
    }

    /// Folded (header, last) line pairs of the active view (tests, session).
    pub fn view_folds(&self) -> Vec<(usize, usize)> {
        let Some((g, v, d)) = self.active() else { return Vec::new() };
        let (Some(view), Some(doc)) = (self.layout.group(g).and_then(|gr| gr.views.get(v)), self.docs.doc(d)) else {
            return Vec::new();
        };
        view.folds.iter().map(|f| f.lines(doc.rope())).collect()
    }
}
