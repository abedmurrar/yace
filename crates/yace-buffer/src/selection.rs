//! Multi-range selections. Every range is `(anchor, head)` in char indices;
//! the head is where the cursor is drawn.

use smallvec::{SmallVec, smallvec};

use crate::changes::{Assoc, ChangeSet};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Range {
    pub anchor: usize,
    pub head: usize,
    /// Desired visual column for vertical movement ("sticky column").
    pub goal: Option<usize>,
}

impl Range {
    pub const fn new(anchor: usize, head: usize) -> Self {
        Self { anchor, head, goal: None }
    }
    pub const fn point(pos: usize) -> Self {
        Self::new(pos, pos)
    }
    pub fn from(&self) -> usize {
        self.anchor.min(self.head)
    }
    pub fn to(&self) -> usize {
        self.anchor.max(self.head)
    }
    pub fn is_empty(&self) -> bool {
        self.anchor == self.head
    }
    pub fn len(&self) -> usize {
        self.to() - self.from()
    }
    pub fn overlaps(&self, other: &Range) -> bool {
        // Empty ranges touching each other at the same point overlap; non-empty
        // ranges that merely touch do not.
        self.from() == other.from() || (self.from() < other.to() && other.from() < self.to())
    }
    /// Map both ends through a change set.
    pub fn map(&self, cs: &ChangeSet) -> Range {
        if self.is_empty() {
            let p = cs.map_pos(self.head, Assoc::After);
            return Range::point(p);
        }
        let (fa, fh) =
            if self.anchor < self.head { (Assoc::After, Assoc::Before) } else { (Assoc::Before, Assoc::After) };
        Range::new(cs.map_pos(self.anchor, fa), cs.map_pos(self.head, fh))
    }
    pub fn with_head(self, head: usize, extend: bool) -> Range {
        if extend { Range { anchor: self.anchor, head, goal: None } } else { Range::point(head) }
    }
    pub fn clamp(self, len: usize) -> Range {
        Range { anchor: self.anchor.min(len), head: self.head.min(len), goal: self.goal }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Selection {
    ranges: SmallVec<[Range; 1]>,
    primary: usize,
}

impl Default for Selection {
    fn default() -> Self {
        Self::point(0)
    }
}

impl Selection {
    pub fn point(pos: usize) -> Self {
        Self { ranges: smallvec![Range::point(pos)], primary: 0 }
    }
    pub fn single(range: Range) -> Self {
        Self { ranges: smallvec![range], primary: 0 }
    }
    /// Build from ranges; `primary` indexes into `ranges`. Normalizes.
    pub fn new(ranges: impl IntoIterator<Item = Range>, primary: usize) -> Self {
        let ranges: SmallVec<[Range; 1]> = ranges.into_iter().collect();
        assert!(!ranges.is_empty(), "selection needs at least one range");
        let primary = primary.min(ranges.len() - 1);
        let mut s = Self { ranges, primary };
        s.normalize();
        s
    }
    pub fn ranges(&self) -> &[Range] {
        &self.ranges
    }
    pub fn len(&self) -> usize {
        self.ranges.len()
    }
    pub fn is_empty(&self) -> bool {
        false
    }
    pub fn primary_index(&self) -> usize {
        self.primary
    }
    pub fn primary(&self) -> Range {
        self.ranges[self.primary]
    }
    pub fn is_multi(&self) -> bool {
        self.ranges.len() > 1
    }
    pub fn has_non_empty(&self) -> bool {
        self.ranges.iter().any(|r| !r.is_empty())
    }
    /// Keep only the primary range.
    pub fn into_primary(self) -> Self {
        Self::single(self.primary())
    }
    /// Add a range and make it primary.
    pub fn push(mut self, range: Range) -> Self {
        self.ranges.push(range);
        self.primary = self.ranges.len() - 1;
        self.normalize();
        self
    }
    /// Apply `f` to every range, then normalize.
    pub fn transform(&self, mut f: impl FnMut(Range) -> Range) -> Self {
        let ranges = self.ranges.iter().map(|r| f(*r)).collect();
        let mut s = Self { ranges, primary: self.primary };
        s.normalize();
        s
    }
    pub fn map(&self, cs: &ChangeSet) -> Self {
        self.transform(|r| Range { goal: None, ..r.map(cs) })
    }
    pub fn clamp(&self, len: usize) -> Self {
        self.transform(|r| r.clamp(len))
    }

    /// Sort by start and merge overlapping ranges, keeping the primary stable.
    fn normalize(&mut self) {
        if self.ranges.len() == 1 {
            return;
        }
        let primary = self.ranges[self.primary];
        self.ranges.sort_by_key(|r| (r.from(), r.to()));
        let mut out: SmallVec<[Range; 1]> = SmallVec::with_capacity(self.ranges.len());
        let mut primary_idx = 0;
        for r in self.ranges.drain(..) {
            if let Some(last) = out.last_mut()
                && last.overlaps(&r)
            {
                let from = last.from().min(r.from());
                let to = last.to().max(r.to());
                let reversed = last.head < last.anchor;
                *last = if reversed { Range::new(to, from) } else { Range::new(from, to) };
                if r == primary {
                    primary_idx = out.len() - 1;
                }
                continue;
            }
            if r == primary {
                primary_idx = out.len();
            }
            out.push(r);
        }
        self.ranges = out;
        self.primary = primary_idx.min(self.ranges.len() - 1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn merges_overlapping_and_keeps_primary() {
        let s = Selection::new([Range::new(5, 8), Range::new(0, 2), Range::new(7, 10)], 0);
        assert_eq!(s.ranges(), &[Range::new(0, 2), Range::new(5, 10)]);
        assert_eq!(s.primary(), Range::new(5, 10));
    }

    #[test]
    fn duplicate_cursors_merge_touching_selections_do_not() {
        let s = Selection::new([Range::point(3), Range::point(3)], 1);
        assert_eq!(s.len(), 1);
        let s = Selection::new([Range::new(0, 3), Range::new(3, 5)], 0);
        assert_eq!(s.len(), 2);
    }
}
