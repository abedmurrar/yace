//! Editing operations. Each takes the current rope and selection and returns
//! an [`Edit`]: the change set to apply, the resulting selection and the
//! history kind. Nothing here touches the document directly, so every
//! operation is unit-testable and multi-cursor aware by construction.

use ropey::Rope;

use crate::{
    changes::{Assoc, Change, ChangeSet},
    history::EditKind,
    line_ending::{LineEnding, normalize},
    movement::{self, Direction, word_at},
    selection::{Range, Selection},
    text::{
        indent_len, indent_str, line_end_char, line_without_ending, next_grapheme_boundary, prev_grapheme_boundary,
        visual_col,
    },
};

#[derive(Clone, Copy, Debug)]
pub struct EditOptions {
    pub tab_width: usize,
    pub insert_spaces: bool,
    pub auto_close: bool,
    pub eol: LineEnding,
}

impl Default for EditOptions {
    fn default() -> Self {
        Self { tab_width: 4, insert_spaces: true, auto_close: true, eol: LineEnding::Lf }
    }
}

impl EditOptions {
    pub fn indent_unit(&self) -> String {
        if self.insert_spaces { " ".repeat(self.tab_width.max(1)) } else { "\t".to_string() }
    }
}

#[derive(Debug, Clone)]
pub struct Edit {
    pub changes: ChangeSet,
    pub selection: Selection,
    pub kind: EditKind,
}

impl Edit {
    fn new(changes: ChangeSet, selection: Selection, kind: EditKind) -> Self {
        Self { changes, selection, kind }
    }
}

const PAIRS: &[(char, char)] = &[('(', ')'), ('[', ']'), ('{', '}'), ('"', '"'), ('\'', '\''), ('`', '`')];

fn closer_for(c: char) -> Option<char> {
    PAIRS.iter().find(|(o, _)| *o == c).map(|(_, c)| *c)
}
fn is_closer(c: char) -> bool {
    PAIRS.iter().any(|(_, cl)| *cl == c)
}
fn is_quote(c: char) -> bool {
    matches!(c, '"' | '\'' | '`')
}

/// Build an edit where every range is replaced by `f(range) -> (text,
/// cursor offset within text)`; resulting cursors are placed explicitly.
fn replace_each(
    rope: &Rope,
    sel: &Selection,
    kind: EditKind,
    mut f: impl FnMut(Range) -> (Range, String, usize),
) -> Edit {
    // f returns (range to replace, replacement, cursor offset in replacement)
    let mut changes = Vec::with_capacity(sel.len());
    let mut cursors = Vec::with_capacity(sel.len());
    for r in sel.ranges() {
        let (target, text, cursor) = f(*r);
        cursors.push((target.from(), cursor));
        changes.push(Change::replace(target.from(), target.to(), text));
    }
    let cs = ChangeSet::new(changes, rope.len_chars());
    let ranges = cursors.into_iter().map(|(from, off)| {
        let base = cs.map_pos(from, Assoc::Before);
        Range::point(base + off)
    });
    let selection = Selection::new(ranges, sel.primary_index());
    Edit::new(cs, selection, kind)
}

/// Insert (paste) text at every range, replacing selections. Line endings in
/// `text` are normalised to the document's.
pub fn insert_text(rope: &Rope, sel: &Selection, text: &str, opts: &EditOptions) -> Edit {
    let text = normalize(text, opts.eol);
    let n = text.chars().count();
    replace_each(rope, sel, EditKind::Other, |r| (r, text.clone(), n))
}

/// Type a single character with auto-closing, wrapping and type-over.
pub fn type_char(rope: &Rope, sel: &Selection, ch: char, opts: &EditOptions) -> Edit {
    let len = rope.len_chars();
    let mut changes = Vec::new();
    let mut new_ranges = Vec::new();
    for r in sel.ranges() {
        let next = (r.to() < len).then(|| rope.char(r.to()));
        let prev = (r.from() > 0).then(|| rope.char(r.from() - 1));
        if opts.auto_close && r.is_empty() && is_closer(ch) && next == Some(ch) {
            // Type over an existing closer.
            new_ranges.push(Placement::Shift(r.head, 1));
            continue;
        }
        if opts.auto_close
            && let Some(close) = closer_for(ch)
        {
            if !r.is_empty() {
                // Surround the selection.
                changes.push(Change::insert(r.from(), ch.to_string()));
                changes.push(Change::insert(r.to(), close.to_string()));
                new_ranges.push(Placement::Wrap(r.from(), r.to()));
                continue;
            }
            let next_ok = next.is_none_or(|c| c.is_whitespace() || is_closer(c) || matches!(c, ',' | ';' | '.' | ':'));
            let prev_ok = !is_quote(ch) || prev.is_none_or(|c| !(c.is_alphanumeric() || c == '_' || c == ch));
            if next_ok && prev_ok {
                changes.push(Change::insert(r.head, format!("{ch}{close}")));
                new_ranges.push(Placement::After(r.head, 1));
                continue;
            }
        }
        changes.push(Change::replace(r.from(), r.to(), ch.to_string()));
        new_ranges.push(Placement::After(r.from(), 1));
    }
    let cs = ChangeSet::new(changes, len);
    let ranges = new_ranges.into_iter().map(|p| match p {
        Placement::Shift(pos, by) => Range::point(cs.map_pos(pos, Assoc::After) + by),
        Placement::After(pos, by) => Range::point(cs.map_pos(pos, Assoc::Before) + by),
        Placement::Wrap(from, to) => Range::new(cs.map_pos(from, Assoc::Before) + 1, cs.map_pos(to, Assoc::Before)),
    });
    let selection = Selection::new(ranges, sel.primary_index());
    Edit::new(cs, selection, EditKind::Insert)
}

