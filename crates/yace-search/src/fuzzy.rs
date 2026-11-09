//! VS Code-style fuzzy matching for file paths and labels (spec §6.3).
//!
//! The query is matched case-insensitively as a subsequence. Among all
//! alignments, a dynamic program picks the one with the best score:
//! matches at word / path-segment starts and contiguous runs score high,
//! gaps cost a little. For paths, a match inside the file name is strongly
//! preferred, with extra bonuses for an exact name or name prefix.
//! Whitespace separates tokens; every token must match.
//!
//! Scoring is deterministic: equal inputs always give equal outputs.

const BASE: i32 = 16;
const CONSECUTIVE: i32 = 12;
const GAP_START: i32 = 3;
const GAP_EXTEND: i32 = 1;
const BONUS_START: i32 = 10;
const BONUS_SEGMENT: i32 = 9;
const BONUS_SEPARATOR: i32 = 8;
const BONUS_CAMEL: i32 = 7;
const FILENAME_MATCH: i32 = 60;
const FILENAME_PREFIX: i32 = 40;
const FILENAME_EXACT: i32 = 200;
const NEG: i32 = i32::MIN / 4;
/// Candidates longer than this are matched on their tail only.
const MAX_LEN: usize = 512;

/// A successful match: higher `score` is better; `indices` are char
/// positions in the candidate to highlight (sorted, unique).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FuzzyMatch {
    pub score: i32,
    pub indices: Vec<usize>,
}

/// A prepared (lowercased, tokenized) query.
#[derive(Clone, Debug, Default)]
pub struct Query {
    tokens: Vec<Vec<char>>,
}

impl Query {
    pub fn new(q: &str) -> Self {
        let tokens = q
            .split_whitespace()
            .map(|t| t.chars().flat_map(char::to_lowercase).filter(|c| *c != '\\').collect::<Vec<char>>())
            .filter(|t| !t.is_empty())
            .collect();
        Self { tokens }
    }
    pub fn is_empty(&self) -> bool {
        self.tokens.is_empty()
    }
    pub fn tokens(&self) -> &[Vec<char>] {
        &self.tokens
    }
}

fn bonus(prev: Option<char>, cur: char) -> i32 {
    match prev {
        None => BONUS_START,
        Some('/') | Some('\\') => BONUS_SEGMENT,
        Some('_') | Some('-') | Some('.') | Some(' ') | Some(':') => BONUS_SEPARATOR,
        Some(p) if (p.is_lowercase() || p.is_ascii_digit()) && cur.is_uppercase() => BONUS_CAMEL,
        Some(p) if !p.is_alphanumeric() => BONUS_SEPARATOR / 2,
        _ => 0,
    }
}

/// Is `q` a (case-insensitive) subsequence of `c`? Allocation-free prefilter.
fn is_subsequence(q: &[char], c: &str) -> bool {
    let mut qi = 0;
    for ch in c.chars().flat_map(char::to_lowercase) {
        if qi < q.len() && ch == q[qi] {
            qi += 1;
            if qi == q.len() {
                return true;
            }
        }
    }
    qi == q.len()
}

/// Reusable DP buffers (avoids per-candidate allocation).
#[derive(Default)]
struct Scratch {
    lower: Vec<char>,
    bonuses: Vec<i32>,
    mm: Vec<i32>,
    consec_from: Vec<bool>,
    g: Vec<i32>,
    g_idx: Vec<usize>,
}

thread_local! {
    static SCRATCH: std::cell::RefCell<Scratch> = std::cell::RefCell::new(Scratch::default());
}

fn is_subsequence_chars(q: &[char], c: &[char]) -> bool {
    let mut qi = 0;
    for ch in c.iter().flat_map(|c| c.to_lowercase()) {
        if ch == q[qi] {
            qi += 1;
            if qi == q.len() {
                return true;
            }
        }
    }
    false
}

/// Best alignment of one token against `cand` (chars). Returns the score
/// and matched indices.
fn align(q: &[char], cand: &[char]) -> Option<(i32, Vec<usize>)> {
    align_opt(q, cand, true)
}

fn align_opt(q: &[char], cand: &[char], want_indices: bool) -> Option<(i32, Vec<usize>)> {
    let (m, n) = (q.len(), cand.len());
    if m == 0 {
        return Some((0, Vec::new()));
    }
    if m > n {
        return None;
    }
    SCRATCH.with(|s| align_with(&mut s.borrow_mut(), q, cand, want_indices))
}

