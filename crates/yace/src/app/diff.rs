//! Side-by-side diff tabs (spec §15.2): "Open Changes" and Source Control
//! show a file's two versions in aligned, read-only panes. The right side
//! is the open document when it is the working tree, so unsaved edits show
//! (the rows follow its edits). Narrow groups get the unified diff instead.

use std::{
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

use yace_buffer::Document;
use yace_git::GitError;
use yace_keymap::{KeyCode, KeyPress, Mods};

use super::{App, Severity};
use crate::{
    diff_view::{DiffView, build_rows},
    editor::{DocId, GroupId, View},
    git::{GitJob, Side},
};

/// Below this many text columns a diff opens as one unified document.
pub const SIDE_BY_SIDE_MIN_WIDTH: u16 = 100;
/// Rows are rebuilt this long after the compared document changes.
const REBUILD_DELAY: Duration = Duration::from_millis(150);

impl App {
    /// The diff shown by a document's tab.
    pub fn diff_of(&self, d: DocId) -> Option<&DiffView> {
        self.docs.get(d).and_then(|e| e.diff.as_deref())
    }

    fn diff_mut(&mut self, d: DocId) -> Option<&mut DiffView> {
        self.docs.get_mut(d).and_then(|e| e.diff.as_deref_mut())
    }

    /// The diff in the active group's tab.
    pub(crate) fn active_diff(&self) -> Option<DocId> {
        let (_, _, d) = self.active()?;
        self.diff_of(d).map(|_| d)
    }

    /// Whether the active group is wide enough for two panes.
    pub(crate) fn side_by_side_fits(&self) -> bool {
        let wl = crate::ui::workbench_layout(self);
        wl.groups.iter().find(|g| g.id == self.layout.active).is_some_and(|g| g.text_width >= SIDE_BY_SIDE_MIN_WIDTH)
    }

    /// Compare two versions of `rel` in repository `repo`.
    pub(crate) fn git_compare(&mut self, repo: usize, rel: &str, left: Side, right: Side) {
        let toplevel = self.git.repos[repo].info.toplevel.clone();
        let name = rel.trim_end_matches('/').rsplit('/').next().unwrap_or(rel);
        let title = format!("{name} ({} ↔ {})", left.label(), right.label());
        self.git_job(GitJob::Compare { toplevel, rel: rel.to_string(), left, right, title });
    }

    pub(crate) fn on_git_compare(
        &mut self,
        title: String,
        file: PathBuf,
        (left, right): (Side, Side),
        result: Result<(Option<String>, Option<String>), GitError>,
    ) {
        let (l, r) = match result {
            Ok(x) => x,
            Err(e) => {
                self.notify(Severity::Error, format!("Git: could not compare {title}: {e}"));
                return;
            }
        };
        let mut owned = Vec::new();
        let left_doc = self.hidden_version(l.unwrap_or_default(), &file, left.label());
        owned.push(left_doc);
        let right_doc = if right == Side::WorkingTree {
            match self.working_tree_doc(&file) {
                Some((d, opened)) => {
                    if opened {
                        owned.push(d);
                    }
                    d
                }
                None => {
                    self.release_hidden(&owned);
                    self.notify(Severity::Info, format!("{} is not a file in the working tree", file.display()));
                    return;
                }
            }
        } else {
            let d = self.hidden_version(r.unwrap_or_default(), &file, right.label());
            owned.push(d);
            d
        };
        let (Some(a), Some(b)) = (self.docs.doc(left_doc), self.docs.doc(right_doc)) else { return };
        if a.rope() == b.rope() {
            self.release_hidden(&owned);
            self.notify(Severity::Info, format!("No changes in {title}"));
            return;
        }
        let (rows, changes) = build_rows(a.rope(), b.rope());
        let versions = (a.version(), b.version());
        let cursor = changes.first().copied().unwrap_or(0);
        let view = DiffView {
            file: file.clone(),
            left_label: left.label().into(),
            right_label: right.label().into(),
            left: left_doc,
            right: right_doc,
            rows,
            changes,
            versions,
            cursor,
            top: cursor.saturating_sub(3),
            scroll_col: 0,
        };
        // One tab per comparison: opening it again replaces it.
        let existing = self.docs.ids().into_iter().find(|d| {
            self.diff_of(*d).is_some()
                && self.docs.get(*d).and_then(|e| e.virtual_doc.as_ref()).is_some_and(|v| v.title == title)
        });
        if let Some(d) = existing {
            self.close_doc_views(d);
        }
        let tab = self.docs.insert_virtual(Document::from_text(""), title, Some(file));
        if let Some(e) = self.docs.get_mut(tab) {
            e.diff = Some(Box::new(view));
        }
        let gid = self.layout.active;
        self.add_view(gid, View::new(tab));
        self.focus_editor();
    }

    /// A version of `file` shown on one side: a hidden read-only document
    /// coloured as the file's language.
    fn hidden_version(&mut self, text: String, file: &Path, label: &str) -> DocId {
        let name = file.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let d = self.docs.insert_virtual(Document::from_text(&text), format!("{name} ({label})"), Some(file.into()));
        if let Some(e) = self.docs.get_mut(d) {
            e.language_from = Some(file.to_path_buf());
        }
        d
    }

    /// The working tree side: the open document, or the file opened
    /// without a tab (`true`: this view opened it).
    fn working_tree_doc(&mut self, file: &Path) -> Option<(DocId, bool)> {
        if let Some(d) = self.docs.find_by_path(file)
            && self.docs.get(d).is_some_and(|e| !e.loading)
        {
            return Some((d, false));
        }
        let doc = Document::open(file, self.default_eol(Some(file))).ok()?;
        Some((self.docs.insert(doc), true))
    }

    /// Drop documents a diff opened, unless something else shows them.
    fn release_hidden(&mut self, docs: &[DocId]) {
        for d in docs {
            let dirty = self.docs.doc(*d).is_some_and(|x| x.is_dirty());
            if self.layout.views_of(*d).is_empty() && !dirty && !self.diff_uses(*d) {
                self.docs.remove(*d);
            }
        }
    }

    /// Whether a diff tab shows `d` on one of its sides.
    pub(crate) fn diff_uses(&self, d: DocId) -> bool {
        self.docs.ids().into_iter().any(|t| self.diff_of(t).is_some_and(|v| v.left == d || v.right == d))
    }

    /// A diff tab closed: release its sides unless something else shows
    /// them (a tab, another diff).
    pub(crate) fn diff_closed(&mut self, view: DiffView) {
        for d in [view.left, view.right] {
            self.release_doc_if_unused(d);
        }
    }

    /// Rebuild the rows of diffs whose documents changed (after a pause).
    pub(crate) fn diff_track(&mut self, now: Instant) {
        let stale: Vec<DocId> = self
            .docs
            .ids()
            .into_iter()
            .filter(|t| {
                self.diff_of(*t).is_some_and(|v| {
                    let ver = |d| self.docs.doc(d).map_or(0, |x| x.version());
                    (ver(v.left), ver(v.right)) != v.versions
                })
            })
            .collect();
        if stale.is_empty() {
            self.diff_deadline = None;
            return;
        }
        let due = *self.diff_deadline.get_or_insert(now + REBUILD_DELAY);
        if now < due {
            return;
        }
        self.diff_deadline = None;
        for t in stale {
            let Some(v) = self.diff_of(t) else { continue };
            let (Some(a), Some(b)) = (self.docs.doc(v.left), self.docs.doc(v.right)) else { continue };
            let (rows, changes) = build_rows(a.rope(), b.rope());
            let versions = (a.version(), b.version());
            if let Some(v) = self.diff_mut(t) {
                v.cursor = v.cursor.min(rows.len().saturating_sub(1));
                v.top = v.top.min(rows.len().saturating_sub(1));
                (v.rows, v.changes, v.versions) = (rows, changes, versions);
            }
            self.needs_render = true;
        }
    }

    /// Rows the diff in group `g` shows at once.
    fn diff_height(&self, g: GroupId) -> usize {
        let wl = crate::ui::workbench_layout(self);
        wl.groups.iter().find(|x| x.id == g).map_or(20, |x| x.content.height as usize).max(1)
    }

    /// Move the diff's cursor row by `delta`, keeping it on screen.
    fn diff_move(&mut self, d: DocId, g: GroupId, to: impl FnOnce(&DiffView, usize) -> usize) {
        let height = self.diff_height(g);
        let Some(v) = self.diff_mut(d) else { return };
        let last = v.rows.len().saturating_sub(1);
        v.cursor = to(v, height).min(last);
        if v.cursor < v.top {
            v.top = v.cursor;
        } else if v.cursor >= v.top + height {
            v.top = v.cursor + 1 - height;
        }
    }

    /// Scroll the diff in group `g` (the mouse wheel).
    pub(crate) fn diff_scroll(&mut self, d: DocId, g: GroupId, delta: isize) {
        let height = self.diff_height(g);
        if let Some(v) = self.diff_mut(d) {
            let max = v.rows.len().saturating_sub(height.min(v.rows.len()));
            v.top = (v.top as isize + delta).clamp(0, max as isize) as usize;
        }
    }

    /// Next or previous change from the cursor row (wrapping around).
    pub(crate) fn diff_goto_change(&mut self, forward: bool) {
        let Some(d) = self.active_diff() else { return };
        let g = self.layout.active;
        let Some(v) = self.diff_of(d) else { return };
        let (cur, ch) = (v.cursor, v.changes.clone());
        let target = if forward {
            ch.iter().copied().find(|c| *c > cur).or(ch.first().copied())
        } else {
            ch.iter().rev().copied().find(|c| *c < cur).or(ch.last().copied())
        };
        if let Some(t) = target {
            let height = self.diff_height(g);
            self.diff_move(d, g, |_, _| t);
            if let Some(v) = self.diff_mut(d) {
                v.top = t.saturating_sub(height / 3);
            }
        }
    }

    /// Open the compared file at the cursor row's line.
    pub(crate) fn diff_open_file(&mut self) {
        let Some(d) = self.active_diff() else { return };
        let Some(v) = self.diff_of(d) else { return };
        let file = v.file.clone();
        // The right side's line, or the nearest one above it.
        let line = v.rows[..=v.cursor.min(v.rows.len().saturating_sub(1))].iter().rev().find_map(|r| r.right);
        if !file.is_file() {
            self.notify(Severity::Info, format!("{} is not in the working tree", file.display()));
            return;
        }
        self.open_file(&file, false);
        self.set_cursor_line_col(line.unwrap_or(0), 0);
        self.center_active_cursor();
    }

    /// Keys in a diff tab: moving, scrolling, Enter. True when handled.
    pub(crate) fn diff_key(&mut self, key: KeyPress) -> bool {
        let Some(d) = self.active_diff() else { return false };
        let g = self.layout.active;
        if !(key.mods.is_empty() || (key.mods == Mods::CTRL && matches!(key.code, KeyCode::Home | KeyCode::End))) {
            return false;
        }
        match key.code {
            KeyCode::Up => self.diff_move(d, g, |v, _| v.cursor.saturating_sub(1)),
            KeyCode::Down => self.diff_move(d, g, |v, _| v.cursor + 1),
            KeyCode::PageUp => self.diff_move(d, g, |v, h| v.cursor.saturating_sub(h.saturating_sub(1))),
            KeyCode::PageDown => self.diff_move(d, g, |v, h| v.cursor + h.saturating_sub(1)),
            KeyCode::Home => self.diff_move(d, g, |_, _| 0),
            KeyCode::End => self.diff_move(d, g, |v, _| v.rows.len()),
            KeyCode::Left | KeyCode::Right => {
                let right = key.code == KeyCode::Right;
                if let Some(v) = self.diff_mut(d) {
                    v.scroll_col = if right { v.scroll_col + 4 } else { v.scroll_col.saturating_sub(4) };
                }
            }
            KeyCode::Enter => self.diff_open_file(),
            _ => return false,
        }
        true
    }

    /// A click in a diff tab: select the row (`count` 2 opens the file).
    pub(crate) fn diff_click(&mut self, d: DocId, g: GroupId, row: usize, count: u8) {
        self.layout.active = g;
        self.focus_editor();
        self.diff_move(d, g, |v, _| v.top + row);
        if count == 2 {
            self.diff_open_file();
        }
    }
}
