//! Change sets: a batch of non-overlapping replacements expressed in the
//! coordinates of the document *before* the change. Applying, inverting and
//! mapping positions through a change set are the primitives behind editing,
//! undo and keeping other views' cursors in sync.

use ropey::Rope;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Change {
    /// Start char index (pre-change coordinates).
    pub from: usize,
    /// End char index, exclusive (pre-change coordinates).
    pub to: usize,
    /// Replacement text.
    pub insert: String,
}

impl Change {
    pub fn insert(at: usize, text: impl Into<String>) -> Self {
        Self { from: at, to: at, insert: text.into() }
    }
    pub fn delete(from: usize, to: usize) -> Self {
        Self { from, to, insert: String::new() }
    }
    pub fn replace(from: usize, to: usize, text: impl Into<String>) -> Self {
        Self { from, to, insert: text.into() }
    }
    fn insert_chars(&self) -> usize {
        self.insert.chars().count()
    }
}

/// How a position at the exact boundary of a change maps.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Assoc {
    /// Stay before inserted text.
    Before,
    /// Move after inserted text.
    After,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ChangeSet {
    changes: Vec<Change>,
    len_before: usize,
}

impl ChangeSet {
    /// Build a change set. Changes are sorted; overlapping changes are merged
    /// conservatively (the later one wins inside the overlap) so callers
    /// that compute edits per cursor can never corrupt the rope.
    pub fn new(mut changes: Vec<Change>, len_before: usize) -> Self {
        changes.retain(|c| !(c.from == c.to && c.insert.is_empty()));
        for c in &mut changes {
            c.to = c.to.min(len_before);
            c.from = c.from.min(c.to);
        }
        changes.sort_by_key(|c| (c.from, c.to));
        let mut out: Vec<Change> = Vec::with_capacity(changes.len());
        for c in changes {
            if let Some(last) = out.last_mut()
                && c.from < last.to
            {
                last.to = last.to.max(c.to);
                last.insert.push_str(&c.insert);
                continue;
            }
            out.push(c);
        }
        Self { changes: out, len_before }
    }

    pub fn empty(len: usize) -> Self {
        Self { changes: Vec::new(), len_before: len }
    }
    pub fn is_empty(&self) -> bool {
        self.changes.is_empty()
    }
    pub fn changes(&self) -> &[Change] {
        &self.changes
    }
    pub fn len_before(&self) -> usize {
        self.len_before
    }
    pub fn len_after(&self) -> usize {
        let mut len = self.len_before as isize;
        for c in &self.changes {
            len += c.insert_chars() as isize - (c.to - c.from) as isize;
        }
        len as usize
    }

    /// Apply to a rope (in reverse so earlier offsets stay valid).
    pub fn apply(&self, rope: &mut Rope) {
        debug_assert_eq!(rope.len_chars(), self.len_before, "change set applied to wrong document");
        for c in self.changes.iter().rev() {
            if c.to > c.from {
                rope.remove(c.from..c.to);
            }
            if !c.insert.is_empty() {
                rope.insert(c.from, &c.insert);
            }
        }
    }

    /// The change set that undoes this one; `original` is the rope before
    /// `self` was applied.
    pub fn invert(&self, original: &Rope) -> ChangeSet {
        let mut delta: isize = 0;
        let mut inv = Vec::with_capacity(self.changes.len());
        for c in &self.changes {
            let ins = c.insert_chars();
            let from = (c.from as isize + delta) as usize;
            let removed: String = original.slice(c.from..c.to).into();
            inv.push(Change { from, to: from + ins, insert: removed });
            delta += ins as isize - (c.to - c.from) as isize;
        }
        ChangeSet { changes: inv, len_before: self.len_after() }
    }

    /// Map a pre-change position to its post-change position.
    pub fn map_pos(&self, pos: usize, assoc: Assoc) -> usize {
        let mut delta: isize = 0;
        for c in &self.changes {
            let ins = c.insert_chars() as isize;
            if pos < c.from || (pos == c.from && c.from == c.to && assoc == Assoc::Before) {
                break;
            }
            if pos <= c.to {
                // Inside (or at the end of) the replaced range.
                let base = c.from as isize + delta;
                let mapped = if pos == c.from && c.to > c.from && assoc == Assoc::Before {
                    base
                } else if assoc == Assoc::After || pos == c.to {
                    base + ins
                } else {
                    base
                };
                return mapped as usize;
            }
            delta += ins - (c.to - c.from) as isize;
        }
        (pos as isize + delta) as usize
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn apply_and_invert_roundtrip() {
        let mut rope = Rope::from_str("hello world");
        let original = rope.clone();
        let cs = ChangeSet::new(
            vec![Change::replace(0, 5, "HEY"), Change::insert(6, ">> "), Change::delete(10, 11)],
            rope.len_chars(),
        );
        cs.apply(&mut rope);
        assert_eq!(rope.to_string(), "HEY >> worl");
        let inv = cs.invert(&original);
        inv.apply(&mut rope);
        assert_eq!(rope, original);
    }

    #[test]
    fn map_positions() {
        let cs = ChangeSet::new(vec![Change::insert(2, "xx"), Change::delete(4, 6)], 10);
        assert_eq!(cs.map_pos(0, Assoc::After), 0);
        assert_eq!(cs.map_pos(2, Assoc::Before), 2);
        assert_eq!(cs.map_pos(2, Assoc::After), 4);
        assert_eq!(cs.map_pos(3, Assoc::After), 5);
        assert_eq!(cs.map_pos(5, Assoc::After), 6, "inside deletion collapses to its start");
        assert_eq!(cs.map_pos(8, Assoc::After), 8);
        assert_eq!(cs.len_after(), 10);
    }

    #[test]
    fn overlapping_changes_are_merged_not_corrupting() {
        let mut rope = Rope::from_str("abcdef");
        let cs = ChangeSet::new(vec![Change::delete(1, 4), Change::delete(2, 5)], 6);
        cs.apply(&mut rope);
        assert_eq!(rope.to_string(), "af");
    }

    #[test]
    fn unicode_insert_lengths() {
        let mut rope = Rope::from_str("ab");
        let cs = ChangeSet::new(vec![Change::insert(1, "👍中")], 2);
        cs.apply(&mut rope);
        assert_eq!(rope.to_string(), "a👍中b");
        assert_eq!(cs.map_pos(1, Assoc::After), 3);
        assert_eq!(cs.len_after(), 4);
    }
}