enum Placement {
    Shift(usize, usize),
    After(usize, usize),
    Wrap(usize, usize),
}

/// Enter: newline + auto-indent; splits `{|}` into three lines.
pub fn insert_newline(rope: &Rope, sel: &Selection, opts: &EditOptions) -> Edit {
    let len = rope.len_chars();
    let eol = opts.eol.as_str();
    replace_each(rope, sel, EditKind::Other, |r| {
        let line = rope.char_to_line(r.from());
        let content = line_without_ending(rope, line);
        let line_start = rope.line_to_char(line);
        let indent = indent_str(content);
        // If the cursor sits inside the leading whitespace, don't duplicate it.
        let indent = if r.from() - line_start < indent.chars().count() {
            indent.chars().take(r.from() - line_start).collect()
        } else {
            indent
        };
        let prev = (r.from() > line_start).then(|| rope.char(r.from() - 1));
        let next = (r.to() < len).then(|| rope.char(r.to()));
        let opens = prev.is_some_and(|c| matches!(c, '(' | '[' | '{'));
        if opens && next.is_some_and(|c| closer_for(prev.unwrap()) == Some(c)) {
            let inner = format!("{eol}{indent}{}", opts.indent_unit());
            let text = format!("{inner}{eol}{indent}");
            let cursor = inner.chars().count();
            return (r, text, cursor);
        }
        let extra = if opens { opts.indent_unit() } else { String::new() };
        let text = format!("{eol}{indent}{extra}");
        let n = text.chars().count();
        (r, text, n)
    })
}

/// Backspace: deletes selections, a grapheme, an auto-closed pair, or back
/// to the previous tab stop inside leading spaces.
pub fn backspace(rope: &Rope, sel: &Selection, opts: &EditOptions) -> Edit {
    let len = rope.len_chars();
    let s = rope.slice(..);
    let mut changes = Vec::new();
    for r in sel.ranges() {
        if !r.is_empty() {
            changes.push(Change::delete(r.from(), r.to()));
            continue;
        }
        let pos = r.head;
        if pos == 0 {
            continue;
        }
        let line = rope.char_to_line(pos);
        let line_start = rope.line_to_char(line);
        let before: String = rope.slice(line_start..pos).into();
        if opts.insert_spaces && !before.is_empty() && before.chars().all(|c| c == ' ') {
            let col = before.chars().count();
            let tw = opts.tab_width.max(1);
            let back = if col.is_multiple_of(tw) { tw } else { col % tw };
            changes.push(Change::delete(pos - back.min(col), pos));
            continue;
        }
        let prev = rope.char(pos - 1);
        if opts.auto_close && pos < len && closer_for(prev).is_some_and(|c| c == rope.char(pos)) {
            changes.push(Change::delete(pos - 1, pos + 1));
            continue;
        }
        changes.push(Change::delete(prev_grapheme_boundary(s, pos), pos));
    }
    let cs = ChangeSet::new(changes, len);
    let selection = sel.map(&cs);
    Edit::new(cs, selection, EditKind::Delete)
}

/// Delete: deletes selections or the next grapheme.
pub fn delete_forward(rope: &Rope, sel: &Selection) -> Edit {
    let s = rope.slice(..);
    let changes = sel
        .ranges()
        .iter()
        .map(|r| {
            if r.is_empty() {
                Change::delete(r.head, next_grapheme_boundary(s, r.head))
            } else {
                Change::delete(r.from(), r.to())
            }
        })
        .collect();
    let cs = ChangeSet::new(changes, rope.len_chars());
    let selection = sel.map(&cs);
    Edit::new(cs, selection, EditKind::Delete)
}

/// Ctrl+Backspace / Ctrl+Delete.
pub fn delete_word(rope: &Rope, sel: &Selection, dir: Direction) -> Edit {
    let changes = sel
        .ranges()
        .iter()
        .map(|r| {
            if !r.is_empty() {
                return Change::delete(r.from(), r.to());
            }
            let other = movement::word_boundary(rope, r.head, dir);
            Change::delete(r.head.min(other), r.head.max(other))
        })
        .collect();
    let cs = ChangeSet::new(changes, rope.len_chars());
    let selection = sel.map(&cs);
    Edit::new(cs, selection, EditKind::Other)
}

/// Lines covered by a range. A multi-line selection ending at column 0
/// does not include that last line (VS Code behaviour).
pub fn range_lines(rope: &Rope, r: &Range) -> (usize, usize) {
    let start = rope.char_to_line(r.from());
    let mut end = rope.char_to_line(r.to());
    if end > start && rope.line_to_char(end) == r.to() {
        end -= 1;
    }
    (start, end)
}

