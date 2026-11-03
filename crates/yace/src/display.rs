//! Display map: buffer lines → screen rows, accounting for folds (hidden
//! line ranges) and soft wrap. Used by rendering, scrolling, vertical
//! movement and mouse hit-testing so they always agree.
//!
//! Everything is computed on demand for the rows actually asked for (the
//! viewport, or the distance from the top of the view to the cursor); no
//! per-document row index is maintained, so huge files cost nothing extra.
//! With no folds and no wrap every query reduces to plain line arithmetic.

use std::cell::RefCell;

use unicode_segmentation::UnicodeSegmentation;
use yace_buffer::{
    ChangeSet, Rope,
    text::{char_at_visual_col, line_without_ending},
};

/// A collapsed region: chars `start..end` are hidden, where `start` is the
/// end of the header line and `end` the end of the last hidden line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Fold {
    pub start: usize,
    pub end: usize,
}

impl Fold {
    pub fn from_lines(rope: &Rope, header: usize, last: usize) -> Option<Fold> {
        let n = rope.len_lines();
        if header >= last || last >= n {
            return None;
        }
        let end_of = |l: usize| rope.line_to_char(l) + line_without_ending(rope, l).len_chars();
        Some(Fold { start: end_of(header), end: end_of(last) })
    }

    pub fn lines(&self, rope: &Rope) -> (usize, usize) {
        let len = rope.len_chars();
        (rope.char_to_line(self.start.min(len)), rope.char_to_line(self.end.min(len)))
    }
}

/// Map folds through an edit. A fold whose hidden text (or the line break
/// that starts it) is touched is dropped: the edit reveals it.
pub fn map_folds(folds: &mut Vec<Fold>, cs: &ChangeSet) {
    folds.retain(|f| !cs.changes().iter().any(|c| c.to > f.start && c.from <= f.end));
    for f in folds.iter_mut() {
        f.start = cs.map_pos(f.start, yace_buffer::Assoc::After);
        f.end = cs.map_pos(f.end, yace_buffer::Assoc::Before);
    }
}

/// A screen row: buffer line plus wrapped segment index.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default)]
pub struct Row {
    pub line: usize,
    pub sub: usize,
}

/// One wrapped segment of a line: chars `start..end` (relative to the line
/// start) shown after `indent` blank cells.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Segment {
    pub start: usize,
    pub end: usize,
    pub indent: usize,
}

pub struct DisplayMap<'a> {
    rope: &'a Rope,
    /// Merged hidden line ranges `[first, last]`, sorted.
    hidden: Vec<(usize, usize)>,
    /// Header lines of folds, sorted.
    headers: Vec<usize>,
    /// Wrap width in cells.
    wrap: Option<usize>,
    tab: usize,
    /// Segments computed during this frame (lines can be long).
    seg_cache: RefCell<Vec<(usize, Vec<Segment>)>>,
}

fn grapheme_cells(g: &str, col: usize, tab: usize) -> usize {
    if g == "\t" {
        tab - col % tab
    } else if g.chars().next().is_some_and(char::is_control) {
        1
    } else {
        unicode_width::UnicodeWidthStr::width(g).max(if g.is_empty() { 0 } else { 1 })
    }
}

/// Split a line into wrapped segments (word boundaries; continuation rows
/// keep the line's indentation, as VS Code's `wrappingIndent: same`).
pub fn wrap_line(text: &str, width: usize, tab: usize) -> Vec<Segment> {
    let width = width.max(4);
    let mut segs = Vec::new();
    // Leading indentation in cells (continuation rows reuse it).
    let mut indent = 0;
    for c in text.chars() {
        match c {
            ' ' => indent += 1,
            '\t' => indent += tab - indent % tab,
            _ => break,
        }
    }
    if indent > width / 2 {
        indent = 0;
    }
    let mut seg_start = 0usize; // char index
    let mut seg_indent = 0usize;
    let mut col = 0usize; // cells in the current row (including indent)
    let mut abs_col = 0usize; // cells from the line start (tab stops)
    let mut ci = 0usize;
    let mut last_break: Option<(usize, usize)> = None; // (char index after whitespace, col there)
    let mut prev_ws = false;
    for g in text.graphemes(true) {
        let w = grapheme_cells(g, abs_col, tab);
        let ws = g.chars().all(char::is_whitespace);
        if !ws && prev_ws && ci > seg_start {
            last_break = Some((ci, col));
        }
        if col + w > width && ci > seg_start && !ws {
            // Break before this grapheme: at the last word boundary if any.
            let (brk, brk_col) = last_break.filter(|(b, _)| *b > seg_start).unwrap_or((ci, col));
            segs.push(Segment { start: seg_start, end: brk, indent: seg_indent });
            seg_start = brk;
            seg_indent = indent;
            col = indent + (col - brk_col);
            last_break = None;
        }
        col += w;
        abs_col += w;
        ci += g.chars().count();
        prev_ws = ws;
    }
    segs.push(Segment { start: seg_start, end: ci, indent: seg_indent });
    segs
}

