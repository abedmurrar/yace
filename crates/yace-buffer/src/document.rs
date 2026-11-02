//! A document: rope + undo history + file metadata. Views (cursors, scroll)
//! live outside the document so the same document can be shown in several
//! editor groups; every edit returns the applied `ChangeSet` so other views
//! can map their selections through it.

use std::path::{Path, PathBuf};

use ropey::Rope;

use crate::{
    changes::ChangeSet,
    history::{EditKind, History},
    io::{self, DiskSnapshot, LoadError, SaveError, SaveOptions},
    line_ending::{self, LineEnding, LineEndingInfo},
    selection::Selection,
};

/// Disk-sync state of a document relative to its file.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum DiskState {
    #[default]
    InSync,
    /// The file changed on disk while the buffer had unsaved edits and the
    /// user chose to keep theirs; the next save must confirm.
    Conflicted,
    /// The file was deleted or moved away on disk.
    Deleted,
}

#[derive(Debug)]
pub struct Document {
    rope: Rope,
    history: History,
    path: Option<PathBuf>,
    pub line_ending: LineEndingInfo,
    pub has_bom: bool,
    pub disk: Option<DiskSnapshot>,
    pub disk_state: DiskState,
    /// Monotonic counter bumped on every content change (render caches).
    version: u64,
    saved_revision: u64,
    /// Opened in reduced-feature mode (very large file).
    pub large_file: bool,
}

/// The result of an edit, for syncing other views.
#[derive(Debug, Clone)]
pub struct Applied {
    pub changes: ChangeSet,
    pub selection: Selection,
}

impl Document {
    pub fn new_untitled(default_eol: LineEnding) -> Self {
        Self::from_rope(Rope::new(), None, default_eol)
    }

    pub fn from_text(text: &str) -> Self {
        Self::from_rope(Rope::from_str(text), None, LineEnding::Lf)
    }

    fn from_rope(rope: Rope, path: Option<PathBuf>, default_eol: LineEnding) -> Self {
        let line_ending = line_ending::detect(&rope, default_eol);
        Self {
            rope,
            history: History::default(),
            path,
            line_ending,
            has_bom: false,
            disk: None,
            disk_state: DiskState::InSync,
            version: 0,
            saved_revision: 0,
            large_file: false,
        }
    }

    pub fn open(path: &Path, default_eol: LineEnding) -> Result<Self, LoadError> {
        let loaded = io::load(path)?;
        let mut doc = Self::from_rope(loaded.rope, Some(path.to_path_buf()), default_eol);
        doc.has_bom = loaded.has_bom;
        doc.disk = Some(loaded.snapshot);
        Ok(doc)
    }

