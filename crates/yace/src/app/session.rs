//! Session save/restore (spec §4.9, Workflow G).

use std::path::Path;

use yace_buffer::{Range, Selection, text::line_without_ending};
use yace_workspace::{
    WorkspaceKind,
    session::{self, ExplorerRootState, TabState, WorkspaceState},
};

use super::{App, Focus, Severity};
use crate::editor::{Layout, View};

impl App {
    fn state_path(&self) -> std::path::PathBuf {
        session::state_file(&self.paths.workspaces_state_dir(), &self.workspace.identity())
    }

    /// Persist the session (and an untitled workspace's backing file).
    pub fn save_session(&mut self) {
        if self.workspace.roots().is_empty() {
            return;
        }
        if matches!(self.workspace.kind, WorkspaceKind::Untitled(_))
            && (self.workspace.dirty || self.workspace.file().is_some_and(|f| !f.exists()))
            && let Err(e) = self.workspace.save()
        {
            tracing::warn!("could not save untitled workspace: {e}");
        }
        let mut st = WorkspaceState {
            sidebar_visible: Some(self.sidebar_visible),
            sidebar_width: Some(self.sidebar_width),
            explorer_scroll: self.explorer.scroll,
            focus_explorer: self.focus == Focus::Explorer,
            recent_files: self.recent_files.iter().take(50).cloned().collect(),
            ..Default::default()
        };
        for (root, expanded, open) in self.explorer.expansion_state() {
            st.explorer.insert(root.display().to_string(), ExplorerRootState { collapsed: !expanded, expanded: open });
        }
        if let Some(r) = self.explorer.selected_row()
            && let Some(tree) = self.explorer.trees.iter().find(|t| t.id == r.root)
            && let Ok(rel) = r.path.strip_prefix(&tree.path)
        {
            st.explorer_selected = Some((tree.path.display().to_string(), rel.display().to_string()));
        }
        let docs = &self.docs;
        st.editor = Some(self.layout.to_state(|v| {
            let doc = docs.doc(v.doc)?;
            let path = doc.path()?.to_path_buf();
            let rope = doc.rope();
            let lc = |pos: usize| {
                let pos = pos.min(rope.len_chars());
                let line = rope.char_to_line(pos);
                (line, pos - rope.line_to_char(line))
            };
            let p = v.sel.primary();
            Some(TabState {
                path,
                preview: v.preview,
                cursor: lc(p.head),
                anchor: (!p.is_empty()).then(|| lc(p.anchor)),
                scroll_line: v.scroll_line,
                scroll_col: v.scroll_col,
            })
        }));
        st.active_group = self.layout.order().iter().position(|g| *g == self.layout.active).unwrap_or(0);
        if let Err(e) = session::save(&self.state_path(), &st) {
            tracing::warn!("could not save session: {e}");
        }
    }

    pub(crate) fn restore_session(&mut self) {
        let st = match session::load(&self.state_path()) {
            Ok(Some(st)) => st,
            Ok(None) => return,
            Err(msg) => {
                self.notify(Severity::Warning, msg);
                return;
            }
        };
        if let Some(v) = st.sidebar_visible {
            self.sidebar_visible = v;
        }
        if let Some(w) = st.sidebar_width {
            self.sidebar_width = w.max(10);
        }
        for (root, es) in &st.explorer {
            self.explorer.apply_expansion(Path::new(root), !es.collapsed, &es.expanded);
        }
        self.explorer.sync_roots(&self.workspace);
        if let Some((root, rel)) = &st.explorer_selected {
            let p = Path::new(root).join(rel);
            if !self.explorer.select_path(&p) {
                self.explorer.pending_reveal = Some(p);
            }
        }
        self.explorer.scroll = st.explorer_scroll;
        self.recent_files = st.recent_files.into_iter().filter(|p| p.exists()).collect();

        let mut missing = Vec::new();
        if let Some(layout_state) = &st.editor {
            // Open documents first, then build views from them.
            let mut opened: Vec<(std::path::PathBuf, Option<crate::editor::DocId>)> = Vec::new();
            let collect = |s: &yace_workspace::session::LayoutState, out: &mut Vec<TabState>| {
                fn walk(s: &yace_workspace::session::LayoutState, out: &mut Vec<TabState>) {
                    match s {
                        yace_workspace::session::LayoutState::Group(g) => out.extend(g.tabs.iter().cloned()),
                        yace_workspace::session::LayoutState::Split { children, .. } => {
                            children.iter().for_each(|c| walk(c, out))
                        }
                    }
                }
                walk(s, out);
            };
            let mut tabs = Vec::new();
            collect(layout_state, &mut tabs);
            for t in &tabs {
                if opened.iter().any(|(p, _)| *p == t.path) {
                    continue;
                }
                let id = if t.path.is_file() {
                    match yace_buffer::Document::open(&t.path, yace_buffer::LineEnding::Lf) {
                        Ok(d) => Some(self.docs.insert(d)),
                        Err(e) => {
                            missing.push(format!("{} ({e})", t.path.display()));
                            None
                        }
                    }
                } else {
                    missing.push(t.path.display().to_string());
                    None
                };
                opened.push((t.path.clone(), id));
            }
            let docs = &self.docs;
            let layout = Layout::from_state(layout_state, |gs| {
                let mut views = Vec::new();
                let mut active = 0;
                for (i, t) in gs.tabs.iter().enumerate() {
                    let Some(Some(id)) = opened.iter().find(|(p, _)| *p == t.path).map(|(_, id)| *id) else { continue };
                    let Some(doc) = docs.doc(id) else { continue };
                    let rope = doc.rope();
                    let pos = |(line, col): (usize, usize)| {
                        let line = line.min(rope.len_lines().saturating_sub(1));
                        rope.line_to_char(line) + col.min(line_without_ending(rope, line).len_chars())
                    };
                    let head = pos(t.cursor);
                    let anchor = t.anchor.map(pos).unwrap_or(head);
                    if i == gs.active {
                        active = views.len();
                    }
                    views.push(View {
                        doc: id,
                        sel: Selection::single(Range::new(anchor, head)),
                        scroll_line: t.scroll_line.min(rope.len_lines().saturating_sub(1)),
                        scroll_col: t.scroll_col,
                        preview: t.preview,
                    });
                }
                (views, active)
            });
            self.layout = layout;
            if let Some(g) = self.layout.order().get(st.active_group).copied() {
                self.layout.active = g;
            }
            // Drop documents that ended up without views.
            for id in self.docs.ids() {
                self.release_doc_if_unused(id);
            }
        }
        self.focus = if st.focus_explorer || self.active().is_none() { Focus::Explorer } else { Focus::Editor };
        if !missing.is_empty() {
            self.notify(
                Severity::Warning,
                format!(
                    "{} file(s) from the last session could not be reopened: {}",
                    missing.len(),
                    missing.join(", ")
                ),
            );
        }
    }
}
