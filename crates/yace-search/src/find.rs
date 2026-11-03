//! Find within one document (spec §9.7) and regex-aware replacement
//! (spec §14.3). Matches are reported as char ranges so they map directly
//! onto rope coordinates.

use regex::{Regex, RegexBuilder};

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
}