impl<'a> DisplayMap<'a> {
    pub fn new(rope: &'a Rope, folds: &[Fold], wrap: Option<usize>, tab: usize) -> Self {
        let mut ranges: Vec<(usize, usize)> =
            folds.iter().map(|f| f.lines(rope)).filter(|(h, l)| l > h).collect();
        ranges.sort_unstable();
        let mut headers = Vec::new();
        let mut hidden: Vec<(usize, usize)> = Vec::new();
        for (h, l) in ranges {
            // A fold whose header is itself hidden is not a visible header.
            if !hidden.iter().any(|(a, b)| h >= *a && h <= *b) {
                headers.push(h);
            }
            match hidden.last_mut() {
                Some((_, b)) if h <= *b => *b = (*b).max(l),
                _ => hidden.push((h + 1, l)),
            }
        }
        Self { rope, hidden, headers, wrap: wrap.filter(|w| *w > 0), tab: tab.max(1), seg_cache: RefCell::default() }
    }

    pub fn is_plain(&self) -> bool {
        self.hidden.is_empty() && self.wrap.is_none()
    }

    pub fn wraps(&self) -> bool {
        self.wrap.is_some()
    }

    fn last_line(&self) -> usize {
        self.rope.len_lines().saturating_sub(1)
    }

    fn hidden_range(&self, line: usize) -> Option<(usize, usize)> {
        let i = self.hidden.partition_point(|(_, b)| *b < line);
        self.hidden.get(i).copied().filter(|(a, _)| *a <= line)
    }

    pub fn is_hidden(&self, line: usize) -> bool {
        self.hidden_range(line).is_some()
    }

    pub fn is_folded_header(&self, line: usize) -> bool {
        self.headers.binary_search(&line).is_ok()
    }

    /// The visible line showing `line` (its fold header when hidden).
    pub fn visible_line(&self, line: usize) -> usize {
        match self.hidden_range(line) {
            Some((a, _)) => a.saturating_sub(1),
            None => line,
        }
    }

    pub fn next_line(&self, line: usize) -> Option<usize> {
        let mut l = line + 1;
        if let Some((_, b)) = self.hidden_range(l) {
            l = b + 1;
        }
        (l <= self.last_line()).then_some(l)
    }

    pub fn prev_line(&self, line: usize) -> Option<usize> {
        let l = line.checked_sub(1)?;
        Some(self.visible_line(l))
    }

    /// Wrapped segments of a line (one segment without wrap).
    pub fn segments(&self, line: usize) -> Vec<Segment> {
        let Some(width) = self.wrap else {
            let len = line_without_ending(self.rope, line).len_chars();
            return vec![Segment { start: 0, end: len, indent: 0 }];
        };
        if let Some((_, s)) = self.seg_cache.borrow().iter().find(|(l, _)| *l == line) {
            return s.clone();
        }
        let text: String = line_without_ending(self.rope, line).into();
        let segs = wrap_line(&text, width, self.tab);
        let mut c = self.seg_cache.borrow_mut();
        if c.len() > 256 {
            c.clear();
        }
        c.push((line, segs.clone()));
        segs
    }

    fn sub_count(&self, line: usize) -> usize {
        if self.wrap.is_none() { 1 } else { self.segments(line).len() }
    }

    /// Clamp a row to a visible, existing one.
    pub fn clamp(&self, row: Row) -> Row {
        let line = self.visible_line(row.line.min(self.last_line()));
        let sub = if line == row.line { row.sub.min(self.sub_count(line) - 1) } else { 0 };
        Row { line, sub }
    }