    pub fn rope(&self) -> &Rope {
        &self.rope
    }
    pub fn text(&self) -> String {
        self.rope.to_string()
    }
    pub fn len_chars(&self) -> usize {
        self.rope.len_chars()
    }
    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }
    pub fn set_path(&mut self, path: Option<PathBuf>) {
        self.path = path;
    }
    pub fn version(&self) -> u64 {
        self.version
    }
    pub fn history(&self) -> &History {
        &self.history
    }
    pub fn is_dirty(&self) -> bool {
        self.history.current_id() != self.saved_revision
    }
    /// The line ending used for newly inserted line breaks.
    pub fn eol(&self) -> LineEnding {
        self.line_ending.dominant
    }

    /// Prevent the next edit from coalescing with the previous one.
    pub fn seal_history(&mut self) {
        self.history.seal();
    }

    /// Apply a change set produced against the current text.
    pub fn apply(
        &mut self,
        changes: ChangeSet,
        sel_before: &Selection,
        sel_after: Selection,
        kind: EditKind,
    ) -> Applied {
        if changes.is_empty() {
            return Applied { changes, selection: sel_after };
        }
        // A new word after whitespace starts a new undo group, so undo works
        // word by word while typing (whitespace sticks to the previous word).
        let breaks_word = kind == EditKind::Insert
            && changes.changes().first().is_some_and(|c| {
                c.insert.chars().next().is_some_and(|ch| !ch.is_whitespace())
                    && c.from > 0
                    && self.rope.char(c.from - 1).is_whitespace()
            });
        let inverse = changes.invert(&self.rope);
        changes.apply(&mut self.rope);
        self.version += 1;
        let sel_after = sel_after.clamp(self.rope.len_chars());
        self.history.record(changes.clone(), inverse, sel_before.clone(), sel_after.clone(), kind, breaks_word);
        Applied { changes, selection: sel_after }
    }

    /// Undo one revision. Returns the composite changes (in order) and the
    /// selection to restore.
    pub fn undo(&mut self) -> Option<(Vec<ChangeSet>, Selection)> {
        let rev = self.history.pop_undo()?;
        let mut applied = Vec::with_capacity(rev.inverse.len());
        for inv in rev.inverse.iter().rev() {
            inv.apply(&mut self.rope);
            applied.push(inv.clone());
        }
        self.version += 1;
        let sel = rev.sel_before.clamp(self.rope.len_chars());
        self.history.push_redo(rev);
        Some((applied, sel))
    }

    pub fn redo(&mut self) -> Option<(Vec<ChangeSet>, Selection)> {
        let rev = self.history.pop_redo()?;
        let mut applied = Vec::with_capacity(rev.forward.len());
        for fwd in &rev.forward {
            fwd.apply(&mut self.rope);
            applied.push(fwd.clone());
        }
        self.version += 1;
        let sel = rev.sel_after.clamp(self.rope.len_chars());
        self.history.push_undo(rev);
        Some((applied, sel))
    }

    /// Save to the document's path (or `path` for Save As).
    pub fn save(&mut self, path: Option<&Path>, force: bool) -> Result<(), SaveError> {
        let target = path.map(Path::to_path_buf).or_else(|| self.path.clone()).ok_or_else(|| SaveError::Io {
            path: PathBuf::from("Untitled"),
            source: std::io::Error::new(std::io::ErrorKind::InvalidInput, "document has no path"),
        })?;
        let same_file = self.path.as_deref() == Some(target.as_path());
        let expected = if same_file && self.disk_state != DiskState::Deleted { self.disk.as_ref() } else { None };
        let force = force || (same_file && self.disk_state == DiskState::Deleted);
        let snapshot = io::save(&target, &self.rope, SaveOptions { expected, force, bom: self.has_bom })?;
        self.disk = Some(snapshot);
        self.disk_state = DiskState::InSync;
        self.path = Some(target);
        self.mark_saved();
        Ok(())
    }

    pub fn mark_saved(&mut self) {
        self.saved_revision = self.history.current_id();
        self.history.seal();
    }

    /// Replace the content with what is on disk (Revert / external reload).
    /// This is recorded as an undoable edit so a reload never loses work
    /// irrecoverably; the document is clean afterwards.
    pub fn reload(&mut self, sel: &Selection) -> Result<Applied, LoadError> {
        let path = self.path.clone().ok_or_else(|| LoadError::Io {
            path: PathBuf::from("Untitled"),
            source: std::io::Error::new(std::io::ErrorKind::InvalidInput, "document has no path"),
        })?;
        let loaded = io::load(&path)?;
        let new_text: String = loaded.rope.to_string();
        let cs = ChangeSet::new(
            vec![crate::changes::Change::replace(0, self.rope.len_chars(), new_text)],
            self.rope.len_chars(),
        );
        // Keep cursors where they were, clamped.
        let after = sel.clamp(loaded.rope.len_chars());
        let applied = self.apply(cs, sel, after, EditKind::Other);
        self.has_bom = loaded.has_bom;
        self.disk = Some(loaded.snapshot);
        self.disk_state = DiskState::InSync;
        self.line_ending = line_ending::detect(&self.rope, self.line_ending.dominant);
        self.mark_saved();
        Ok(Applied { changes: applied.changes, selection: sel.clamp(self.rope.len_chars()) })
    }

    /// Recompute line-ending info (after edits that may have changed it).
    pub fn refresh_line_ending(&mut self) {
        self.line_ending = line_ending::detect(&self.rope, self.line_ending.dominant);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{changes::Change, selection::Range};

    fn insert(doc: &mut Document, at: usize, s: &str, kind: EditKind) {
        let before = Selection::point(at);
        let cs = ChangeSet::new(vec![Change::insert(at, s)], doc.len_chars());
        let after = Selection::point(at + s.chars().count());
        doc.apply(cs, &before, after, kind);
    }

    #[test]
    fn typing_coalesces_and_undo_restores_selection() {
        let mut doc = Document::from_text("");
        for (i, c) in "abc".chars().enumerate() {
            insert(&mut doc, i, &c.to_string(), EditKind::Insert);
        }
        assert_eq!(doc.text(), "abc");
        let (_, sel) = doc.undo().unwrap();
        assert_eq!(doc.text(), "");
        assert_eq!(sel, Selection::point(0));
        assert!(doc.undo().is_none());
        let (_, sel) = doc.redo().unwrap();
        assert_eq!(doc.text(), "abc");
        assert_eq!(sel, Selection::point(3));
    }

    #[test]
    fn whitespace_breaks_coalescing() {
        let mut doc = Document::from_text("");
        insert(&mut doc, 0, "a", EditKind::Insert);
        insert(&mut doc, 1, " ", EditKind::Insert);
        insert(&mut doc, 2, "b", EditKind::Insert);
        doc.undo();
        assert_eq!(doc.text(), "a ");
    }

    #[test]
    fn dirty_tracks_save_point_through_undo_redo() {
        let mut doc = Document::from_text("x");
        assert!(!doc.is_dirty());
        insert(&mut doc, 1, "y", EditKind::Other);
        assert!(doc.is_dirty());
        doc.mark_saved();
        assert!(!doc.is_dirty());
        doc.undo();
        assert!(doc.is_dirty());
        doc.redo();
        assert!(!doc.is_dirty());
        // Coalescing after save must make it dirty again.
        insert(&mut doc, 2, "z", EditKind::Insert);
        assert!(doc.is_dirty());
    }

    #[test]
    fn multi_cursor_edit_is_one_undo_step() {
        let mut doc = Document::from_text("a\nb\nc");
        let before = Selection::new([Range::point(0), Range::point(2), Range::point(4)], 0);
        let cs =
            ChangeSet::new(before.ranges().iter().map(|r| Change::insert(r.head, "- ")).collect(), doc.len_chars());
        let after = before.map(&cs);
        doc.apply(cs, &before, after, EditKind::Other);
        assert_eq!(doc.text(), "- a\n- b\n- c");
        doc.undo();
        assert_eq!(doc.text(), "a\nb\nc");
    }

    #[test]
    fn save_and_reload() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("f.txt");
        std::fs::write(&p, "hello\n").unwrap();
        let mut doc = Document::open(&p, LineEnding::Lf).unwrap();
        insert(&mut doc, 5, "!", EditKind::Insert);
        doc.save(None, false).unwrap();
        assert!(!doc.is_dirty());
        assert_eq!(std::fs::read_to_string(&p).unwrap(), "hello!\n");
        std::fs::write(&p, "changed\n").unwrap();
        doc.reload(&Selection::point(3)).unwrap();
        assert_eq!(doc.text(), "changed\n");
        assert!(!doc.is_dirty());
        // The reload itself can be undone (no irrecoverable loss).
        doc.undo();
        assert_eq!(doc.text(), "hello!\n");
    }
}
