//! Unicode helpers: grapheme boundaries over ropes, display-cell widths and
//! character classes.
//!
//! Coordinates used across YACE:
//! - **char index**: Unicode scalar offset into the rope (Ropey's native unit).
//! - **grapheme**: a user-perceived character; cursor movement steps by these.
//! - **cell**: a terminal display column; tabs expand to tab stops, wide
//!   characters take two cells.

use ropey::{Rope, RopeSlice, str_utils::byte_to_char_idx};
use unicode_segmentation::{GraphemeCursor, GraphemeIncomplete, UnicodeSegmentation};
use unicode_width::UnicodeWidthStr;

/// Char index of the grapheme boundary before `char_idx` (0 when at start).
pub fn prev_grapheme_boundary(slice: RopeSlice, char_idx: usize) -> usize {
    if char_idx == 0 {
        return 0;
    }
    let byte_idx = slice.char_to_byte(char_idx);
    let (mut chunk, mut chunk_byte_idx, mut chunk_char_idx, _) = slice.chunk_at_byte(byte_idx);
    let mut gc = GraphemeCursor::new(byte_idx, slice.len_bytes(), true);
    loop {
        match gc.prev_boundary(chunk, chunk_byte_idx) {
            Ok(None) => return 0,
            Ok(Some(n)) => {
                return chunk_char_idx + byte_to_char_idx(chunk, n - chunk_byte_idx);
            }
            Err(GraphemeIncomplete::PrevChunk) => {
                let (c, b, ch, _) = slice.chunk_at_byte(chunk_byte_idx - 1);
                chunk = c;
                chunk_byte_idx = b;
                chunk_char_idx = ch;
            }
            Err(GraphemeIncomplete::PreContext(n)) => {
                let ctx = slice.chunk_at_byte(n - 1).0;
                gc.provide_context(ctx, n - ctx.len());
            }
            Err(_) => unreachable!("unexpected grapheme cursor state"),
        }
    }
}

/// Char index of the grapheme boundary after `char_idx` (len when at end).
pub fn next_grapheme_boundary(slice: RopeSlice, char_idx: usize) -> usize {
    let len = slice.len_chars();
    if char_idx >= len {
        return len;
    }
    let byte_idx = slice.char_to_byte(char_idx);
    let (mut chunk, mut chunk_byte_idx, mut chunk_char_idx, _) = slice.chunk_at_byte(byte_idx);
    let mut gc = GraphemeCursor::new(byte_idx, slice.len_bytes(), true);
    loop {
        match gc.next_boundary(chunk, chunk_byte_idx) {
            Ok(None) => return len,
            Ok(Some(n)) => {
                return chunk_char_idx + byte_to_char_idx(chunk, n - chunk_byte_idx);
            }
            Err(GraphemeIncomplete::NextChunk) => {
                chunk_byte_idx += chunk.len();
                let (c, _, ch, _) = slice.chunk_at_byte(chunk_byte_idx);
                chunk = c;
                chunk_char_idx = ch;
            }
            Err(GraphemeIncomplete::PreContext(n)) => {
                let ctx = slice.chunk_at_byte(n - 1).0;
                gc.provide_context(ctx, n - ctx.len());
            }
            Err(_) => unreachable!("unexpected grapheme cursor state"),
        }
    }
}

/// True when `char_idx` lies on a grapheme boundary.
pub fn is_grapheme_boundary(slice: RopeSlice, char_idx: usize) -> bool {
    if char_idx == 0 || char_idx >= slice.len_chars() {
        return true;
    }
    next_grapheme_boundary(slice, prev_grapheme_boundary(slice, char_idx)) == char_idx
}

/// Display width in cells of one grapheme starting at visual column `col`.
#[inline]
pub fn grapheme_width(g: &str, col: usize, tab_width: usize) -> usize {
    if g == "\t" {
        let tw = tab_width.max(1);
        return tw - (col % tw);
    }
    if g.as_bytes().first().is_some_and(|b| *b < 0x20 || *b == 0x7f) {
        // Control characters are rendered as a single replacement glyph.
        return 1;
    }
    UnicodeWidthStr::width(g).max(if g.is_empty() { 0 } else { 1 })
}

/// Line content without its trailing line break.
pub fn line_without_ending(rope: &Rope, line: usize) -> RopeSlice<'_> {
    let slice = rope.line(line);
    let len = slice.len_chars();
    let trim = ending_len(slice);
    slice.slice(..len - trim)
}

/// Number of chars occupied by the trailing line break of `slice` (0, 1 or 2).
pub fn ending_len(slice: RopeSlice) -> usize {
    let len = slice.len_chars();
    if len == 0 {
        return 0;
    }
    match slice.char(len - 1) {
        '\n' => {
            if len >= 2 && slice.char(len - 2) == '\r' {
                2
            } else {
                1
            }
        }
        '\r' => 1,
        _ => 0,
    }
}

/// Char index of the end of `line`'s content (before its line break).
pub fn line_end_char(rope: &Rope, line: usize) -> usize {
    rope.line_to_char(line) + line_without_ending(rope, line).len_chars()
}

/// Iterate `(char_offset_in_line, grapheme)` over a line slice.
///
/// Allocates only when a line spans several rope chunks.
pub fn graphemes(slice: RopeSlice) -> Vec<(usize, String)> {
    let s: std::borrow::Cow<str> = slice.into();
    let mut out = Vec::with_capacity(s.len());
    let mut chars = 0;
    for g in s.graphemes(true) {
        out.push((chars, g.to_string()));
        chars += g.chars().count();
    }
    out
}