    /// Move `delta` rows, stopping at the first/last row.
    pub fn advance(&self, row: Row, delta: isize) -> Row {
        let mut r = self.clamp(row);
        if self.is_plain() {
            let line = (r.line as isize + delta).clamp(0, self.last_line() as isize) as usize;
            return Row { line, sub: 0 };
        }
        if delta > 0 {
            for _ in 0..delta {
                if r.sub + 1 < self.sub_count(r.line) {
                    r.sub += 1;
                } else if let Some(n) = self.next_line(r.line) {
                    r = Row { line: n, sub: 0 };
                } else {
                    break;
                }
            }
        } else {
            for _ in 0..delta.unsigned_abs() {
                if r.sub > 0 {
                    r.sub -= 1;
                } else if let Some(p) = self.prev_line(r.line) {
                    r = Row { line: p, sub: self.sub_count(p) - 1 };
                } else {
                    break;
                }
            }
        }
        r
    }

    /// Rows from `a` forward to `b` (`b >= a`), or None beyond `limit`.
    pub fn distance(&self, a: Row, b: Row, limit: usize) -> Option<usize> {
        if b < a {
            return None;
        }
        if self.is_plain() {
            let d = b.line - a.line;
            return (d <= limit).then_some(d);
        }
        let mut r = a;
        for d in 0..=limit {
            if r >= b {
                return Some(d);
            }
            let n = self.advance(r, 1);
            if n == r {
                return None;
            }
            r = n;
        }
        None
    }

    /// `n` rows starting at `from`.
    pub fn rows(&self, from: Row, n: usize) -> Vec<Row> {
        let mut out = Vec::with_capacity(n);
        let mut r = self.clamp(from);
        for _ in 0..n {
            out.push(r);
            let next = self.advance(r, 1);
            if next == r {
                break;
            }
            r = next;
        }
        out
    }

    /// The row showing char `pos`, and its column in cells within that row
    /// (including the continuation indent).
    pub fn row_of(&self, pos: usize) -> (Row, usize) {
        let pos = pos.min(self.rope.len_chars());
        let line = self.rope.char_to_line(pos);
        let vline = self.visible_line(line);
        let ls = self.rope.line_to_char(vline);
        if vline != line {
            // Inside a fold: report the end of the header line.
            let segs = self.segments(vline);
            let last = segs.len() - 1;
            let content: String = line_without_ending(self.rope, vline).into();
            return (Row { line: vline, sub: last }, self.col_in(&content, segs[last], segs[last].end));
        }
        let off = pos - ls;
        let segs = self.segments(line);
        let sub = segs.iter().rposition(|s| s.start <= off).unwrap_or(0);
        let content: String = line_without_ending(self.rope, line).into();
        (Row { line, sub }, self.col_in(&content, segs[sub], off))
    }

    /// Cells from the start of the row to char offset `off` of the line.
    fn col_in(&self, content: &str, seg: Segment, off: usize) -> usize {
        let mut abs = 0usize;
        let mut ci = 0usize;
        let mut col = seg.indent;
        for g in content.graphemes(true) {
            if ci >= off {
                break;
            }
            let w = grapheme_cells(g, abs, self.tab);
            if ci >= seg.start {
                col += w;
            }
            abs += w;
            ci += g.chars().count();
        }
        col
    }

    /// The char at cell `col` of `row` (clamped to the row's text).
    pub fn char_at(&self, row: Row, col: usize) -> usize {
        let row = self.clamp(row);
        let ls = self.rope.line_to_char(row.line);
        let content = line_without_ending(self.rope, row.line);
        if self.wrap.is_none() {
            return ls + char_at_visual_col(content, col, self.tab);
        }
        let segs = self.segments(row.line);
        let seg = segs[row.sub.min(segs.len() - 1)];
        let is_last = row.sub + 1 >= segs.len();
        let text: String = content.into();
        let mut abs = 0usize;
        let mut ci = 0usize;
        let mut c = seg.indent;
        for g in text.graphemes(true) {
            let w = grapheme_cells(g, abs, self.tab);
            if ci >= seg.start {
                if ci >= seg.end {
                    break;
                }
                if c + w > col {
                    return ls + ci;
                }
                c += w;
            }
            abs += w;
            ci += g.chars().count();
        }
        // Past the end of the row: a wrapped row ends before its last char
        // so the cursor stays on it; the last row ends at the line end.
        let end = if is_last { seg.end } else { seg.end.saturating_sub(1).max(seg.start) };
        ls + end
    }

    /// Hidden lines before `line` (O(folds)).
    pub fn hidden_before(&self, line: usize) -> usize {
        self.hidden.iter().take_while(|(a, _)| *a < line).map(|(a, b)| (*b).min(line - 1) + 1 - a).sum()
    }