fn align_with(s: &mut Scratch, q: &[char], cand: &[char], want_indices: bool) -> Option<(i32, Vec<usize>)> {
    let (m, n) = (q.len(), cand.len());
    s.lower.clear();
    s.lower.extend(cand.iter().map(|c| c.to_lowercase().next().unwrap_or(*c)));
    s.bonuses.clear();
    s.bonuses.extend((0..n).map(|j| bonus(if j == 0 { None } else { Some(cand[j - 1]) }, cand[j])));
    for v in [&mut s.mm, &mut s.g] {
        v.clear();
        v.resize(m * n, NEG);
    }
    s.consec_from.clear();
    s.consec_from.resize(m * n, false);
    s.g_idx.clear();
    s.g_idx.resize(m * n, usize::MAX);
    let Scratch { lower, bonuses, mm, consec_from, g, g_idx } = s;
    // mm[i][j]: best score with q[i] matched exactly at j.
    // g[i][j]: best of mm[i][k] - gap(j - k) over k <= j (with argmax).
    for (i, qc) in q.iter().enumerate() {
        for j in i..n {
            let idx = i * n + j;
            if lower[j] == *qc {
                let here = BASE + bonuses[j];
                if i == 0 {
                    // Slight preference for early matches.
                    mm[idx] = here - (j as i32).min(12) / 3;
                } else {
                    let prev = (i - 1) * n + j - 1;
                    let cons =
                        if mm[prev] > NEG { mm[prev] + CONSECUTIVE + bonuses[j].max(BONUS_SEPARATOR / 2) } else { NEG };
                    let gap = if g[prev] > NEG { g[prev] - GAP_START } else { NEG };
                    if cons >= gap && cons > NEG {
                        mm[idx] = cons + here;
                        consec_from[idx] = true;
                    } else if gap > NEG {
                        mm[idx] = gap + here;
                    }
                }
            }
            let from_left = if j > 0 && g[idx - 1] > NEG { g[idx - 1] - GAP_EXTEND } else { NEG };
            if mm[idx] >= from_left && mm[idx] > NEG {
                g[idx] = mm[idx];
                g_idx[idx] = j;
            } else if from_left > NEG {
                g[idx] = from_left;
                g_idx[idx] = g_idx[idx - 1];
            }
        }
    }
    let last = (m - 1) * n;
    let (mut best_j, mut best) = (usize::MAX, NEG);
    for j in 0..n {
        if mm[last + j] > best {
            best = mm[last + j];
            best_j = j;
        }
    }
    if best_j == usize::MAX {
        return None;
    }
    if !want_indices {
        return Some((best, Vec::new()));
    }
    let mut indices = vec![0; m];
    let mut j = best_j;
    for i in (0..m).rev() {
        indices[i] = j;
        if i == 0 {
            break;
        }
        let idx = i * n + j;
        j = if consec_from[idx] { j - 1 } else { g_idx[(i - 1) * n + j - 1] };
    }
    Some((best, indices))
}

/// Match a free-form label (command titles, picker items).
pub fn match_label(query: &Query, label: &str) -> Option<FuzzyMatch> {
    if query.is_empty() {
        return Some(FuzzyMatch { score: 0, indices: Vec::new() });
    }
    let chars: Vec<char> = label.chars().collect();
    let mut score = 0;
    let mut indices = Vec::new();
    for t in query.tokens() {
        if !is_subsequence(t, label) {
            return None;
        }
        let (s, idx) = align(t, &chars)?;
        score += s;
        indices.extend(idx);
    }
    indices.sort_unstable();
    indices.dedup();
    // Shorter labels win ties.
    Some(FuzzyMatch { score: score - (chars.len() as i32) / 8, indices })
}

thread_local! {
    static LABEL_CHARS: std::cell::RefCell<Vec<char>> = const { std::cell::RefCell::new(Vec::new()) };
}

/// The score [`match_label`] would give, without computing highlight
/// indices or allocating (ranking long lists, e.g. completion).
pub fn score_label(query: &Query, label: &str) -> Option<i32> {
    if query.is_empty() {
        return Some(0);
    }
    // One character (the first keystroke over a long list): the best
    // single position, without the alignment tables. Same score as below.
    if let [t] = query.tokens()
        && let [qc] = t.as_slice()
    {
        return score_single(*qc, label);
    }
    LABEL_CHARS.with(|buf| {
        let mut chars = buf.borrow_mut();
        chars.clear();
        chars.extend(label.chars());
        let mut score = 0;
        for t in query.tokens() {
            if !is_subsequence(t, label) {
                return None;
            }
            score += align_opt(t, &chars, false)?.0;
        }
        Some(score - (chars.len() as i32) / 8)
    })
}