/// Merge the line blocks of all ranges into sorted, disjoint blocks.
fn line_blocks(rope: &Rope, sel: &Selection) -> Vec<(usize, usize)> {
    let mut blocks: Vec<(usize, usize)> = Vec::new();
    for r in sel.ranges() {
        let (s, e) = range_lines(rope, r);
        if let Some(last) = blocks.last_mut()
            && s <= last.1 + 1
        {
            last.1 = last.1.max(e);
            continue;
        }
        blocks.push((s, e));
    }
    blocks
}

/// Tab: indent lines for multi-line selections, otherwise insert an indent
/// at each cursor (to the next tab stop when using spaces).
pub fn indent(rope: &Rope, sel: &Selection, opts: &EditOptions) -> Edit {
    let multi_line = sel.ranges().iter().any(|r| {
        let (s, e) = range_lines(rope, r);
        e > s
    });
    if multi_line {
        return indent_lines(rope, sel, opts);
    }
    let unit = opts.indent_unit();
    replace_each(rope, sel, EditKind::Other, |r| {
        let text = if opts.insert_spaces {
            let line = rope.char_to_line(r.from());
            let col = visual_col(line_without_ending(rope, line), r.from() - rope.line_to_char(line), opts.tab_width);
            " ".repeat(opts.tab_width.max(1) - col % opts.tab_width.max(1))
        } else {
            unit.clone()
        };
        let n = text.chars().count();
        (r, text, n)
    })
}

pub fn indent_lines(rope: &Rope, sel: &Selection, opts: &EditOptions) -> Edit {
    let unit = opts.indent_unit();
    let mut changes = Vec::new();
    for (s, e) in line_blocks(rope, sel) {
        for line in s..=e {
            if line_without_ending(rope, line).len_chars() == 0 {
                continue;
            }
            changes.push(Change::insert(rope.line_to_char(line), unit.clone()));
        }
    }
    let cs = ChangeSet::new(changes, rope.len_chars());
    let selection = sel.transform(|r| {
        // Expand selections to include the inserted indent at their start.
        let anchor_assoc = if r.anchor <= r.head { Assoc::Before } else { Assoc::After };
        let head_assoc = if r.anchor <= r.head { Assoc::After } else { Assoc::Before };
        let (anchor_assoc, head_assoc) =
            if r.is_empty() { (Assoc::After, Assoc::After) } else { (anchor_assoc, head_assoc) };
        Range::new(cs.map_pos(r.anchor, anchor_assoc), cs.map_pos(r.head, head_assoc))
    });
    Edit::new(cs, selection, EditKind::Other)
}

/// Shift+Tab: remove one indent level from every touched line.
pub fn outdent(rope: &Rope, sel: &Selection, opts: &EditOptions) -> Edit {
    let tw = opts.tab_width.max(1);
    let mut changes = Vec::new();
    for (s, e) in line_blocks(rope, sel) {
        for line in s..=e {
            let content = line_without_ending(rope, line);
            let start = rope.line_to_char(line);
            let n = match content.chars().next() {
                Some('\t') => 1,
                Some(' ') => content.chars().take(tw).take_while(|c| *c == ' ').count(),
                _ => 0,
            };
            if n > 0 {
                changes.push(Change::delete(start, start + n));
            }
        }
    }
    let cs = ChangeSet::new(changes, rope.len_chars());
    let selection = sel.map(&cs);
    Edit::new(cs, selection, EditKind::Other)
}

/// Lines `s..=e` as (content, ending) pairs.
fn lines_with_endings(rope: &Rope, s: usize, e: usize) -> Vec<(String, String)> {
    (s..=e)
        .map(|l| {
            let full: String = rope.line(l).into();
            let content: String = line_without_ending(rope, l).into();
            let ending = full[content.len()..].to_string();
            (content, ending)
        })
        .collect()
}

