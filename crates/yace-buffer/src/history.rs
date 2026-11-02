//! Undo/redo history made of revisions. A revision groups one or more change
//! sets (typing coalesces into a single revision) together with the selection
//! before and after, so undo restores cursors as VS Code does.

use std::time::{Duration, Instant};

use crate::{changes::ChangeSet, selection::Selection};

/// What kind of edit produced a revision; drives coalescing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EditKind {
    /// Typing ordinary characters.
    Insert,
    /// Backspace/Delete of single graphemes.
    Delete,
    /// Anything else (paste, line moves, indent, …) — never coalesced.
    Other,
}

#[derive(Clone, Debug)]
pub(crate) struct Revision {
    pub id: u64,
    pub forward: Vec<ChangeSet>,
    pub inverse: Vec<ChangeSet>,
    pub sel_before: Selection,
    pub sel_after: Selection,
    pub kind: EditKind,
    pub at: Instant,
    pub sealed: bool,
}

#[derive(Debug)]
pub struct History {
    undo: Vec<Revision>,
    redo: Vec<Revision>,
    next_id: u64,
    limit: usize,
    coalesce_window: Duration,
}

impl Default for History {
    fn default() -> Self {
        Self {
            undo: Vec::new(),
            redo: Vec::new(),
            next_id: 1,
            limit: 10_000,
            coalesce_window: Duration::from_millis(1000),
        }
    }
}

impl History {
    /// Id identifying the current document state (0 = initial).
    pub fn current_id(&self) -> u64 {
        self.undo.last().map_or(0, |r| r.id)
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }
    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    /// Stop the current revision from absorbing further edits (cursor moved,
    /// file saved, focus changed…).
    pub fn seal(&mut self) {
        if let Some(r) = self.undo.last_mut() {
            r.sealed = true;
        }
    }

    pub(crate) fn record(
        &mut self,
        forward: ChangeSet,
        inverse: ChangeSet,
        sel_before: Selection,
        sel_after: Selection,
        kind: EditKind,
        breaks_word: bool,
    ) {
        self.redo.clear();
        let now = Instant::now();
        let id = self.next_id;
        self.next_id += 1;
        if let Some(last) = self.undo.last_mut()
            && !last.sealed
            && kind != EditKind::Other
            && last.kind == kind
            && !breaks_word
            && now.duration_since(last.at) < self.coalesce_window
        {
            last.forward.push(forward);
            last.inverse.push(inverse);
            last.sel_after = sel_after;
            last.at = now;
            // The state changed, so it is a new revision id (dirty tracking).
            last.id = id;
            return;
        }
        self.undo.push(Revision {
            id,
            forward: vec![forward],
            inverse: vec![inverse],
            sel_before,
            sel_after,
            kind,
            at: now,
            sealed: kind == EditKind::Other,
        });
        if self.undo.len() > self.limit {
            self.undo.remove(0);
        }
    }

    pub(crate) fn pop_undo(&mut self) -> Option<Revision> {
        let r = self.undo.pop()?;
        Some(r)
    }
    pub(crate) fn push_redo(&mut self, mut r: Revision) {
        r.sealed = true;
        self.redo.push(r);
    }
    pub(crate) fn pop_redo(&mut self) -> Option<Revision> {
        self.redo.pop()
    }
    pub(crate) fn push_undo(&mut self, mut r: Revision) {
        r.sealed = true;
        self.undo.push(r);
    }
}
