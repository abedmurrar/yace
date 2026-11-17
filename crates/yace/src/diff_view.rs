//! The side-by-side diff: two documents shown in aligned rows (spec §15.2).
//! Equal lines pair up; a changed region pairs its lines and pads the
//! shorter side with filler rows, so both sides scroll together.

use std::path::PathBuf;

use yace_buffer::Rope;
use yace_git::diff::line_hunks;

use crate::editor::DocId;

/// What a row shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RowKind {
    Equal,
    /// A line only on the right (filler on the left).
    Added,
    /// A line only on the left (filler on the right).
    Removed,
    /// A changed line on both sides.
    Modified,
}

/// One screen row of the diff: a line of each side (0-based), or none for
/// filler.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DiffRow {
    pub left: Option<usize>,
    pub right: Option<usize>,
    pub kind: RowKind,
}

/// A diff tab's state (kept on its document entry).
#[derive(Debug, Clone)]
pub struct DiffView {
    /// The file it compares (Enter opens it).
    pub file: PathBuf,
    /// Short names of the sides, e.g. `Index` and `Working Tree`.
    pub left_label: String,
    pub right_label: String,
    /// The two sides; the left is always a hidden read-only version, the
    /// right too unless it is the file's own document.
    pub left: DocId,
    pub right: DocId,
    pub rows: Vec<DiffRow>,
    /// The first row of each change.
    pub changes: Vec<usize>,
    /// Document versions the rows were computed for.
    pub versions: (u64, u64),
    /// The row the cursor is on; the first row shown; the first column.
    pub cursor: usize,
    pub top: usize,
    pub scroll_col: usize,
}

/// Lines of a text as the diff counts them (a final line break doesn't
/// start another line).
fn line_count(rope: &Rope) -> usize {
    if rope.len_chars() == 0 {
        return 0;
    }
    let n = rope.len_lines();
    if rope.line(n - 1).len_chars() == 0 { n - 1 } else { n }
}

/// Aligned rows turning `left` into `right`, and where each change starts.
pub fn build_rows(left: &Rope, right: &Rope) -> (Vec<DiffRow>, Vec<usize>) {
    let hunks = line_hunks(&left.to_string(), &right.to_string());
    let (nl, nr) = (line_count(left), line_count(right));
    let mut rows = Vec::with_capacity(nl.max(nr));
    let mut changes = Vec::new();
    let (mut i, mut j) = (0usize, 0usize);
    for h in hunks {
        let (old, new) = (h.old.start as usize..h.old.end as usize, h.lines.start as usize..h.lines.end as usize);
        while i < old.start && j < new.start {
            rows.push(DiffRow { left: Some(i), right: Some(j), kind: RowKind::Equal });
            i += 1;
            j += 1;
        }
        changes.push(rows.len());
        for k in 0..old.len().max(new.len()) {
            let (l, r) = ((k < old.len()).then_some(old.start + k), (k < new.len()).then_some(new.start + k));
            let kind = match (l, r) {
                (Some(_), Some(_)) => RowKind::Modified,
                (Some(_), None) => RowKind::Removed,
                _ => RowKind::Added,
            };
            rows.push(DiffRow { left: l, right: r, kind });
        }
        (i, j) = (old.end, new.end);
    }
    while i < nl || j < nr {
        let (l, r) = ((i < nl).then_some(i), (j < nr).then_some(j));
        rows.push(DiffRow { left: l, right: r, kind: RowKind::Equal });
        i += 1;
        j += 1;
    }
    (rows, changes)
}

/// The changed middle of two versions of a line (char ranges), after the
/// common start and end: what a modified row highlights more strongly.
pub fn changed_span(a: &str, b: &str) -> ((usize, usize), (usize, usize)) {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let pre = a.iter().zip(&b).take_while(|(x, y)| x == y).count();
    let max_suf = a.len().min(b.len()) - pre;
    let suf = a.iter().rev().zip(b.iter().rev()).take(max_suf).take_while(|(x, y)| x == y).count();
    ((pre, a.len() - suf), (pre, b.len() - suf))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(l: &str, r: &str) -> Vec<(Option<usize>, Option<usize>, RowKind)> {
        build_rows(&Rope::from_str(l), &Rope::from_str(r)).0.iter().map(|x| (x.left, x.right, x.kind)).collect()
    }

    #[test]
    fn rows_align_adds_removes_and_changes() {
        use RowKind::*;
        assert_eq!(kinds("a\nb\n", "a\nb\n"), [(Some(0), Some(0), Equal), (Some(1), Some(1), Equal)]);
        // Added line: filler on the left.
        assert_eq!(
            kinds("a\nc\n", "a\nb\nc\n"),
            [(Some(0), Some(0), Equal), (None, Some(1), Added), (Some(1), Some(2), Equal)]
        );
        // Removed line: filler on the right.
        assert_eq!(
            kinds("a\nb\nc\n", "a\nc\n"),
            [(Some(0), Some(0), Equal), (Some(1), None, Removed), (Some(2), Some(1), Equal)]
        );
        // One line became two: a modified pair, then an added line.
        assert_eq!(
            kinds("a\nb\nz\n", "a\nB1\nB2\nz\n"),
            [
                (Some(0), Some(0), Equal),
                (Some(1), Some(1), Modified),
                (None, Some(2), Added),
                (Some(2), Some(3), Equal)
            ]
        );
        // A new file: everything added.
        assert_eq!(kinds("", "x\ny\n"), [(None, Some(0), Added), (None, Some(1), Added)]);
        // Changes start where they start.
        let (_, changes) = build_rows(&Rope::from_str("a\nb\nc\nd\n"), &Rope::from_str("A\nb\nc\nD\n"));
        assert_eq!(changes, [0, 3]);
        // No final line break.
        assert_eq!(kinds("a", "a\nb"), [(Some(0), Some(0), Equal), (None, Some(1), Added)]);
    }

    #[test]
    fn changed_span_skips_the_common_start_and_end() {
        assert_eq!(changed_span("let x = 1;", "let x = 22;"), ((8, 9), (8, 10)));
        assert_eq!(changed_span("abc", "abc"), ((3, 3), (3, 3)));
        assert_eq!(changed_span("aXa", "aa"), ((1, 2), (1, 1)));
    }
}