/// Alt+Up / Alt+Down.
pub fn move_lines(rope: &Rope, sel: &Selection, dir: Direction) -> Edit {
    let blocks = line_blocks(rope, sel);
    let last_line = rope.len_lines() - 1;
    let noop = Edit::new(ChangeSet::empty(rope.len_chars()), sel.clone(), EditKind::Other);
    if blocks.is_empty()
        || (dir == Direction::Backward && blocks[0].0 == 0)
        || (dir == Direction::Forward && blocks.last().unwrap().1 >= last_line)
    {
        return noop;
    }
    let mut changes = Vec::new();
    // For each block: region lines, and mapping old line -> new line index
    let mut line_moves: Vec<(usize, usize, isize)> = Vec::new(); // (s, e, delta lines)
    for &(s, e) in &blocks {
        let (rs, re) = if dir == Direction::Backward { (s - 1, e) } else { (s, e + 1) };
        let lines = lines_with_endings(rope, rs, re);
        let endings: Vec<String> = lines.iter().map(|(_, e)| e.clone()).collect();
        let mut contents: Vec<String> = lines.into_iter().map(|(c, _)| c).collect();
        if dir == Direction::Backward {
            contents.rotate_left(1);
        } else {
            contents.rotate_right(1);
        }
        let text: String = contents.iter().zip(&endings).map(|(c, e)| format!("{c}{e}")).collect();
        let from = rope.line_to_char(rs);
        let to = if re + 1 < rope.len_lines() { rope.line_to_char(re + 1) } else { rope.len_chars() };
        changes.push(Change::replace(from, to, text));
        line_moves.push((s, e, if dir == Direction::Backward { -1 } else { 1 }));
    }
    let cs = ChangeSet::new(changes, rope.len_chars());
    // Compute the new rope to place cursors by (line, col).
    let mut new_rope = rope.clone();
    cs.apply(&mut new_rope);
    let selection = sel.transform(|r| {
        let map = |pos: usize| {
            let line = rope.char_to_line(pos);
            let col = pos - rope.line_to_char(line);
            let delta = line_moves.iter().find(|(s, e, _)| line >= *s && line <= *e).map_or(0, |m| m.2);
            let nl = (line as isize + delta) as usize;
            let nl_len = line_without_ending(&new_rope, nl).len_chars();
            new_rope.line_to_char(nl) + col.min(nl_len)
        };
        Range::new(map(r.anchor), map(r.head))
    });
    Edit::new(cs, selection, EditKind::Other)
}

/// Shift+Alt+Up / Shift+Alt+Down: duplicate lines.
pub fn copy_lines(rope: &Rope, sel: &Selection, dir: Direction, opts: &EditOptions) -> Edit {
    let blocks = line_blocks(rope, sel);
    let mut changes = Vec::new();
    let mut shifts: Vec<(usize, usize, usize)> = Vec::new(); // (s, e, inserted chars)
    for &(s, e) in &blocks {
        let start = rope.line_to_char(s);
        let has_next = e + 1 < rope.len_lines();
        let (at, text) = if has_next {
            let end = rope.line_to_char(e + 1);
            let block: String = rope.slice(start..end).into();
            (end, block)
        } else {
            let block: String = rope.slice(start..).into();
            (rope.len_chars(), format!("{}{}", opts.eol.as_str(), block))
        };
        shifts.push((s, e, text.chars().count()));
        changes.push(Change::insert(at, text));
    }
    let cs = ChangeSet::new(changes, rope.len_chars());
    let selection = sel.transform(|r| {
        let line = rope.char_to_line(r.head);
        let mut before = 0;
        let mut own = 0;
        for (s, e, n) in &shifts {
            if *e < line {
                before += n;
            } else if line >= *s && line <= *e {
                own = *n;
            }
        }
        let extra = if dir == Direction::Forward { own } else { 0 };
        Range::new(r.anchor + before + extra, r.head + before + extra)
    });
    Edit::new(cs, selection, EditKind::Other)
}

/// Ctrl+Shift+K.
pub fn delete_lines(rope: &Rope, sel: &Selection) -> Edit {
    let blocks = line_blocks(rope, sel);
    let mut changes = Vec::new();
    for &(s, e) in &blocks {
        let start = rope.line_to_char(s);
        if e + 1 < rope.len_lines() {
            changes.push(Change::delete(start, rope.line_to_char(e + 1)));
        } else if s > 0 {
            // Last line(s): remove the preceding line break instead.
            changes.push(Change::delete(line_end_char(rope, s - 1), rope.len_chars()));
        } else {
            changes.push(Change::delete(0, rope.len_chars()));
        }
    }
    let cs = ChangeSet::new(changes, rope.len_chars());
    let mut new_rope = rope.clone();
    cs.apply(&mut new_rope);
    let selection = sel.transform(|r| {
        let line = rope.char_to_line(r.head);
        let col = r.head - rope.line_to_char(line);
        let p = cs.map_pos(rope.line_to_char(line), Assoc::After).min(new_rope.len_chars());
        let nl = new_rope.char_to_line(p);
        let len = line_without_ending(&new_rope, nl).len_chars();
        Range::point(new_rope.line_to_char(nl) + col.min(len))
    });
    Edit::new(cs, selection, EditKind::Other)
}

/// Ctrl+Enter (below) / Ctrl+Shift+Enter (above).
pub fn insert_line(rope: &Rope, sel: &Selection, dir: Direction, opts: &EditOptions) -> Edit {
    let eol = opts.eol.as_str();
    replace_each(rope, sel, EditKind::Other, |r| {
        let line = rope.char_to_line(r.head);
        let indent = indent_str(line_without_ending(rope, line));
        match dir {
            Direction::Forward => {
                let at = line_end_char(rope, line);
                let text = format!("{eol}{indent}");
                let n = text.chars().count();
                (Range::point(at), text, n)
            }
            Direction::Backward => {
                let at = rope.line_to_char(line);
                let n = indent.chars().count();
                (Range::point(at), format!("{indent}{eol}"), n)
            }
        }
    })
}