    /// Visible line count (for the scrollbar).
    pub fn visible_lines(&self) -> usize {
        let hidden: usize = self.hidden.iter().map(|(a, b)| b + 1 - a).sum();
        self.rope.len_lines().saturating_sub(hidden).max(1)
    }

    /// Hidden char ranges, for horizontal movement that must skip folds.
    pub fn hidden_chars(&self) -> Vec<(usize, usize)> {
        self.hidden
            .iter()
            .map(|(a, b)| {
                let header = a - 1;
                let s = self.rope.line_to_char(header) + line_without_ending(self.rope, header).len_chars();
                let e = self.rope.line_to_char(*b) + line_without_ending(self.rope, *b).len_chars();
                (s, e)
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wraps_at_word_boundaries_with_indent() {
        let s = wrap_line("    alpha beta gamma delta", 14, 4);
        let parts: Vec<String> = s
            .iter()
            .map(|g| "    alpha beta gamma delta".chars().skip(g.start).take(g.end - g.start).collect())
            .collect();
        assert_eq!(parts, vec!["    alpha beta ", "gamma ", "delta"]);
        assert_eq!(s[1].indent, 4);
        // A word longer than the width is broken hard.
        let s = wrap_line("abcdefghij", 4, 4);
        assert_eq!(s.len(), 3);
        // Wide characters never straddle the edge.
        let s = wrap_line("中文中文中文", 5, 4);
        assert!(s.iter().all(|g| g.end - g.start <= 2));
    }

    #[test]
    fn folds_hide_lines_and_rows_skip_them() {
        let rope = Rope::from_str("a {\n  b\n  c\n}\nd\n");
        let f = Fold::from_lines(&rope, 0, 2).unwrap();
        let m = DisplayMap::new(&rope, &[f], None, 4);
        assert!(m.is_folded_header(0));
        assert!(m.is_hidden(1) && m.is_hidden(2) && !m.is_hidden(3));
        let rows: Vec<usize> = m.rows(Row::default(), 10).iter().map(|r| r.line).collect();
        assert_eq!(rows, vec![0, 3, 4, 5]);
        assert_eq!(m.advance(Row { line: 3, sub: 0 }, -1).line, 0);
        assert_eq!(m.visible_lines(), 4);
        // A char inside the fold maps to the end of the header row.
        let (row, col) = m.row_of(rope.line_to_char(2) + 1);
        assert_eq!((row.line, col), (0, 3));
    }

    #[test]
    fn edits_inside_a_fold_reveal_it() {
        let rope = Rope::from_str("a {\n  b\n}\nx\n");
        let mut folds = vec![Fold::from_lines(&rope, 0, 1).unwrap()];
        // Typing on line 3 shifts nothing before it; fold stays.
        let at = rope.line_to_char(3);
        map_folds(&mut folds, &ChangeSet::new(vec![yace_buffer::Change::insert(at, "y")], rope.len_chars()));
        assert_eq!(folds.len(), 1);
        // Typing at the end of the header keeps the fold (anchor moves).
        map_folds(&mut folds, &ChangeSet::new(vec![yace_buffer::Change::insert(3, " ")], rope.len_chars() + 1));
        assert_eq!(folds.len(), 1);
        assert_eq!(folds[0].start, 4);
        // Editing a hidden line reveals it.
        map_folds(&mut folds, &ChangeSet::new(vec![yace_buffer::Change::insert(7, "z")], rope.len_chars() + 2));
        assert!(folds.is_empty());
    }

    #[test]
    fn wrapped_rows_and_hit_testing() {
        let rope = Rope::from_str("one two three four\nend");
        let m = DisplayMap::new(&rope, &[], Some(10), 4);
        let rows = m.rows(Row::default(), 10);
        assert_eq!(rows.len(), 3, "{rows:?}");
        assert_eq!(rows[1], Row { line: 0, sub: 1 });
        let (r, c) = m.row_of(8); // 't' of "three"
        assert_eq!((r.sub, c), (1, 0));
        assert_eq!(m.char_at(Row { line: 0, sub: 1 }, 2), 10);
        // Past the end of a wrapped row stays on that row.
        let p = m.char_at(Row { line: 0, sub: 0 }, 50);
        assert_eq!(m.row_of(p).0.sub, 0);
        assert_eq!(m.distance(Row::default(), Row { line: 1, sub: 0 }, 10), Some(2));
    }
}
