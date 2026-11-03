//! Workspace file discovery (spec §6.5, §6.6, §14.2): a parallel,
//! cancellable, bounded walk of one root that honours `.gitignore`,
//! `.ignore`, global git excludes and YACE's exclude globs.

use std::{
    path::Path,
    sync::{
        Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
};

use globset::{Glob, GlobSet, GlobSetBuilder};
use ignore::{WalkBuilder, WalkState};

/// How to walk a root.
#[derive(Clone, Debug)]
pub struct WalkOptions {
    /// Honour `.gitignore` / `.ignore` / global excludes.
    pub use_ignore_files: bool,
    /// Include dot-files and dot-directories.
    pub include_hidden: bool,
    /// Glob patterns (relative to the root) that are skipped entirely.
    pub exclude: Vec<String>,
    /// Stop after this many files.
    pub max_files: usize,
}

impl Default for WalkOptions {
    fn default() -> Self {
        Self { use_ignore_files: true, include_hidden: true, exclude: vec!["**/.git".into()], max_files: 500_000 }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct WalkSummary {
    pub files: usize,
    pub truncated: bool,
    pub cancelled: bool,
}

/// Normalise VS Code-style globs: patterns without `/` match at any depth,
/// and every pattern also matches everything below a matching directory.
pub fn normalize_globs(patterns: &[String]) -> Vec<String> {
    let mut out = Vec::new();
    for p in patterns {
        let p = p.trim().trim_start_matches("./");
        if p.is_empty() {
            continue;
        }
        let base = if p.contains('/') { p.to_string() } else { format!("**/{p}") };
        let base = base.trim_end_matches('/').to_string();
        out.push(format!("{base}/**"));
        out.push(base);
    }
    out
}

pub fn build_globset(patterns: &[String]) -> GlobSet {
    let mut b = GlobSetBuilder::new();
    for p in normalize_globs(patterns) {
        if let Ok(g) = Glob::new(&p) {
            b.add(g);
        }
    }
    b.build().unwrap_or_else(|_| GlobSet::empty())
}

pub(crate) fn builder(root: &Path, opts: &WalkOptions) -> WalkBuilder {
    let mut b = WalkBuilder::new(root);
    b.hidden(!opts.include_hidden)
        .ignore(opts.use_ignore_files)
        .git_ignore(opts.use_ignore_files)
        .git_global(opts.use_ignore_files)
        .git_exclude(opts.use_ignore_files)
        .parents(opts.use_ignore_files)
        .require_git(false)
        .follow_links(false)
        .threads(std::thread::available_parallelism().map(|n| n.get().min(4)).unwrap_or(2));
    let excludes = build_globset(&opts.exclude);
    let root = root.to_path_buf();
    b.filter_entry(move |e| {
        let rel = e.path().strip_prefix(&root).unwrap_or(e.path());
        rel.as_os_str().is_empty() || !excludes.is_match(rel)
    });
    b
}

const BATCH: usize = 512;

/// Walk `root`, delivering relative paths (with `/` separators) in batches.
pub fn walk_root(
    root: &Path,
    opts: &WalkOptions,
    cancel: &AtomicBool,
    on_batch: &(dyn Fn(Vec<Box<str>>) + Sync),
) -> WalkSummary {
    let count = AtomicUsize::new(0);
    let truncated = AtomicBool::new(false);
    let walker = builder(root, opts).build_parallel();
    let root_buf = root.to_path_buf();

    struct Flusher<'a> {
        buf: Vec<Box<str>>,
        sink: &'a (dyn Fn(Vec<Box<str>>) + Sync),
    }
    impl Drop for Flusher<'_> {
        fn drop(&mut self) {
            if !self.buf.is_empty() {
                (self.sink)(std::mem::take(&mut self.buf));
            }
        }
    }

    walker.run(|| {
        let mut flusher = Flusher { buf: Vec::with_capacity(BATCH), sink: on_batch };
        let root = root_buf.clone();
        let count = &count;
        let truncated = &truncated;
        Box::new(move |entry| {
            if cancel.load(Ordering::Relaxed) {
                return WalkState::Quit;
            }
            let Ok(e) = entry else { return WalkState::Continue };
            if !e.file_type().is_some_and(|t| t.is_file() || t.is_symlink()) {
                return WalkState::Continue;
            }
            if e.file_type().is_some_and(|t| t.is_symlink())
                && std::fs::metadata(e.path()).map(|m| m.is_dir()).unwrap_or(true)
            {
                return WalkState::Continue;
            }
            if count.fetch_add(1, Ordering::Relaxed) >= opts.max_files {
                truncated.store(true, Ordering::Relaxed);
                return WalkState::Quit;
            }
            let rel = e.path().strip_prefix(&root).unwrap_or(e.path());
            let s = rel.to_string_lossy().replace('\\', "/");
            flusher.buf.push(s.into_boxed_str());
            if flusher.buf.len() >= BATCH {
                (flusher.sink)(std::mem::take(&mut flusher.buf));
            }
            WalkState::Continue
        })
    });
    WalkSummary {
        files: count.load(Ordering::Relaxed).min(opts.max_files),
        truncated: truncated.load(Ordering::Relaxed),
        cancelled: cancel.load(Ordering::Relaxed),
    }
}

/// Convenience: collect every file (tests, small roots).
pub fn collect(root: &Path, opts: &WalkOptions) -> (Vec<Box<str>>, WalkSummary) {
    let out = Mutex::new(Vec::new());
    let s = walk_root(root, opts, &AtomicBool::new(false), &|b| out.lock().unwrap().extend(b));
    let mut v = out.into_inner().unwrap();
    v.sort();
    (v, s)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn tree() -> tempfile::TempDir {
        let d = tempfile::tempdir().unwrap();
        let p = d.path();
        for f in [
            "src/main.rs",
            "src/lib.rs",
            "target/debug/out.bin",
            "node_modules/x/index.js",
            ".env",
            ".git/config",
            "docs/a b.md",
        ] {
            fs::create_dir_all(p.join(f).parent().unwrap()).unwrap();
            fs::write(p.join(f), "x").unwrap();
        }
        fs::write(p.join(".gitignore"), "target/\n").unwrap();
        d
    }

    #[test]
    fn respects_gitignore_and_excludes() {
        let d = tree();
        let opts = WalkOptions { exclude: vec!["**/.git".into(), "node_modules".into()], ..Default::default() };
        let (files, s) = collect(d.path(), &opts);
        let names: Vec<&str> = files.iter().map(|b| &**b).collect();
        assert_eq!(names, vec![".env", ".gitignore", "docs/a b.md", "src/lib.rs", "src/main.rs"]);
        assert!(!s.truncated);
        let all = WalkOptions { use_ignore_files: false, exclude: vec![], include_hidden: true, ..Default::default() };
        let (files, _) = collect(d.path(), &all);
        assert!(files.iter().any(|f| &**f == "target/debug/out.bin"));
        assert!(files.iter().any(|f| &**f == ".git/config"));
    }

    #[test]
    fn cap_and_cancel() {
        let d = tree();
        let opts = WalkOptions { max_files: 2, exclude: vec![], ..Default::default() };
        let (files, s) = collect(d.path(), &opts);
        assert!(s.truncated);
        assert!(files.len() <= 2);
        let cancel = AtomicBool::new(true);
        let s = walk_root(d.path(), &WalkOptions::default(), &cancel, &|_| {});
        assert!(s.cancelled);
    }

    #[test]
    fn glob_normalisation() {
        let g = build_globset(&["dist".into(), "src/gen/".into(), "*.min.js".into()]);
        assert!(g.is_match("dist"));
        assert!(g.is_match("apps/web/dist/x.js"));
        assert!(g.is_match("src/gen/a.rs"));
        assert!(g.is_match("lib/a.min.js"));
        assert!(!g.is_match("src/main.rs"));
    }
}
