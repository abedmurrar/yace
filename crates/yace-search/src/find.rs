//! Find within one document (spec §9.7) and regex-aware replacement
//! (spec §14.3). Matches are reported as char ranges so they map directly
//! onto rope coordinates.

use regex::{Regex, RegexBuilder};
use yace_buffer::{Assoc, ChangeSet, Rope};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FindOptions {
    pub case_sensitive: bool,
    pub whole_word: bool,
    pub regex: bool,
}

/// Build the regex for a query. Literal queries are escaped.
pub fn build_regex(query: &str, opts: FindOptions) -> Result<Regex, String> {
    let body = if opts.regex { query.to_string() } else { regex::escape(query) };
    let body = if opts.whole_word { format!(r"\b(?:{body})\b") } else { body };
    RegexBuilder::new(&body)
        .case_insensitive(!opts.case_sensitive)
        .multi_line(true)
        .size_limit(10 * (1 << 20))
        .build()
        .map_err(|e| match e {
            regex::Error::Syntax(s) => s.lines().last().unwrap_or("invalid regular expression").trim().to_string(),
            other => other.to_string(),
        })
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FindResult {
    /// Char ranges `(from, to)`, sorted.
    pub matches: Vec<(usize, usize)>,
    /// True when the match limit was hit.
    pub truncated: bool,
}

pub const MAX_FIND_MATCHES: usize = 100_000;

/// Find all non-empty matches in `text`, optionally only inside the char
/// range `within`.
pub fn find_in_text(text: &str, re: &Regex, within: Option<(usize, usize)>) -> FindResult {
    let mut out = FindResult::default();
    // Incremental byte → char conversion.
    let mut last_byte = 0usize;
    let mut last_char = 0usize;
    let mut to_char = |b: usize| {
        if b < last_byte {
            last_byte = 0;
            last_char = 0;
        }
        last_char += text[last_byte..b].chars().count();
        last_byte = b;
        last_char
    };
    for m in re.find_iter(text) {
        if m.start() == m.end() {
            continue;
        }
        let from = to_char(m.start());
        let to = to_char(m.end());
        if let Some((a, b)) = within {
            if to <= a {
                continue;
            }
            if from >= b {
                break;
            }
            if from < a || to > b {
                continue;
            }
        }
        if out.matches.len() >= MAX_FIND_MATCHES {
            out.truncated = true;
            break;
        }
        out.matches.push((from, to));
    }
    out
}

/// True when no match of the query can span a line break, so lines can be
/// searched separately (after an edit, only the edited lines). Literal
/// queries never can (the find field is one line); a regex can when it
/// names a line break or uses a class that includes one (`\s`, `[^…]`,
/// `(?s)`…), so those are treated as spanning lines.
pub fn is_line_local(query: &str, opts: FindOptions) -> bool {
    if !opts.regex {
        return !query.contains('\n');
    }
    const SPANNING: [&str; 13] =
        [r"\n", r"\r", r"\s", r"\W", r"\D", r"\x", r"\u", r"\U", r"\p", r"\P", "[^", "(?s", r"\0"];
    !SPANNING.iter().any(|t| query.contains(t)) && !query.contains("(?") && !query.contains('\n')
}

/// Size of the text searched at once by [`find_in_rope`].
const SEGMENT: usize = 64 * 1024;

/// [`find_in_text`] over a rope without copying the whole text: the
/// rope's chunks are gathered into one reused buffer and searched about
/// 64 KiB at a time, cut at line ends. Only valid for a line-local query
/// ([`is_line_local`]); otherwise, and for Find in Selection, the whole
/// text is searched at once.
pub fn find_in_rope(rope: &Rope, re: &Regex, within: Option<(usize, usize)>, line_local: bool) -> FindResult {
    if !line_local || within.is_some() {
        return find_in_text(&rope.to_string(), re, within);
    }
    let mut out = FindResult::default();
    let mut buf = String::with_capacity(2 * SEGMENT);
    let mut seg_start = 0usize; // char offset of `buf`
    let search = |text: &str, seg_start: usize, out: &mut FindResult| -> bool {
        let r = find_in_text(text, re, None);
        out.matches.extend(r.matches.into_iter().map(|(a, b)| (a + seg_start, b + seg_start)));
        if r.truncated || out.matches.len() >= MAX_FIND_MATCHES {
            out.matches.truncate(MAX_FIND_MATCHES);
            out.truncated = true;
            return false;
        }
        true
    };
    for chunk in rope.chunks() {
        buf.push_str(chunk);
        if buf.len() < SEGMENT {
            continue;
        }
        let Some(nl) = buf.rfind('\n') else { continue };
        let head = &buf[..=nl];
        if !search(head, seg_start, &mut out) {
            return out;
        }
        seg_start += head.chars().count();
        buf.drain(..=nl);
    }
    search(&buf, seg_start, &mut out);
    out
}

/// Update `matches` (in the text before `edit`) for the text after it,
/// re-searching only the edited lines (plus one line either side) of
/// `after`. For line-local queries only; returns `None` when more than
/// `max_lines` lines would have to be searched again.
pub fn update_matches(
    matches: &[(usize, usize)],
    edit: &ChangeSet,
    after: &Rope,
    re: &Regex,
    max_lines: usize,
) -> Option<Vec<(usize, usize)>> {
    // Lines of `after` touched by the edit, merged.
    let mut spans: Vec<(usize, usize)> = Vec::new(); // line ranges, inclusive
    let mut delta: isize = 0;
    let last_line = after.len_lines().saturating_sub(1);
    let mut total = 0;
    for c in edit.changes() {
        let from = (c.from as isize + delta) as usize;
        let to = from + c.insert_chars();
        delta += c.insert_chars() as isize - (c.to - c.from) as isize;
        let a = after.char_to_line(from.min(after.len_chars())).saturating_sub(1);
        let b = (after.char_to_line(to.min(after.len_chars())) + 1).min(last_line);
        match spans.last_mut() {
            Some(last) if a <= last.1 + 1 => last.1 = last.1.max(b),
            _ => spans.push((a, b)),
        }
    }
    for (a, b) in &spans {
        total += b - a + 1;
    }
    if total > max_lines {
        return None;
    }
    // Char ranges of those lines in `after`.
    let ranges: Vec<(usize, usize)> = spans
        .iter()
        .map(|(a, b)| {
            let end = if *b + 1 < after.len_lines() { after.line_to_char(b + 1) } else { after.len_chars() };
            (after.line_to_char(*a), end)
        })
        .collect();
    let dirty = |(a, b): (usize, usize)| ranges.iter().any(|(x, y)| a < *y && b > *x || (a == b && a >= *x && a < *y));
    // Old matches that survive, mapped; those in edited lines are dropped.
    let mut out: Vec<(usize, usize)> = matches
        .iter()
        .map(|(a, b)| (edit.map_pos(*a, Assoc::After), edit.map_pos(*b, Assoc::Before)))
        .filter(|(a, b)| a < b && !dirty((*a, *b)))
        .collect();
    let mut buf = String::new();
    for (x, y) in &ranges {
        buf.clear();
        for chunk in after.slice(*x..*y).chunks() {
            buf.push_str(chunk);
        }
        out.extend(find_in_text(&buf, re, None).matches.into_iter().map(|(a, b)| (a + x, b + x)));
    }
    out.sort_unstable();
    out.dedup();
    Some(out)
}

/// The replacement text for the match `text[byte_range]`, expanding `$1`,
/// `${name}` when `regex` mode is on.
pub fn expand_replacement(
    re: &Regex,
    haystack: &str,
    byte_start: usize,
    replacement: &str,
    regex_mode: bool,
) -> String {
    if !regex_mode {
        return replacement.to_string();
    }
    match re.captures_at(haystack, byte_start) {
        Some(caps) => {
            let mut dst = String::new();
            caps.expand(replacement, &mut dst);
            dst
        }
        None => replacement.to_string(),
    }
}

/// Every match with its replacement: `(char_from, char_to, replacement)`.
pub fn replacements(text: &str, re: &Regex, replacement: &str, regex_mode: bool) -> Vec<(usize, usize, String)> {
    let mut out = Vec::new();
    let mut last_byte = 0usize;
    let mut last_char = 0usize;
    for m in re.find_iter(text) {
        if m.start() == m.end() {
            continue;
        }
        last_char += text[last_byte..m.start()].chars().count();
        last_byte = m.start();
        let from = last_char;
        let to = from + m.as_str().chars().count();
        out.push((from, to, expand_replacement(re, text, m.start(), replacement, regex_mode)));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn literal_case_word_regex() {
        let text = "Foo foo food\nfoo.bar";
        let lit = build_regex("foo", FindOptions::default()).unwrap();
        assert_eq!(find_in_text(text, &lit, None).matches.len(), 4);
        let case = build_regex("foo", FindOptions { case_sensitive: true, ..Default::default() }).unwrap();
        assert_eq!(find_in_text(text, &case, None).matches.len(), 3);
        let word = build_regex("foo", FindOptions { whole_word: true, ..Default::default() }).unwrap();
        assert_eq!(find_in_text(text, &word, None).matches, vec![(0, 3), (4, 7), (13, 16)]);
        let dot = build_regex("o.b", FindOptions::default()).unwrap();
        assert_eq!(find_in_text(text, &dot, None).matches, vec![(15, 18)], "literal dot is escaped");
        let re =
            build_regex(r"^f\w+", FindOptions { regex: true, case_sensitive: true, ..Default::default() }).unwrap();
        assert_eq!(find_in_text(text, &re, None).matches, vec![(13, 16)]);
        assert!(build_regex("(", FindOptions { regex: true, ..Default::default() }).is_err());
    }

    #[test]
    fn unicode_char_offsets_and_selection() {
        let text = "中文 abc 中文 abc";
        let re = build_regex("abc", FindOptions::default()).unwrap();
        assert_eq!(find_in_text(text, &re, None).matches, vec![(3, 6), (10, 13)]);
        assert_eq!(find_in_text(text, &re, Some((7, 13))).matches, vec![(10, 13)]);
    }

    #[test]
    fn capture_group_replacement() {
        let re = build_regex(r"(\w+)@(\w+)", FindOptions { regex: true, ..Default::default() }).unwrap();
        let r = replacements("a@b, c@d", &re, "$2@$1", true);
        assert_eq!(r, vec![(0, 3, "b@a".into()), (5, 8, "d@c".into())]);
        let lit = build_regex("x", FindOptions::default()).unwrap();
        assert_eq!(replacements("x$1x", &lit, "$1", false)[0].2, "$1");
    }

    fn rope_find(text: &str, q: &str, opts: FindOptions) -> Vec<(usize, usize)> {
        let re = build_regex(q, opts).unwrap();
        find_in_rope(&Rope::from_str(text), &re, None, is_line_local(q, opts)).matches
    }

    #[test]
    fn rope_search_matches_text_search() {
        let mut text = String::new();
        for i in 0..20_000 {
            text.push_str(&format!("line {i} has foo and é foo\n"));
        }
        let re = build_regex("foo", FindOptions::default()).unwrap();
        let rope = Rope::from_str(&text);
        let full = find_in_text(&text, &re, None);
        assert_eq!(find_in_rope(&rope, &re, None, true), full);
        assert_eq!(
            find_in_rope(&rope, &re, Some((1000, 90_000)), true),
            find_in_text(&text, &re, Some((1000, 90_000)))
        );
        let anchored = build_regex("^line 1", FindOptions { regex: true, ..Default::default() }).unwrap();
        assert_eq!(find_in_rope(&rope, &anchored, None, true), find_in_text(&text, &anchored, None));
        assert_eq!(rope_find("a\nb", r"a\sb", FindOptions { regex: true, ..Default::default() }), vec![(0, 3)]);
    }

    #[test]
    fn line_local_queries() {
        let re = FindOptions { regex: true, ..Default::default() };
        assert!(is_line_local("foo bar", FindOptions::default()));
        assert!(is_line_local(r"fo+\b\w", re));
        for q in [r"a\sb", "a[^x]b", r"(?s)a.b", r"a\nb", r"\W"] {
            assert!(!is_line_local(q, re), "{q}");
        }
    }

    /// Updating matches through random edits equals searching again.
    #[test]
    fn incremental_update_equals_full_search() {
        use yace_buffer::Change;
        let mut seed = 0x2545_f491_4f6c_dd1du64;
        let mut rnd = |n: usize| {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            (seed % n as u64) as usize
        };
        let words = ["foo", "fo", "o", "x", "\n", "foo\n", "bar foo", "f"];
        for q in ["foo", "o f", r"fo+"] {
            let opts = FindOptions { regex: q == r"fo+", ..Default::default() };
            let re = build_regex(q, opts).unwrap();
            let mut rope = Rope::from_str("foo bar\nbaz foo foo\n\nfoofoo\nend");
            let mut matches = find_in_text(&rope.to_string(), &re, None).matches;
            for _ in 0..500 {
                let len = rope.len_chars();
                let a = rnd(len + 1);
                let b = (a + rnd(4)).min(len);
                let ins = if rnd(3) == 0 { String::new() } else { words[rnd(words.len())].to_string() };
                let cs = ChangeSet::new(vec![Change::replace(a, b, ins)], len);
                cs.apply(&mut rope);
                matches = update_matches(&matches, &cs, &rope, &re, 10_000).unwrap();
                assert_eq!(matches, find_in_text(&rope.to_string(), &re, None).matches, "query {q:?}: {rope:?}");
            }
        }
    }
}