/// Ctrl+/: toggle line comments with `token` (e.g. "//", "#").
pub fn toggle_line_comment(rope: &Rope, sel: &Selection, token: &str) -> Edit {
    let blocks = line_blocks(rope, sel);
    let tok_len = token.chars().count();
    let lines: Vec<usize> = blocks.iter().flat_map(|(s, e)| *s..=*e).collect();
    let non_blank: Vec<usize> =
        lines.iter().copied().filter(|l| line_without_ending(rope, *l).chars().any(|c| !c.is_whitespace())).collect();
    let all_commented = !non_blank.is_empty()
        && non_blank.iter().all(|l| {
            let c = line_without_ending(rope, *l);
            let i = indent_len(c);
            c.slice(i..).chars().take(tok_len).eq(token.chars())
        });
    let mut changes = Vec::new();
    if all_commented {
        for l in &non_blank {
            let c = line_without_ending(rope, *l);
            let i = indent_len(c);
            let start = rope.line_to_char(*l) + i;
            let mut end = start + tok_len;
            if c.len_chars() > i + tok_len && c.char(i + tok_len) == ' ' {
                end += 1;
            }
            changes.push(Change::delete(start, end));
        }
    } else {
        let min_indent = non_blank.iter().map(|l| indent_len(line_without_ending(rope, *l))).min().unwrap_or(0);
        for l in &non_blank {
            changes.push(Change::insert(rope.line_to_char(*l) + min_indent, format!("{token} ")));
        }
    }
    let cs = ChangeSet::new(changes, rope.len_chars());
    let selection = sel.map(&cs);
    Edit::new(cs, selection, EditKind::Other)
}

/// Ctrl+L: select the current line(s); repeated presses extend downwards.
pub fn select_line(rope: &Rope, sel: &Selection) -> Selection {
    sel.transform(|r| {
        let (s, e) = range_lines(rope, &r);
        let start = rope.line_to_char(s);
        let line_end = movement::line_range(rope, e).to();
        // Already a full-line selection? extend by one more line.
        if r.from() == start && r.to() == line_end && e + 1 < rope.len_lines() {
            return Range::new(start, movement::line_range(rope, e + 1).to());
        }
        Range::new(start, line_end)
    })
}

pub fn select_all(rope: &Rope) -> Selection {
    Selection::single(Range::new(0, rope.len_chars()))
}

/// Find `needle` occurrences (case-sensitive, non-overlapping).
fn find_all(hay: &str, needle: &str) -> Vec<usize> {
    if needle.is_empty() {
        return Vec::new();
    }
    // Map byte offsets to char offsets incrementally.
    let mut out = Vec::new();
    let mut last_byte = 0;
    let mut last_char = 0;
    let mut start = 0;
    while let Some(i) = hay[start..].find(needle) {
        let b = start + i;
        last_char += hay[last_byte..b].chars().count();
        last_byte = b;
        out.push(last_char);
        start = b + needle.len();
    }
    out
}

/// Ctrl+D. With an empty primary selection, selects the word under every
/// cursor; otherwise adds the next occurrence of the primary selection's
/// text (wrapping around) as a new primary selection.
pub fn add_next_match(rope: &Rope, sel: &Selection) -> Selection {
    let primary = sel.primary();
    if primary.is_empty() {
        return sel.transform(|r| if r.is_empty() { word_at(rope, r.head) } else { r });
    }
    let needle: String = rope.slice(primary.from()..primary.to()).into();
    let hay = rope.to_string();
    let n = needle.chars().count();
    let hits = find_all(&hay, &needle);
    let after = sel.ranges().iter().map(Range::to).max().unwrap_or(0);
    let taken = |p: usize| sel.ranges().iter().any(|r| r.from() == p);
    let next =
        hits.iter().copied().find(|p| *p >= after && !taken(*p)).or_else(|| hits.iter().copied().find(|p| !taken(*p)));
    match next {
        Some(p) => sel.clone().push(Range::new(p, p + n)),
        None => sel.clone(),
    }
}

/// Ctrl+Shift+L: select all occurrences of the primary selection (or word).
pub fn select_all_matches(rope: &Rope, sel: &Selection) -> Selection {
    let mut primary = sel.primary();
    if primary.is_empty() {
        primary = word_at(rope, primary.head);
        if primary.is_empty() {
            return sel.clone();
        }
    }
    let needle: String = rope.slice(primary.from()..primary.to()).into();
    let n = needle.chars().count();
    let hits = find_all(&rope.to_string(), &needle);
    let primary_idx = hits.iter().position(|p| *p == primary.from()).unwrap_or(0);
    Selection::new(hits.into_iter().map(|p| Range::new(p, p + n)), primary_idx)
}

/// Ctrl+Shift+Up/Down: add a cursor on the line above/below the outermost cursor.
pub fn add_cursor_vertical(rope: &Rope, sel: &Selection, dir: Direction, tab_width: usize) -> Selection {
    let edge = match dir {
        Direction::Backward => sel.ranges()[0],
        Direction::Forward => *sel.ranges().last().unwrap(),
    };
    let line = rope.char_to_line(edge.head);
    if (dir == Direction::Backward && line == 0) || (dir == Direction::Forward && line + 1 >= rope.len_lines()) {
        return sel.clone();
    }
    let moved = movement::move_vertical(rope, Range::point(edge.head), dir, 1, false, tab_width);
    sel.clone().push(Range::point(moved.head))
}