/// [`score_label`] for a one-character query: the best of
/// `BASE + bonus - min(position, 12) / 3` over the positions of `qc`, minus
/// the length penalty.
fn score_single(qc: char, label: &str) -> Option<i32> {
    let mut best = NEG;
    let mut prev = None;
    let mut len = 0usize;
    for (j, c) in label.chars().enumerate() {
        let lower = if c.is_ascii() { c.to_ascii_lowercase() } else { c.to_lowercase().next().unwrap_or(c) };
        if lower == qc {
            best = best.max(BASE + bonus(prev, c) - (j as i32).min(12) / 3);
        }
        prev = Some(c);
        len = j + 1;
    }
    (best > NEG).then(|| best - (len as i32) / 8)
}

/// Per-token fallback scores for tokens that match the *root name* in a
/// multi-root workspace (e.g. `api index` finds `src/index.ts` in "Global
/// API"). `None` = the token must match the path.
pub fn root_fallbacks(query: &Query, root_name: &str) -> Vec<Option<i32>> {
    let chars: Vec<char> = root_name.chars().collect();
    query
        .tokens()
        .iter()
        .map(|t| if is_subsequence(t, root_name) { align(t, &chars).map(|(s, _)| s / 2) } else { None })
        .collect()
}

/// Match a relative path. File-name matches are preferred.
pub fn match_path(query: &Query, path: &str) -> Option<FuzzyMatch> {
    match_path_in_root(query, path, &[])
}

