//! Workspace content search (spec §4.6, §14): streamed, cancellable,
//! bounded, ignore-aware search across roots, built on ripgrep's crates.
//! Dirty open documents are searched from memory ("overlays") so results
//! match what the user sees.

use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
};

use globset::GlobSet;
use grep_matcher::Matcher;
use grep_regex::RegexMatcherBuilder;
use grep_searcher::{BinaryDetection, SearcherBuilder, sinks::Lossy};
use ignore::WalkState;

use crate::{
    find::FindOptions,
    index::RootKey,
    walk::{WalkOptions, build_globset},
};

#[derive(Clone, Debug)]
pub struct SearchRoot {
    pub key: RootKey,
    pub name: String,
    pub path: PathBuf,
}

#[derive(Clone, Debug)]
pub struct ContentQuery {
    pub pattern: String,
    pub opts: FindOptions,
    /// "files to include": globs; `./<Root Name>/…` scopes to a root.
    pub include: Vec<String>,
    /// "files to exclude" (added to `walk.exclude`).
    pub exclude: Vec<String>,
    pub walk: WalkOptions,
    pub max_results: usize,
    pub max_per_file: usize,
}

impl Default for ContentQuery {
    fn default() -> Self {
        Self {
            pattern: String::new(),
            opts: FindOptions::default(),
            include: vec![],
            exclude: vec![],
            walk: WalkOptions::default(),
            max_results: 20_000,
            max_per_file: 1_000,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LineMatch {
    /// 0-based line number.
    pub line: usize,
    /// Line text (without line break), capped for display.
    pub text: String,
    /// Char column ranges of matches within `text`.
    pub ranges: Vec<(usize, usize)>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileMatches {
    pub root: RootKey,
    pub rel: String,
    pub path: PathBuf,
    pub matches: Vec<LineMatch>,
    /// Hash of the searched content (for safe replace).
    pub hash: u64,
    pub from_buffer: bool,
    pub truncated: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SearchSummary {
    pub files: usize,
    pub matches: usize,
    pub truncated: bool,
    pub cancelled: bool,
}

const MAX_LINE_CHARS: usize = 1_000;
const MAX_FILE_BYTES: u64 = 64 * 1024 * 1024;

/// Per-root include rules derived from `./<Root Name>/pattern` entries.
#[derive(Debug, Clone)]
pub struct Scope {
    /// Roots to search (all when no scoped pattern applies).
    pub roots: Vec<RootKey>,
    /// Include globs per root (empty = everything).
    pub include: HashMap<RootKey, Vec<String>>,
}

pub fn scope(roots: &[SearchRoot], include: &[String]) -> Scope {
    let mut global = Vec::new();
    let mut scoped: HashMap<RootKey, Vec<String>> = HashMap::new();
    let mut any_scoped = false;
    for raw in include.iter().map(|s| s.trim()).filter(|s| !s.is_empty()) {
        let p = raw.trim_start_matches("./");
        let hit = roots.iter().find(|r| p == r.name || p.starts_with(&format!("{}/", r.name)));
        match hit {
            Some(r) => {
                any_scoped = true;
                let rest = p[r.name.len()..].trim_start_matches('/');
                let e = scoped.entry(r.key).or_default();
                if !rest.is_empty() {
                    e.push(rest.to_string());
                }
            }
            None => global.push(p.to_string()),
        }
    }
    let selected: Vec<RootKey> = roots
        .iter()
        .filter(|r| !any_scoped || scoped.contains_key(&r.key) || !global.is_empty())
        .map(|r| r.key)
        .collect();
    let include = roots
        .iter()
        .map(|r| {
            let mut v = global.clone();
            if let Some(s) = scoped.get(&r.key) {
                v.extend(s.iter().cloned());
            }
            (r.key, v)
        })
        .collect();
    Scope { roots: selected, include }
}

fn char_ranges(line: &str, byte_ranges: &[(usize, usize)]) -> Vec<(usize, usize)> {
    byte_ranges
        .iter()
        .filter_map(|(a, b)| {
            let a = line.get(..*a)?.chars().count();
            let len = line.get(..*b)?.chars().count();
            (a < MAX_LINE_CHARS).then_some((a, len.min(MAX_LINE_CHARS)))
        })
        .collect()
}

/// Search one in-memory buffer.
fn search_bytes(matcher: &grep_regex::RegexMatcher, bytes: &[u8], max_per_file: usize) -> (Vec<LineMatch>, bool) {
    let mut out = Vec::new();
    let mut truncated = false;
    let mut searcher =
        SearcherBuilder::new().binary_detection(BinaryDetection::quit(b'\x00')).line_number(true).build();
    let _ = searcher.search_slice(
        matcher,
        bytes,
        Lossy(|lnum, line| {
            let text = line.trim_end_matches(['\n', '\r']);
            let mut ranges = Vec::new();
            let _ = matcher.find_iter(text.as_bytes(), |m| {
                if m.start() != m.end() {
                    ranges.push((m.start(), m.end()));
                }
                true
            });
            if ranges.is_empty() {
                return Ok(true);
            }
            let shown: String = text.chars().take(MAX_LINE_CHARS).collect();
            out.push(LineMatch { line: lnum as usize - 1, ranges: char_ranges(text, &ranges), text: shown });
            if out.len() >= max_per_file {
                truncated = true;
                return Ok(false);
            }
            Ok(true)
        }),
    );
    (out, truncated)
}

pub fn build_matcher(q: &ContentQuery) -> Result<grep_regex::RegexMatcher, String> {
    if q.pattern.is_empty() {
        return Err("empty query".into());
    }
    RegexMatcherBuilder::new()
        .case_insensitive(!q.opts.case_sensitive)
        .word(q.opts.whole_word)
        .fixed_strings(!q.opts.regex)
        .multi_line(false)
        .build(&q.pattern)
        .map_err(|e| e.to_string().lines().last().unwrap_or("invalid pattern").trim().to_string())
}

/// Run a search. `overlays` maps absolute paths of dirty open documents
/// to their current text. Results are delivered per file via `on_file`.
pub fn search(
    roots: &[SearchRoot],
    q: &ContentQuery,
    overlays: &HashMap<PathBuf, Arc<str>>,
    cancel: &AtomicBool,
    on_file: &(dyn Fn(FileMatches) + Sync),
) -> Result<SearchSummary, String> {
    let matcher = build_matcher(q)?;
    let sc = scope(roots, &q.include);
    let total = AtomicUsize::new(0);
    let files = AtomicUsize::new(0);
    let truncated = AtomicBool::new(false);
    let mut walk = q.walk.clone();
    walk.exclude.extend(q.exclude.iter().cloned());
    for r in roots.iter().filter(|r| sc.roots.contains(&r.key)) {
        if cancel.load(Ordering::Relaxed) || truncated.load(Ordering::Relaxed) {
            break;
        }
        let include: Option<GlobSet> = sc.include.get(&r.key).filter(|v| !v.is_empty()).map(|v| build_globset(v));
        let walker = crate::walk::builder(&r.path, &walk).build_parallel();
        walker.run(|| {
            let matcher = matcher.clone();
            let include = include.clone();
            let (total, files, truncated) = (&total, &files, &truncated);
            let root = r;
            Box::new(move |entry| {
                if cancel.load(Ordering::Relaxed) || truncated.load(Ordering::Relaxed) {
                    return WalkState::Quit;
                }
                let Ok(e) = entry else { return WalkState::Continue };
                if !e.file_type().is_some_and(|t| t.is_file()) {
                    return WalkState::Continue;
                }
                let rel_path = e.path().strip_prefix(&root.path).unwrap_or(e.path());
                if include.as_ref().is_some_and(|g| !g.is_match(rel_path)) {
                    return WalkState::Continue;
                }
                let path = e.path().to_path_buf();
                let (bytes, from_buffer): (std::borrow::Cow<[u8]>, bool) = match overlays.get(&path) {
                    Some(t) => (std::borrow::Cow::Owned(t.as_bytes().to_vec()), true),
                    None => {
                        if e.metadata().map(|m| m.len() > MAX_FILE_BYTES).unwrap_or(true) {
                            return WalkState::Continue;
                        }
                        match std::fs::read(&path) {
                            Ok(b) => (std::borrow::Cow::Owned(b), false),
                            Err(_) => return WalkState::Continue,
                        }
                    }
                };
                let (matches, file_trunc) = search_bytes(&matcher, &bytes, q.max_per_file);
                if matches.is_empty() {
                    return WalkState::Continue;
                }
                let n: usize = matches.iter().map(|m| m.ranges.len().max(1)).sum();
                let before = total.fetch_add(n, Ordering::Relaxed);
                if before >= q.max_results {
                    truncated.store(true, Ordering::Relaxed);
                    return WalkState::Quit;
                }
                files.fetch_add(1, Ordering::Relaxed);
                on_file(FileMatches {
                    root: root.key,
                    rel: rel_path.to_string_lossy().replace('\\', "/"),
                    path,
                    hash: yace_buffer::io::hash_bytes(&bytes),
                    matches,
                    from_buffer,
                    truncated: file_trunc,
                });
                WalkState::Continue
            })
        });
    }
    Ok(SearchSummary {
        files: files.load(Ordering::Relaxed),
        matches: total.load(Ordering::Relaxed).min(q.max_results),
        truncated: truncated.load(Ordering::Relaxed),
        cancelled: cancel.load(Ordering::Relaxed),
    })
}

/// Replace matches in a file on disk. Refuses if the file changed since
/// it was searched (`expected_hash`). `skip` holds `(line, char_col)` of
/// matches the user excluded. Returns the number of replacements.
pub fn replace_in_file(
    path: &std::path::Path,
    re: &regex::Regex,
    replacement: &str,
    regex_mode: bool,
    expected_hash: u64,
    skip: &std::collections::HashSet<(usize, usize)>,
) -> Result<usize, String> {
    let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
    if yace_buffer::io::hash_bytes(&bytes) != expected_hash {
        return Err("changed since the search; search again".into());
    }
    let text = std::str::from_utf8(&bytes).map_err(|_| "not valid UTF-8".to_string())?;
    let reps = crate::find::replacements(text, re, replacement, regex_mode);
    let line_starts: Vec<usize> =
        std::iter::once(0).chain(text.char_indices().filter(|(_, c)| *c == '\n').map(|(i, _)| i + 1)).collect();
    // Char offset → (line, col): walk alongside.
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars();
    let mut pos = 0usize;
    let mut count = 0;
    let char_line_starts: Vec<usize> = {
        let mut v = Vec::with_capacity(line_starts.len());
        let mut c = 0;
        let mut b = 0;
        for &ls in &line_starts {
            c += text[b..ls].chars().count();
            b = ls;
            v.push(c);
        }
        v
    };
    for (from, to, rep) in reps {
        let line = char_line_starts.partition_point(|s| *s <= from) - 1;
        let col = from - char_line_starts[line];
        if skip.contains(&(line, col)) {
            continue;
        }
        out.extend(chars.by_ref().take(from - pos));
        for _ in from..to {
            chars.next();
        }
        out.push_str(&rep);
        pos = to;
        count += 1;
    }
    out.extend(chars);
    if count > 0 {
        yace_buffer::io::write_safely(path, out.as_bytes()).map_err(|e| e.to_string())?;
    }
    Ok(count)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{collections::HashSet, fs, sync::Mutex};

    fn setup() -> (tempfile::TempDir, tempfile::TempDir, Vec<SearchRoot>) {
        let a = tempfile::tempdir().unwrap();
        let b = tempfile::tempdir().unwrap();
        fs::create_dir_all(a.path().join("src")).unwrap();
        fs::write(a.path().join("src/main.rs"), "fn main() {\n    let todo = 1; // TODO fix\n}\n").unwrap();
        fs::write(a.path().join("README.md"), "todo list\n").unwrap();
        fs::write(a.path().join("bin.dat"), b"todo\0\x01\x02").unwrap();
        fs::write(a.path().join(".gitignore"), "ignored.txt\n").unwrap();
        fs::write(a.path().join("ignored.txt"), "todo\n").unwrap();
        fs::write(b.path().join("app.ts"), "// todo: web\nconst todoList = [];\n").unwrap();
        let roots = vec![
            SearchRoot { key: 1, name: "API".into(), path: a.path().to_path_buf() },
            SearchRoot { key: 2, name: "Web App".into(), path: b.path().to_path_buf() },
        ];
        (a, b, roots)
    }

    fn run(
        roots: &[SearchRoot],
        q: &ContentQuery,
        overlays: &HashMap<PathBuf, Arc<str>>,
    ) -> (Vec<FileMatches>, SearchSummary) {
        let out = Mutex::new(Vec::new());
        let s = search(roots, q, overlays, &AtomicBool::new(false), &|f| out.lock().unwrap().push(f)).unwrap();
        let mut v = out.into_inner().unwrap();
        v.sort_by(|a, b| (a.root, &a.rel).cmp(&(b.root, &b.rel)));
        (v, s)
    }

    #[test]
    fn searches_all_roots_respecting_ignores_and_binaries() {
        let (_a, _b, roots) = setup();
        let q = ContentQuery { pattern: "todo".into(), ..Default::default() };
        let (files, s) = run(&roots, &q, &HashMap::new());
        let rels: Vec<(u32, &str)> = files.iter().map(|f| (f.root, f.rel.as_str())).collect();
        assert_eq!(rels, vec![(1, "README.md"), (1, "src/main.rs"), (2, "app.ts")]);
        assert_eq!(s.files, 3);
        let main = &files[1];
        assert_eq!(main.matches[0].line, 1);
        assert_eq!(main.matches[0].ranges, vec![(8, 12), (21, 25)]);
    }

    #[test]
    fn case_word_regex_and_globs() {
        let (_a, _b, roots) = setup();
        let case = ContentQuery {
            pattern: "TODO".into(),
            opts: FindOptions { case_sensitive: true, ..Default::default() },
            ..Default::default()
        };
        assert_eq!(run(&roots, &case, &HashMap::new()).1.matches, 1);
        let word = ContentQuery {
            pattern: "todo".into(),
            opts: FindOptions { whole_word: true, ..Default::default() },
            ..Default::default()
        };
        let (f, _) = run(&roots, &word, &HashMap::new());
        assert!(f.iter().all(|f| f.matches.iter().all(|m| !m.text.contains("todoList") || m.ranges.len() == 1)));
        let re = ContentQuery {
            pattern: r"todo\w+".into(),
            opts: FindOptions { regex: true, ..Default::default() },
            ..Default::default()
        };
        assert_eq!(run(&roots, &re, &HashMap::new()).1.matches, 1);
        let inc = ContentQuery { pattern: "todo".into(), include: vec!["*.rs".into()], ..Default::default() };
        assert_eq!(run(&roots, &inc, &HashMap::new()).0.len(), 1);
        let exc = ContentQuery { pattern: "todo".into(), exclude: vec!["src".into()], ..Default::default() };
        assert_eq!(run(&roots, &exc, &HashMap::new()).0.len(), 2);
    }

    #[test]
    fn root_scoping() {
        let (_a, _b, roots) = setup();
        let q = ContentQuery { pattern: "todo".into(), include: vec!["./Web App".into()], ..Default::default() };
        let (f, _) = run(&roots, &q, &HashMap::new());
        assert!(f.iter().all(|f| f.root == 2) && !f.is_empty());
        let q = ContentQuery { pattern: "todo".into(), include: vec!["./API/src".into()], ..Default::default() };
        let (f, _) = run(&roots, &q, &HashMap::new());
        assert_eq!(f.iter().map(|f| f.rel.as_str()).collect::<Vec<_>>(), vec!["src/main.rs"]);
    }

    #[test]
    fn overlays_caps_and_cancel() {
        let (a, _b, roots) = setup();
        let mut ov = HashMap::new();
        ov.insert(a.path().join("README.md"), Arc::<str>::from("nothing here\nstill todo todo\n"));
        let q = ContentQuery { pattern: "todo".into(), ..Default::default() };
        let (f, _) = run(&roots, &q, &ov);
        let readme = f.iter().find(|f| f.rel == "README.md").unwrap();
        assert!(readme.from_buffer);
        assert_eq!(readme.matches[0].line, 1);
        let capped = ContentQuery { pattern: "todo".into(), max_results: 1, ..Default::default() };
        assert!(run(&roots, &capped, &HashMap::new()).1.truncated);
        let s = search(&roots, &q, &HashMap::new(), &AtomicBool::new(true), &|_| {}).unwrap();
        assert!(s.cancelled);
        assert!(search(&roots, &ContentQuery::default(), &HashMap::new(), &AtomicBool::new(false), &|_| {}).is_err());
    }

    #[test]
    fn replace_on_disk_with_skip_and_stale_check() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("f.txt");
        fs::write(&p, "a1 a2\na3\n").unwrap();
        let h = yace_buffer::io::hash_bytes(&fs::read(&p).unwrap());
        let re = crate::find::build_regex(r"a(\d)", FindOptions { regex: true, ..Default::default() }).unwrap();
        let skip: HashSet<(usize, usize)> = [(0, 3)].into_iter().collect();
        assert_eq!(replace_in_file(&p, &re, "b$1", true, h, &skip).unwrap(), 2);
        assert_eq!(fs::read_to_string(&p).unwrap(), "b1 a2\nb3\n");
        assert!(replace_in_file(&p, &re, "x", true, h, &HashSet::new()).is_err(), "stale hash refused");
    }
}