/// Remove trailing whitespace on every line.
pub fn trim_trailing_whitespace(rope: &Rope) -> ChangeSet {
    let mut changes = Vec::new();
    for l in 0..rope.len_lines() {
        let c = line_without_ending(rope, l);
        let len = c.len_chars();
        let mut i = len;
        while i > 0 && matches!(c.char(i - 1), ' ' | '\t') {
            i -= 1;
        }
        if i < len {
            let start = rope.line_to_char(l);
            changes.push(Change::delete(start + i, start + len));
        }
    }
    ChangeSet::new(changes, rope.len_chars())
}

/// Ensure the document ends with exactly one line break (adds only).
pub fn ensure_final_newline(rope: &Rope, eol: LineEnding) -> ChangeSet {
    let len = rope.len_chars();
    if len == 0 || matches!(rope.char(len - 1), '\n' | '\r') {
        return ChangeSet::empty(len);
    }
    ChangeSet::new(vec![Change::insert(len, eol.as_str())], len)
}

/// Convert all line endings (one undo step).
pub fn convert_line_endings(rope: &Rope, to: LineEnding) -> ChangeSet {
    let text = rope.to_string();
    let converted = normalize(&text, to);
    if converted == text {
        return ChangeSet::empty(rope.len_chars());
    }
    ChangeSet::new(vec![Change::replace(0, rope.len_chars(), converted)], rope.len_chars())
}

/// Clipboard payload, multi-cursor aware.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ClipboardText {
    pub text: String,
    /// One piece per cursor, when copied from several cursors.
    pub pieces: Option<Vec<String>>,
    /// Copied whole lines (no selection) — paste inserts above the line.
    pub line_mode: bool,
}

/// Ctrl+C. Empty selections copy their whole line.
pub fn copy(rope: &Rope, sel: &Selection, eol: LineEnding) -> ClipboardText {
    if !sel.has_non_empty() {
        let lines: Vec<String> = line_blocks(rope, sel)
            .into_iter()
            .map(|(s, e)| {
                let start = rope.line_to_char(s);
                let end = movement::line_range(rope, e).to();
                let mut t: String = rope.slice(start..end).into();
                if !t.ends_with('\n') && !t.ends_with('\r') {
                    t.push_str(eol.as_str());
                }
                t
            })
            .collect();
        let text = lines.concat();
        let pieces = (lines.len() > 1).then_some(lines);
        return ClipboardText { text, pieces, line_mode: true };
    }
    let pieces: Vec<String> = sel.ranges().iter().map(|r| rope.slice(r.from()..r.to()).into()).collect();
    let text = pieces.join(eol.as_str());
    ClipboardText { text, pieces: (pieces.len() > 1).then_some(pieces), line_mode: false }
}

/// Ctrl+X: copy then delete (whole lines when nothing is selected).
pub fn cut(rope: &Rope, sel: &Selection, eol: LineEnding) -> (ClipboardText, Edit) {
    let clip = copy(rope, sel, eol);
    let edit = if clip.line_mode {
        delete_lines(rope, sel)
    } else {
        let changes = sel.ranges().iter().map(|r| Change::delete(r.from(), r.to())).collect();
        let cs = ChangeSet::new(changes, rope.len_chars());
        let s = sel.map(&cs);
        Edit::new(cs, s, EditKind::Other)
    };
    (clip, edit)
}

/// Ctrl+V.
pub fn paste(rope: &Rope, sel: &Selection, clip: &ClipboardText, opts: &EditOptions) -> Edit {
    if let Some(pieces) = &clip.pieces
        && pieces.len() == sel.len()
        && !clip.line_mode
    {
        let mut i = 0;
        return replace_each(rope, sel, EditKind::Other, |r| {
            let t = normalize(&pieces[i], opts.eol);
            i += 1;
            let n = t.chars().count();
            (r, t, n)
        });
    }
    if clip.line_mode && !sel.has_non_empty() {
        let text = normalize(&clip.text, opts.eol);
        let changes = sel
            .ranges()
            .iter()
            .map(|r| Change::insert(rope.line_to_char(rope.char_to_line(r.head)), text.clone()))
            .collect();
        let cs = ChangeSet::new(changes, rope.len_chars());
        let s = sel.transform(|r| Range::point(cs.map_pos(r.head, Assoc::After)));
        return Edit::new(cs, s, EditKind::Other);
    }
    insert_text(rope, sel, &clip.text, opts)
}

