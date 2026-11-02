//! Cursor movement, VS Code semantics. All functions are pure: they take a
//! rope and a range and return the new range.

use ropey::Rope;

use crate::{
    selection::Range,
    text::{
        CharClass, char_at_visual_col, char_class, indent_len, line_end_char, line_without_ending,
        next_grapheme_boundary, prev_grapheme_boundary, visual_col,
    },
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Direction {
    Backward,
    Forward,
}

/// Left/Right by one grapheme. Without `extend`, a non-empty selection
/// collapses to its start/end first (VS Code behaviour).
pub fn move_grapheme(rope: &Rope, r: Range, dir: Direction, extend: bool) -> Range {
    if !extend && !r.is_empty() {
        return Range::point(if dir == Direction::Backward { r.from() } else { r.to() });
    }
    let s = rope.slice(..);
    let head = match dir {
        Direction::Backward => prev_grapheme_boundary(s, r.head),
        Direction::Forward => next_grapheme_boundary(s, r.head),
    };
    r.with_head(head, extend)
}

/// Ctrl+Left/Right: word boundaries. Forward stops at the end of the next
/// word; backward at the start of the previous one. Line breaks are a stop.
pub fn word_boundary(rope: &Rope, pos: usize, dir: Direction) -> usize {
    let len = rope.len_chars();
    match dir {
        Direction::Forward => {
            if pos >= len {
                return len;
            }
            let mut i = pos;
            if char_class(rope.char(i)) == CharClass::LineBreak {
                return next_grapheme_boundary(rope.slice(..), i);
            }
            while i < len && char_class(rope.char(i)) == CharClass::Whitespace {
                i += 1;
            }
            if i < len {
                let cls = char_class(rope.char(i));
                if cls != CharClass::LineBreak {
                    while i < len && char_class(rope.char(i)) == cls {
                        i += 1;
                    }
                }
            }
            i
        }
        Direction::Backward => {
            if pos == 0 {
                return 0;
            }
            let mut i = pos;
            if char_class(rope.char(i - 1)) == CharClass::LineBreak {
                return prev_grapheme_boundary(rope.slice(..), i);
            }
            while i > 0 && char_class(rope.char(i - 1)) == CharClass::Whitespace {
                i -= 1;
            }
            if i > 0 {
                let cls = char_class(rope.char(i - 1));
                if cls != CharClass::LineBreak {
                    while i > 0 && char_class(rope.char(i - 1)) == cls {
                        i -= 1;
                    }
                }
            }
            i
        }
    }
}

pub fn move_word(rope: &Rope, r: Range, dir: Direction, extend: bool) -> Range {
    r.with_head(word_boundary(rope, r.head, dir), extend)
}

/// Up/Down by `count` lines, keeping the sticky visual column.
pub fn move_vertical(rope: &Rope, r: Range, dir: Direction, count: usize, extend: bool, tab_width: usize) -> Range {
    let line = rope.char_to_line(r.head);
    let line_start = rope.line_to_char(line);
    let goal = r.goal.unwrap_or_else(|| visual_col(line_without_ending(rope, line), r.head - line_start, tab_width));
    let last = rope.len_lines().saturating_sub(1);
    let target = match dir {
        Direction::Backward if line == 0 => {
            let mut nr = r.with_head(0, extend);
            nr.goal = Some(goal);
            return if extend || r.is_empty() { nr } else { Range::point(r.from()) };
        }
        Direction::Forward if line >= last => {
            let mut nr = r.with_head(rope.len_chars(), extend);
            nr.goal = Some(goal);
            return nr;
        }
        Direction::Backward => line.saturating_sub(count),
        Direction::Forward => (line + count).min(last),
    };
    let content = line_without_ending(rope, target);
    let head = rope.line_to_char(target) + char_at_visual_col(content, goal, tab_width);
    let mut nr = r.with_head(head, extend);
    nr.goal = Some(goal);
    nr
}

/// Smart Home: first non-whitespace char, or column 0 if already there.
pub fn line_home(rope: &Rope, r: Range, extend: bool) -> Range {
    let line = rope.char_to_line(r.head);
    let start = rope.line_to_char(line);
    let first = start + indent_len(line_without_ending(rope, line));
    let head = if r.head == first { start } else { first };
    r.with_head(head, extend)
}

pub fn line_end(rope: &Rope, r: Range, extend: bool) -> Range {
    let line = rope.char_to_line(r.head);
    let mut nr = r.with_head(line_end_char(rope, line), extend);
    nr.goal = Some(usize::MAX);
    nr
}

pub fn doc_start(r: Range, extend: bool) -> Range {
    r.with_head(0, extend)
}

pub fn doc_end(rope: &Rope, r: Range, extend: bool) -> Range {
    r.with_head(rope.len_chars(), extend)
}

/// The word (or punctuation run) around `pos`, used for double-click and Ctrl+D.
pub fn word_at(rope: &Rope, pos: usize) -> Range {
    let len = rope.len_chars();
    let at = |i: usize| char_class(rope.char(i));
    let cls = if (pos < len && at(pos) == CharClass::Word) || (pos > 0 && at(pos - 1) == CharClass::Word) {
        CharClass::Word
    } else if pos < len && at(pos) != CharClass::LineBreak {
        at(pos)
    } else {
        return Range::point(pos);
    };
    let mut from = pos;
    while from > 0 && at(from - 1) == cls {
        from -= 1;
    }
    let mut to = pos;
    while to < len && at(to) == cls {
        to += 1;
    }
    Range::new(from, to)
}

/// Whole line including its line break.
pub fn line_range(rope: &Rope, line: usize) -> Range {
    let start = rope.line_to_char(line);
    let end = if line + 1 < rope.len_lines() { rope.line_to_char(line + 1) } else { rope.len_chars() };
    Range::new(start, end)
}

#[cfg(test)]
mod tests {
    use super::*;
    use Direction::*;

    fn rope(s: &str) -> Rope {
        Rope::from_str(s)
    }

    #[test]
    fn grapheme_moves_over_emoji_and_collapses() {
        let r = rope("a👨‍👩‍👧b");
        let p = move_grapheme(&r, Range::point(1), Forward, false);
        assert_eq!(p.head, 6);
        assert_eq!(move_grapheme(&r, p, Backward, false).head, 1);
        let sel = Range::new(1, 6);
        assert_eq!(move_grapheme(&r, sel, Backward, false), Range::point(1));
        assert_eq!(move_grapheme(&r, sel, Forward, true).head, 7);
    }

    #[test]
    fn word_moves() {
        let r = rope("foo.bar  baz\nqux");
        assert_eq!(word_boundary(&r, 0, Forward), 3);
        assert_eq!(word_boundary(&r, 3, Forward), 4);
        assert_eq!(word_boundary(&r, 7, Forward), 12);
        assert_eq!(word_boundary(&r, 12, Forward), 13, "line break is its own stop");
        assert_eq!(word_boundary(&r, 12, Backward), 9);
        assert_eq!(word_boundary(&r, 13, Backward), 12);
        assert_eq!(word_boundary(&r, 4, Backward), 3);
    }

    #[test]
    fn vertical_keeps_goal_column_across_short_lines_and_wide_chars() {
        let r = rope("abcdef\nab\n中文字\n");
        let start = Range::point(5);
        let down = move_vertical(&r, start, Forward, 1, false, 4);
        assert_eq!(down.head, 7 + 2, "clamped to end of short line");
        let down2 = move_vertical(&r, down, Forward, 1, false, 4);
        // visual col 5 is inside the 3rd wide char (cells 4-5) -> its start
        assert_eq!(down2.head, 10 + 2);
        let up = move_vertical(&r, down2, Backward, 2, false, 4);
        assert_eq!(up.head, 5, "goal column restored");
        assert_eq!(move_vertical(&r, Range::point(3), Backward, 1, false, 4).head, 0);
        assert_eq!(move_vertical(&r, Range::point(14), Forward, 1, false, 4).head, r.len_chars());
    }

    #[test]
    fn smart_home_toggles() {
        let r = rope("    code");
        let p = line_home(&r, Range::point(8), false);
        assert_eq!(p.head, 4);
        assert_eq!(line_home(&r, p, false).head, 0);
        assert_eq!(line_home(&r, Range::point(0), false).head, 4);
    }

    #[test]
    fn words_at() {
        let r = rope("let foo_bar = 1;");
        assert_eq!(word_at(&r, 6), Range::new(4, 11));
        assert_eq!(word_at(&r, 11), Range::new(4, 11), "end of word still selects it");
    }
}