/// Like [`match_path`]; tokens with a fallback may match the root name
/// instead of the path.
pub fn match_path_in_root(query: &Query, path: &str, fallbacks: &[Option<i32>]) -> Option<FuzzyMatch> {
    if query.is_empty() {
        return Some(FuzzyMatch { score: 0, indices: Vec::new() });
    }
    let fb = |i: usize| fallbacks.get(i).copied().flatten();
    // Prefilter on the whole path for every required token.
    if !query.tokens().iter().enumerate().all(|(i, t)| fb(i).is_some() || is_subsequence(t, path)) {
        return None;
    }
    let all: Vec<char> = path.chars().collect();
    let skip = all.len().saturating_sub(MAX_LEN);
    let chars = &all[skip..];
    let name_start = chars.iter().rposition(|c| *c == '/' || *c == '\\').map_or(0, |i| i + 1);
    let name = &chars[name_start..];
    let depth = chars.iter().filter(|c| **c == '/').count() as i32;
    let mut total = 0;
    let mut indices = Vec::new();
    for (ti, t) in query.tokens().iter().enumerate() {
        if let Some(root_score) = fb(ti)
            && !is_subsequence(t, path)
        {
            total += root_score;
            continue;
        }
        let has_sep = t.contains(&'/');
        let in_name = if has_sep || !is_subsequence_chars(t, name) { None } else { align(t, name) };
        // The full path is only aligned when the name alone doesn't match.
        let in_path = if in_name.is_some() { None } else { align(t, chars) };
        let name_score = in_name.map(|(s, idx)| {
            let mut s = s + FILENAME_MATCH;
            let lname: Vec<char> = name.iter().map(|c| c.to_lowercase().next().unwrap_or(*c)).collect();
            if lname.starts_with(t) {
                s += FILENAME_PREFIX;
                // Exact name, or exact name without extension.
                let stem_len = lname.iter().rposition(|c| *c == '.').unwrap_or(lname.len());
                if lname.len() == t.len() || stem_len == t.len() {
                    s += FILENAME_EXACT;
                }
            }
            (s, idx.into_iter().map(|i| i + name_start + skip).collect::<Vec<_>>())
        });
        let path_score = in_path.map(|(s, idx)| (s, idx.into_iter().map(|i| i + skip).collect::<Vec<_>>()));
        let best = match (name_score, path_score) {
            (Some(a), Some(b)) => {
                if a.0 >= b.0 {
                    a
                } else {
                    b
                }
            }
            (Some(a), None) => a,
            (None, Some(b)) => b,
            (None, None) => return None,
        };
        total += best.0;
        indices.extend(best.1);
    }
    indices.sort_unstable();
    indices.dedup();
    Some(FuzzyMatch { score: total - depth * 2 - (chars.len() as i32) / 16, indices })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn score_label_matches_match_label() {
        for (q, l) in [("ab", "a_b"), ("usst", "useState"), ("x", "nothing"), ("ser jo", "jobs.service.ts"), ("", "x")]
        {
            let q = Query::new(q);
            assert_eq!(score_label(&q, l), match_label(&q, l).map(|m| m.score), "{l}");
        }
    }

    fn rank<'a>(q: &str, items: &[&'a str]) -> Vec<&'a str> {
        let q = Query::new(q);
        let mut v: Vec<(i32, &str)> = items.iter().filter_map(|p| match_path(&q, p).map(|m| (m.score, *p))).collect();
        v.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.len().cmp(&b.1.len())).then(a.1.cmp(b.1)));
        v.into_iter().map(|(_, p)| p).collect()
    }

    #[test]
    fn spec_examples() {
        let files = [
            "src/jobs/jobs.service.ts",
            "src/jobs/jobs.controller.ts",
            "src/auth/auth.controller.ts",
            "src/auth/auth.service.ts",
            "src/observer/job_observer.ts",
        ];
        assert_eq!(rank("jobserv", &files)[0], "src/jobs/jobs.service.ts");
        assert_eq!(rank("authctrl", &files)[0], "src/auth/auth.controller.ts");
        assert!(rank("zzz", &files).is_empty());
    }

    #[test]
    fn exact_beats_prefix_beats_scattered() {
        let files = ["lib/packager.rs", "package.json", "apps/web/package.json", "src/p_a_c_k_a_g_e.rs"];
        let r = rank("package.json", &files);
        assert_eq!(r[0], "package.json");
        assert_eq!(r[1], "apps/web/package.json");
        let r = rank("package", &files);
        assert_eq!(r[0], "package.json", "{r:?}");
        assert_eq!(*r.last().unwrap(), "src/p_a_c_k_a_g_e.rs");
    }

    #[test]
    fn path_segments_and_tokens() {
        let files = ["web/src/index.ts", "api/src/index.ts", "api/test/index.spec.ts"];
        assert_eq!(rank("api index", &files)[0], "api/src/index.ts");
        assert_eq!(rank("web/index", &files)[0], "web/src/index.ts");
        let q = Query::new("api index");
        let fb = root_fallbacks(&q, "Global API");
        assert!(match_path_in_root(&q, "src/index.ts", &fb).is_some(), "api matched the root name");
        assert!(match_path(&q, "src/index.ts").is_none());
        let m = match_path(&Query::new("idx"), "api/src/index.ts").unwrap();
        assert_eq!(m.indices, vec![8, 10, 12]);
    }

    #[test]
    fn deterministic_and_case_insensitive() {
        let q = Query::new("ReadMe");
        let a = match_path(&q, "docs/README.md").unwrap();
        let b = match_path(&q, "docs/README.md").unwrap();
        assert_eq!(a, b);
        assert!(match_path(&Query::new("rdm"), "README.md").is_some());
    }

    #[test]
    fn labels() {
        let q = Query::new("toggle side");
        let a = match_label(&q, "View: Toggle Primary Side Bar Visibility").unwrap();
        assert!(match_label(&q, "File: Save").is_none());
        assert!(a.score > 0);
        let camel = match_label(&Query::new("sab"), "saveAllBuffers").unwrap();
        assert_eq!(camel.indices.first(), Some(&0));
        assert_eq!(camel.indices.last(), Some(&7), "B of Buffers (word start)");
    }

    #[test]
    fn unicode_paths() {
        let m = match_path(&Query::new("lërn"), "apps/lërn-app/main.ts").unwrap();
        assert_eq!(m.indices.len(), 4);
    }

    /// The one-character shortcut scores exactly like the full alignment.
    #[test]
    fn single_char_shortcut_matches_alignment() {
        let labels = ["readFile", "ReadableStream", "x_read", "a.b-c", "ÉcoleR", "rrr", "URLSearchParams", "", "q9Z"];
        for l in labels {
            for qc in ['r', 'e', 'a', 'z', '9', 'é', '.'] {
                let q = Query::new(&qc.to_string());
                let chars: Vec<char> = l.chars().collect();
                let full = if is_subsequence(&[qc], l) {
                    align_opt(&[qc], &chars, false).map(|(s, _)| s - (chars.len() as i32) / 8)
                } else {
                    None
                };
                assert_eq!(score_label(&q, l), full, "{qc:?} in {l:?}");
            }
        }
    }
}