/// Visual (cell) column of `char_in_line` within `line`, expanding tabs.
pub fn visual_col(line: RopeSlice, char_in_line: usize, tab_width: usize) -> usize {
    let s: std::borrow::Cow<str> = line.slice(..char_in_line.min(line.len_chars())).into();
    let mut col = 0;
    for g in s.graphemes(true) {
        col += grapheme_width(g, col, tab_width);
    }
    col
}

/// Char offset within `line` whose grapheme covers visual column `target`.
/// Snaps to the grapheme start; clamps to the line end.
pub fn char_at_visual_col(line: RopeSlice, target: usize, tab_width: usize) -> usize {
    let s: std::borrow::Cow<str> = line.into();
    let mut col = 0;
    let mut chars = 0;
    for g in s.graphemes(true) {
        let w = grapheme_width(g, col, tab_width);
        if col + w > target {
            return chars;
        }
        col += w;
        chars += g.chars().count();
    }
    chars
}

/// Character class used for word movement and double-click selection.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CharClass {
    Word,
    Punctuation,
    Whitespace,
    LineBreak,
}

pub fn char_class(c: char) -> CharClass {
    if c == '\n' || c == '\r' {
        CharClass::LineBreak
    } else if c.is_whitespace() {
        CharClass::Whitespace
    } else if c.is_alphanumeric() || c == '_' {
        CharClass::Word
    } else {
        CharClass::Punctuation
    }
}

/// Count of leading whitespace chars (spaces/tabs) in a line slice.
pub fn indent_len(line: RopeSlice) -> usize {
    line.chars().take_while(|c| *c == ' ' || *c == '\t').count()
}

/// Leading whitespace of a line as a String.
pub fn indent_str(line: RopeSlice) -> String {
    line.chars().take_while(|c| *c == ' ' || *c == '\t').collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r(s: &str) -> Rope {
        Rope::from_str(s)
    }

    #[test]
    fn grapheme_boundaries_emoji_and_combining() {
        // "e" + combining acute, ZWJ family, CJK
        let s = "e\u{301}👨‍👩‍👧中x";
        let rope = r(s);
        let sl = rope.slice(..);
        let b1 = next_grapheme_boundary(sl, 0);
        assert_eq!(b1, 2, "e + combining mark is one grapheme");
        let b2 = next_grapheme_boundary(sl, b1);
        assert_eq!(b2 - b1, 5, "ZWJ family is one grapheme of 5 chars");
        let b3 = next_grapheme_boundary(sl, b2);
        assert_eq!(b3 - b2, 1);
        assert_eq!(prev_grapheme_boundary(sl, b2), b1);
        assert_eq!(prev_grapheme_boundary(sl, b1), 0);
        assert!(!is_grapheme_boundary(sl, 1));
        assert!(is_grapheme_boundary(sl, b2));
    }

    #[test]
    fn crlf_is_one_grapheme() {
        let rope = r("a\r\nb");
        let sl = rope.slice(..);
        assert_eq!(next_grapheme_boundary(sl, 1), 3);
        assert_eq!(prev_grapheme_boundary(sl, 3), 1);
    }

    #[test]
    fn boundaries_across_chunks() {
        // Force many chunks.
        let s: String = "ab👍".repeat(5000);
        let rope = r(&s);
        let sl = rope.slice(..);
        let mut i = 0;
        let mut n = 0;
        while i < sl.len_chars() {
            i = next_grapheme_boundary(sl, i);
            n += 1;
        }
        assert_eq!(n, 15000);
        let mut j = sl.len_chars();
        let mut m = 0;
        while j > 0 {
            j = prev_grapheme_boundary(sl, j);
            m += 1;
        }
        assert_eq!(m, 15000);
    }

    #[test]
    fn widths() {
        assert_eq!(grapheme_width("中", 0, 4), 2);
        assert_eq!(grapheme_width("a", 0, 4), 1);
        assert_eq!(grapheme_width("\t", 0, 4), 4);
        assert_eq!(grapheme_width("\t", 1, 4), 3);
        assert_eq!(grapheme_width("👍", 0, 4), 2);
        assert_eq!(grapheme_width("e\u{301}", 0, 4), 1);
    }

    #[test]
    fn visual_columns_roundtrip() {
        let rope = r("\ta中b\n");
        let line = line_without_ending(&rope, 0);
        assert_eq!(visual_col(line, 0, 4), 0);
        assert_eq!(visual_col(line, 1, 4), 4);
        assert_eq!(visual_col(line, 2, 4), 5);
        assert_eq!(visual_col(line, 3, 4), 7);
        assert_eq!(char_at_visual_col(line, 4, 4), 1);
        assert_eq!(char_at_visual_col(line, 5, 4), 2);
        assert_eq!(char_at_visual_col(line, 6, 4), 2, "inside wide char snaps to its start");
        assert_eq!(char_at_visual_col(line, 99, 4), 4, "clamps to line end");
    }

    #[test]
    fn line_endings_trimmed() {
        let rope = r("a\r\nbb\ncc\rd");
        assert_eq!(rope.len_lines(), 4);
        assert_eq!(line_without_ending(&rope, 0).to_string(), "a");
        assert_eq!(line_without_ending(&rope, 1).to_string(), "bb");
        assert_eq!(line_without_ending(&rope, 2).to_string(), "cc");
        assert_eq!(line_without_ending(&rope, 3).to_string(), "d");
        assert_eq!(line_end_char(&rope, 0), 1);
    }

    #[test]
    fn form_feed_is_not_a_line_break() {
        let rope = r("a\u{c}b\u{2028}c");
        assert_eq!(rope.len_lines(), 1);
    }
}