/// Position of the bracket matching the one at or just before `pos`.
pub fn matching_bracket(rope: &Rope, pos: usize) -> Option<(usize, usize)> {
    const BR: &[(char, char)] = &[('(', ')'), ('[', ']'), ('{', '}')];
    let len = rope.len_chars();
    let candidates = [pos, pos.wrapping_sub(1)];
    for at in candidates {
        if at >= len {
            continue;
        }
        let c = rope.char(at);
        if let Some((o, cl)) = BR.iter().find(|(o, cl)| *o == c || *cl == c) {
            let forward = c == *o;
            let mut depth = 0usize;
            // Bounded scan to stay cheap on huge files.
            let limit = 200_000;
            if forward {
                for (steps, i) in (at..len).enumerate() {
                    if steps > limit {
                        break;
                    }
                    let ch = rope.char(i);
                    if ch == *o {
                        depth += 1;
                    } else if ch == *cl {
                        depth -= 1;
                        if depth == 0 {
                            return Some((at, i));
                        }
                    }
                }
            } else {
                for (steps, i) in (0..=at).rev().enumerate() {
                    if steps > limit {
                        break;
                    }
                    let ch = rope.char(i);
                    if ch == *cl {
                        depth += 1;
                    } else if ch == *o {
                        depth -= 1;
                        if depth == 0 {
                            return Some((at, i));
                        }
                    }
                }
            }
            return None;
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn apply(text: &str, edit: &Edit) -> String {
        let mut r = Rope::from_str(text);
        edit.changes.apply(&mut r);
        r.to_string()
    }
    fn heads(s: &Selection) -> Vec<usize> {
        s.ranges().iter().map(|r| r.head).collect()
    }
    fn opts() -> EditOptions {
        EditOptions::default()
    }

    #[test]
    fn typing_with_autoclose_typeover_and_wrap() {
        let t = "x";
        let r = Rope::from_str(t);
        let e = type_char(&r, &Selection::point(1), '(', &opts());
        assert_eq!(apply(t, &e), "x()");
        assert_eq!(heads(&e.selection), vec![2]);
        let r2 = Rope::from_str("x()");
        let e2 = type_char(&r2, &Selection::point(2), ')', &opts());
        assert_eq!(apply("x()", &e2), "x()", "type over closer");
        assert_eq!(heads(&e2.selection), vec![3]);
        let e3 = type_char(&r, &Selection::single(Range::new(0, 1)), '"', &opts());
        assert_eq!(apply(t, &e3), "\"x\"");
        assert_eq!(e3.selection.primary(), Range::new(1, 2));
        // No auto-close of quote after a word char (don't -> don't')
        let r4 = Rope::from_str("don");
        let e4 = type_char(&r4, &Selection::point(3), '\'', &opts());
        assert_eq!(apply("don", &e4), "don'");
    }

    #[test]
    fn multi_cursor_typing() {
        let t = "a\nb\nc";
        let r = Rope::from_str(t);
        let sel = Selection::new([Range::point(1), Range::point(3), Range::point(5)], 0);
        let e = type_char(&r, &sel, 'x', &opts());
        assert_eq!(apply(t, &e), "ax\nbx\ncx");
        assert_eq!(heads(&e.selection), vec![2, 5, 8]);
    }

    #[test]
    fn newline_auto_indent_and_brace_split() {
        let t = "    if x {}";
        let r = Rope::from_str(t);
        let e = insert_newline(&r, &Selection::point(10), &opts());
        assert_eq!(apply(t, &e), "    if x {\n        \n    }");
        assert_eq!(heads(&e.selection), vec![19]);
        let t2 = "  foo";
        let e2 = insert_newline(&Rope::from_str(t2), &Selection::point(5), &opts());
        assert_eq!(apply(t2, &e2), "  foo\n  ");
    }

    #[test]
    fn newline_uses_document_eol() {
        let o = EditOptions { eol: LineEnding::CrLf, ..opts() };
        let e = insert_newline(&Rope::from_str("ab"), &Selection::point(1), &o);
        assert_eq!(apply("ab", &e), "a\r\nb");
        let p = insert_text(&Rope::from_str(""), &Selection::point(0), "x\ny", &o);
        assert_eq!(apply("", &p), "x\r\ny");
    }

    #[test]
    fn backspace_cases() {
        let e = backspace(&Rope::from_str("        x"), &Selection::point(8), &opts());
        assert_eq!(apply("        x", &e), "    x", "back to previous tab stop");
        let e = backspace(&Rope::from_str("()"), &Selection::point(1), &opts());
        assert_eq!(apply("()", &e), "", "pair removed");
        let s = "a👨‍👩‍👧";
        let e = backspace(&Rope::from_str(s), &Selection::point(6), &opts());
        assert_eq!(apply(s, &e), "a", "whole grapheme removed");
        let e = backspace(&Rope::from_str("ab"), &Selection::point(0), &opts());
        assert!(e.changes.is_empty());
    }

    #[test]
    fn indent_outdent() {
        let t = "a\nb\nc";
        let r = Rope::from_str(t);
        let sel = Selection::single(Range::new(0, 3));
        let e = indent(&r, &sel, &opts());
        assert_eq!(apply(t, &e), "    a\n    b\nc");
        let t2 = "    a\n\tb\n  c";
        let r2 = Rope::from_str(t2);
        let e2 = outdent(&r2, &Selection::single(Range::new(0, r2.len_chars())), &opts());
        assert_eq!(apply(t2, &e2), "a\nb\nc");
        let e3 = indent(&Rope::from_str("ab"), &Selection::point(1), &opts());
        assert_eq!(apply("ab", &e3), "a   b", "to next tab stop");
    }

    #[test]
    fn move_lines_up_down() {
        let t = "one\ntwo\nthree";
        let r = Rope::from_str(t);
        let e = move_lines(&r, &Selection::point(5), Direction::Backward);
        assert_eq!(apply(t, &e), "two\none\nthree");
        assert_eq!(heads(&e.selection), vec![1]);
        let e = move_lines(&r, &Selection::point(5), Direction::Forward);
        assert_eq!(apply(t, &e), "one\nthree\ntwo");
        assert_eq!(heads(&e.selection), vec![11]);
        let e = move_lines(&r, &Selection::point(0), Direction::Backward);
        assert!(e.changes.is_empty());
        // CRLF preserved positions
        let c = "a\r\nb\r\nc";
        let e = move_lines(&Rope::from_str(c), &Selection::point(0), Direction::Forward);
        assert_eq!(apply(c, &e), "b\r\na\r\nc");
    }

    #[test]
    fn copy_and_delete_lines() {
        let t = "one\ntwo";
        let r = Rope::from_str(t);
        let e = copy_lines(&r, &Selection::point(1), Direction::Forward, &opts());
        assert_eq!(apply(t, &e), "one\none\ntwo");
        assert_eq!(heads(&e.selection), vec![5]);
        let e = copy_lines(&r, &Selection::point(5), Direction::Backward, &opts());
        assert_eq!(apply(t, &e), "one\ntwo\ntwo");
        assert_eq!(heads(&e.selection), vec![5]);
        let e = delete_lines(&r, &Selection::point(5));
        assert_eq!(apply(t, &e), "one");
        let e = delete_lines(&r, &Selection::point(1));
        assert_eq!(apply(t, &e), "two");
    }

    #[test]
    fn comments_toggle() {
        let t = "  a\n\n    b";
        let r = Rope::from_str(t);
        let all = Selection::single(Range::new(0, r.len_chars()));
        let e = toggle_line_comment(&r, &all, "//");
        let commented = apply(t, &e);
        assert_eq!(commented, "  // a\n\n  //   b");
        let r2 = Rope::from_str(&commented);
        let e2 = toggle_line_comment(&r2, &Selection::single(Range::new(0, r2.len_chars())), "//");
        assert_eq!(apply(&commented, &e2), t);
    }

    #[test]
    fn ctrl_d_and_select_all_matches() {
        let r = Rope::from_str("foo bar foo baz foo");
        let s1 = add_next_match(&r, &Selection::point(1));
        assert_eq!(s1.primary(), Range::new(0, 3));
        let s2 = add_next_match(&r, &s1);
        assert_eq!(s2.len(), 2);
        assert_eq!(s2.primary(), Range::new(8, 11));
        let s3 = add_next_match(&r, &s2);
        let s4 = add_next_match(&r, &s3);
        assert_eq!(s4.len(), 3, "no duplicates after wrapping");
        let all = select_all_matches(&r, &Selection::point(17));
        assert_eq!(all.len(), 3);
        assert_eq!(all.primary(), Range::new(16, 19));
    }

    #[test]
    fn clipboard_line_mode_and_pieces() {
        let t = "one\ntwo";
        let r = Rope::from_str(t);
        let c = copy(&r, &Selection::point(1), LineEnding::Lf);
        assert!(c.line_mode);
        assert_eq!(c.text, "one\n");
        let p = paste(&r, &Selection::point(5), &c, &opts());
        assert_eq!(apply(t, &p), "one\none\ntwo");
        assert_eq!(heads(&p.selection), vec![9]);
        let sel = Selection::new([Range::new(0, 3), Range::new(4, 7)], 0);
        let c2 = copy(&r, &sel, LineEnding::Lf);
        assert_eq!(c2.pieces.as_ref().unwrap().len(), 2);
        let swapped = Selection::new([Range::new(4, 7), Range::new(0, 3)], 0);
        let p2 = paste(&r, &swapped, &c2, &opts());
        assert_eq!(apply(t, &p2), "one\ntwo");
        let (_, cut_edit) = cut(&r, &Selection::single(Range::new(0, 3)), LineEnding::Lf);
        assert_eq!(apply(t, &cut_edit), "\ntwo");
    }

    #[test]
    fn select_line_extends() {
        let r = Rope::from_str("a\nb\nc");
        let s = select_line(&r, &Selection::point(0));
        assert_eq!(s.primary(), Range::new(0, 2));
        let s = select_line(&r, &s);
        assert_eq!(s.primary(), Range::new(0, 4));
    }

    #[test]
    fn whitespace_and_final_newline() {
        let t = "a  \nb\t";
        let r = Rope::from_str(t);
        let mut r2 = r.clone();
        trim_trailing_whitespace(&r).apply(&mut r2);
        assert_eq!(r2.to_string(), "a\nb");
        let mut r3 = r2.clone();
        ensure_final_newline(&r2, LineEnding::Lf).apply(&mut r3);
        assert_eq!(r3.to_string(), "a\nb\n");
        assert!(ensure_final_newline(&r3, LineEnding::Lf).is_empty());
    }

    #[test]
    fn brackets_match() {
        let r = Rope::from_str("f(a[1], {b})");
        assert_eq!(matching_bracket(&r, 1), Some((1, 11)));
        assert_eq!(matching_bracket(&r, 12), Some((11, 1)));
        assert_eq!(matching_bracket(&r, 3), Some((3, 5)));
    }
}
