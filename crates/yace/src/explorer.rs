//! Explorer model (spec §4.4, §8): one lazily loaded tree per workspace
//! root. Directories are read only when expanded, on worker threads; this
//! module only holds state and produces jobs.

use std::{
    cmp::Ordering,
    collections::{HashMap, HashSet},
    fs,
    path::{Path, PathBuf},
};

use globset::{Glob, GlobSet, GlobSetBuilder};
use yace_workspace::{RootId, RootStatus, Workspace};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EntryKind {
    Dir,
    File,
    SymlinkDir,
    SymlinkFile,
    Broken,
}

impl EntryKind {
    pub fn is_dir(self) -> bool {
        matches!(self, EntryKind::Dir | EntryKind::SymlinkDir)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub name: String,
    pub path: PathBuf,
    pub kind: EntryKind,
}

/// Read one directory level. Runs on worker threads (and inline in tests).
pub fn read_dir_entries(dir: &Path) -> Result<Vec<Entry>, String> {
    let rd = fs::read_dir(dir).map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    for item in rd {
        let Ok(item) = item else { continue };
        let path = item.path();
        let name = item.file_name().to_string_lossy().into_owned();
        let kind = match item.file_type() {
            Ok(ft) if ft.is_symlink() => match fs::metadata(&path) {
                Ok(m) if m.is_dir() => EntryKind::SymlinkDir,
                Ok(_) => EntryKind::SymlinkFile,
                Err(_) => EntryKind::Broken,
            },
            Ok(ft) if ft.is_dir() => EntryKind::Dir,
            Ok(_) => EntryKind::File,
            Err(_) => EntryKind::Broken,
        };
        out.push(Entry { name, path, kind });
    }
    out.sort_by(|a, b| match (a.kind.is_dir(), b.kind.is_dir()) {
        (true, false) => Ordering::Less,
        (false, true) => Ordering::Greater,
        _ => natural_cmp(&a.name, &b.name),
    });
    Ok(out)
}

/// Case-insensitive natural ordering: `file2` < `file10`.
pub fn natural_cmp(a: &str, b: &str) -> Ordering {
    let (mut ai, mut bi) = (a.chars().peekable(), b.chars().peekable());
    loop {
        match (ai.peek().copied(), bi.peek().copied()) {
            (None, None) => return a.cmp(b),
            (None, _) => return Ordering::Less,
            (_, None) => return Ordering::Greater,
            (Some(x), Some(y)) if x.is_ascii_digit() && y.is_ascii_digit() => {
                let mut na = String::new();
                while let Some(c) = ai.peek().copied().filter(char::is_ascii_digit) {
                    na.push(c);
                    ai.next();
                }
                let mut nb = String::new();
                while let Some(c) = bi.peek().copied().filter(char::is_ascii_digit) {
                    nb.push(c);
                    bi.next();
                }
                let (ta, tb) = (na.trim_start_matches('0'), nb.trim_start_matches('0'));
                let o = ta.len().cmp(&tb.len()).then_with(|| ta.cmp(tb));
                if o != Ordering::Equal {
                    return o;
                }
            }
            (Some(x), Some(y)) => {
                let o = x.to_lowercase().cmp(y.to_lowercase());
                if o != Ordering::Equal {
                    return o;
                }
                ai.next();
                bi.next();
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DirState {
    Loading,
    Loaded(Vec<Entry>),
    Error(String),
}

#[derive(Clone, Debug)]
pub struct RootTree {
    pub id: RootId,
    pub path: PathBuf,
    pub expanded: bool,
    pub dirs: HashMap<PathBuf, DirState>,
    /// Expanded sub-directories (absolute paths).
    pub open: HashSet<PathBuf>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RowKind {
    Root {
        missing: Option<String>,
        duplicate: bool,
    },
    Dir,
    File,
    SymlinkDir,
    SymlinkFile,
    Broken,
    /// "loading…" or an error message under a directory.
    Note {
        error: bool,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Row {
    pub root: RootId,
    pub path: PathBuf,
    pub depth: usize,
    pub label: String,
    pub kind: RowKind,
    pub expanded: bool,
}

impl Row {
    pub fn is_dir_like(&self) -> bool {
        matches!(self.kind, RowKind::Root { .. } | RowKind::Dir | RowKind::SymlinkDir)
    }
    pub fn is_file(&self) -> bool {
        matches!(self.kind, RowKind::File | RowKind::SymlinkFile)
    }
}

/// A directory listing request for the worker pool.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReadDirJob {
    pub dir: PathBuf,
}

#[derive(Debug)]
pub struct Explorer {
    pub trees: Vec<RootTree>,
    pub rows: Vec<Row>,
    pub selected: usize,
    pub scroll: usize,
    pub filter: String,
    exclude: GlobSet,
    pub show_hidden: bool,
    /// Path to select once its ancestors have loaded.
    pub pending_reveal: Option<PathBuf>,
    /// Directories currently expanded (for watcher bookkeeping).
    jobs: Vec<ReadDirJob>,
    labels: HashMap<RootId, (String, Option<String>, bool)>,
}

impl Explorer {
    pub fn new(exclude: &[String], show_hidden: bool) -> Self {
        Self {
            trees: Vec::new(),
            rows: Vec::new(),
            selected: 0,
            scroll: 0,
            filter: String::new(),
            exclude: build_globset(exclude),
            show_hidden,
            pending_reveal: None,
            jobs: Vec::new(),
            labels: HashMap::new(),
        }
    }

    pub fn set_excludes(&mut self, exclude: &[String]) {
        self.exclude = build_globset(exclude);
        self.rebuild_rows();
    }

    /// Jobs produced since the last call.
    pub fn take_jobs(&mut self) -> Vec<ReadDirJob> {
        std::mem::take(&mut self.jobs)
    }

    fn request(&mut self, dir: PathBuf) {
        if !self.jobs.iter().any(|j| j.dir == dir) {
            self.jobs.push(ReadDirJob { dir });
        }
    }

    /// Mirror the workspace's roots (order, names, status); keep state for
    /// roots that still exist.
    pub fn sync_roots(&mut self, ws: &Workspace) {
        let mut old: HashMap<RootId, RootTree> = self.trees.drain(..).map(|t| (t.id, t)).collect();
        self.labels.clear();
        for r in ws.roots() {
            let missing = match &r.status {
                RootStatus::Missing(m) => Some(m.clone()),
                _ => None,
            };
            let duplicate = matches!(r.status, RootStatus::Duplicate(_));
            self.labels.insert(r.id, (r.display_name(), missing, duplicate));
            let tree = match old.remove(&r.id) {
                Some(mut t) if t.path == r.path => {
                    if !r.is_available() {
                        t.dirs.clear();
                    }
                    t
                }
                _ => RootTree {
                    id: r.id,
                    path: r.path.clone(),
                    expanded: r.is_available(),
                    dirs: HashMap::new(),
                    open: HashSet::new(),
                },
            };
            self.trees.push(tree);
        }
        let to_load: Vec<PathBuf> = self
            .trees
            .iter()
            .filter(|t| t.expanded && self.root_available(t.id))
            .flat_map(|t| std::iter::once(t.path.clone()).chain(t.open.iter().cloned()))
            .filter(|d| !self.is_loaded(d))
            .collect();
        for d in to_load {
            self.mark_loading(&d);
            self.request(d);
        }
        self.rebuild_rows();
    }

    fn root_available(&self, id: RootId) -> bool {
        self.labels.get(&id).is_some_and(|(_, missing, dup)| missing.is_none() && !dup)
    }

    fn tree_for(&self, path: &Path) -> Option<usize> {
        self.trees
            .iter()
            .enumerate()
            .filter(|(_, t)| path.starts_with(&t.path))
            .max_by_key(|(_, t)| t.path.components().count())
            .map(|(i, _)| i)
    }

    fn is_loaded(&self, dir: &Path) -> bool {
        self.tree_for(dir).and_then(|i| self.trees[i].dirs.get(dir)).is_some_and(|s| !matches!(s, DirState::Error(_)))
    }

    fn mark_loading(&mut self, dir: &Path) {
        if let Some(i) = self.tree_for(dir) {
            self.trees[i].dirs.entry(dir.to_path_buf()).or_insert(DirState::Loading);
        }
    }

    /// All directories currently visible as expanded (for watching).
    pub fn expanded_dirs(&self) -> Vec<PathBuf> {
        let mut out = Vec::new();
        for t in &self.trees {
            if t.expanded && self.root_available(t.id) {
                out.push(t.path.clone());
                out.extend(t.open.iter().filter(|d| self.ancestors_open(t, d)).cloned());
            }
        }
        out
    }

    fn ancestors_open(&self, t: &RootTree, d: &Path) -> bool {
        let mut cur = d.parent();
        while let Some(p) = cur {
            if p == t.path {
                return true;
            }
            if !t.open.contains(p) {
                return false;
            }
            cur = p.parent();
        }
        false
    }

    /// Apply a finished directory read.
    pub fn on_dir_loaded(&mut self, dir: &Path, result: Result<Vec<Entry>, String>) {
        if let Some(i) = self.tree_for(dir) {
            let state = match result {
                Ok(e) => DirState::Loaded(e),
                Err(e) => DirState::Error(e),
            };
            self.trees[i].dirs.insert(dir.to_path_buf(), state);
        }
        let selected = self.selected_key();
        self.rebuild_rows();
        self.restore_selection(selected);
        if let Some(target) = self.pending_reveal.clone()
            && self.select_path(&target)
        {
            self.pending_reveal = None;
        }
    }

    /// Re-read a directory if it is loaded (filesystem event or refresh).
    pub fn refresh_dir(&mut self, dir: &Path) -> bool {
        if self.tree_for(dir).is_some_and(|i| self.trees[i].dirs.contains_key(dir)) {
            self.request(dir.to_path_buf());
            return true;
        }
        false
    }

    /// Re-read every loaded directory.
    pub fn refresh_all(&mut self) {
        let dirs: Vec<PathBuf> = self.trees.iter().flat_map(|t| t.dirs.keys().cloned()).collect();
        for d in dirs {
            self.request(d);
        }
    }

    fn selected_key(&self) -> Option<(RootId, PathBuf)> {
        self.rows.get(self.selected).map(|r| (r.root, r.path.clone()))
    }

    fn restore_selection(&mut self, key: Option<(RootId, PathBuf)>) {
        if let Some((root, path)) = key
            && let Some(i) = self.rows.iter().position(|r| r.root == root && r.path == path)
        {
            self.selected = i;
        }
        self.selected = self.selected.min(self.rows.len().saturating_sub(1));
    }

    pub fn selected_row(&self) -> Option<&Row> {
        self.rows.get(self.selected)
    }

    /// Expand or collapse a directory row.
    pub fn set_expanded(&mut self, row: usize, expanded: bool) {
        let Some(r) = self.rows.get(row).cloned() else { return };
        let Some(ti) = self.trees.iter().position(|t| t.id == r.root) else { return };
        match r.kind {
            RowKind::Root { .. } => {
                if !self.root_available(r.root) {
                    return;
                }
                self.trees[ti].expanded = expanded;
            }
            RowKind::Dir | RowKind::SymlinkDir => {
                if expanded {
                    self.trees[ti].open.insert(r.path.clone());
                } else {
                    self.trees[ti].open.remove(&r.path);
                }
            }
            _ => return,
        }
        if expanded && !self.is_loaded(&r.path) {
            self.mark_loading(&r.path);
            self.request(r.path.clone());
        }
        let key = self.selected_key();
        self.rebuild_rows();
        self.restore_selection(key);
    }

    pub fn toggle(&mut self, row: usize) {
        if let Some(r) = self.rows.get(row) {
            let e = r.expanded;
            self.set_expanded(row, !e);
        }
    }

    /// Expand all ancestors of `path` and select it once visible.
    pub fn reveal(&mut self, path: &Path) {
        let Some(ti) = self.tree_for(path) else { return };
        let root = self.trees[ti].path.clone();
        if !self.root_available(self.trees[ti].id) {
            return;
        }
        self.trees[ti].expanded = true;
        let mut dirs = Vec::new();
        let mut cur = path.parent();
        while let Some(p) = cur {
            if p == root || !p.starts_with(&root) {
                break;
            }
            dirs.push(p.to_path_buf());
            cur = p.parent();
        }
        dirs.push(root.clone());
        for d in dirs {
            if d != root {
                self.trees[ti].open.insert(d.clone());
            }
            if !self.is_loaded(&d) {
                self.mark_loading(&d);
                self.request(d);
            }
        }
        self.rebuild_rows();
        if !self.select_path(path) {
            self.pending_reveal = Some(path.to_path_buf());
        }
    }

    pub fn select_path(&mut self, path: &Path) -> bool {
        if let Some(i) = self.rows.iter().position(|r| r.path == path && !matches!(r.kind, RowKind::Note { .. })) {
            self.selected = i;
            return true;
        }
        false
    }

    fn excluded(&self, root: &Path, entry: &Entry) -> bool {
        if !self.show_hidden && entry.name.starts_with('.') {
            return true;
        }
        let rel = entry.path.strip_prefix(root).unwrap_or(&entry.path);
        self.exclude.is_match(rel) || self.exclude.is_match(Path::new(&entry.name))
    }

    /// Recompute the flat list of visible rows.
    pub fn rebuild_rows(&mut self) {
        let mut rows = Vec::new();
        for t in &self.trees {
            let (label, missing, duplicate) = self.labels.get(&t.id).cloned().unwrap_or_default();
            let available = missing.is_none() && !duplicate;
            rows.push(Row {
                root: t.id,
                path: t.path.clone(),
                depth: 0,
                label,
                kind: RowKind::Root { missing, duplicate },
                expanded: t.expanded && available,
            });
            if t.expanded && available {
                self.push_children(t, &t.path, 1, &mut rows);
            }
        }
        if !self.filter.is_empty() {
            rows = filter_rows(rows, &self.filter);
        }
        self.rows = rows;
        self.selected = self.selected.min(self.rows.len().saturating_sub(1));
    }

    fn push_children(&self, t: &RootTree, dir: &Path, depth: usize, rows: &mut Vec<Row>) {
        match t.dirs.get(dir) {
            None | Some(DirState::Loading) => rows.push(Row {
                root: t.id,
                path: dir.join("\u{0}loading"),
                depth,
                label: "loading…".into(),
                kind: RowKind::Note { error: false },
                expanded: false,
            }),
            Some(DirState::Error(e)) => rows.push(Row {
                root: t.id,
                path: dir.join("\u{0}error"),
                depth,
                label: e.clone(),
                kind: RowKind::Note { error: true },
                expanded: false,
            }),
            Some(DirState::Loaded(entries)) => {
                for e in entries {
                    if self.excluded(&t.path, e) {
                        continue;
                    }
                    let open = e.kind.is_dir() && t.open.contains(&e.path);
                    rows.push(Row {
                        root: t.id,
                        path: e.path.clone(),
                        depth,
                        label: e.name.clone(),
                        kind: match e.kind {
                            EntryKind::Dir => RowKind::Dir,
                            EntryKind::File => RowKind::File,
                            EntryKind::SymlinkDir => RowKind::SymlinkDir,
                            EntryKind::SymlinkFile => RowKind::SymlinkFile,
                            EntryKind::Broken => RowKind::Broken,
                        },
                        expanded: open,
                    });
                    // Guard against symlink cycles: never descend into a
                    // directory whose canonical path is an ancestor.
                    if open && !(e.kind == EntryKind::SymlinkDir && is_cycle(&e.path)) {
                        self.push_children(t, &e.path, depth + 1, rows);
                    }
                }
            }
        }
    }

    /// Session state: expanded dirs relative to each root.
    pub fn expansion_state(&self) -> Vec<(PathBuf, bool, Vec<String>)> {
        self.trees
            .iter()
            .map(|t| {
                let mut rel: Vec<String> = t
                    .open
                    .iter()
                    .filter_map(|p| p.strip_prefix(&t.path).ok().map(|r| r.to_string_lossy().into_owned()))
                    .collect();
                rel.sort();
                (t.path.clone(), t.expanded, rel)
            })
            .collect()
    }

    pub fn apply_expansion(&mut self, root_path: &Path, expanded: bool, rel: &[String]) {
        if let Some(t) = self.trees.iter_mut().find(|t| t.path == root_path) {
            t.expanded = expanded;
            t.open = rel.iter().map(|r| t.path.join(r)).collect();
        }
    }

    pub fn move_selection(&mut self, delta: isize) {
        if self.rows.is_empty() {
            return;
        }
        let n = self.rows.len() as isize;
        self.selected = (self.selected as isize + delta).clamp(0, n - 1) as usize;
    }

    /// Index of the parent row of `row`.
    pub fn parent_row(&self, row: usize) -> Option<usize> {
        let depth = self.rows.get(row)?.depth;
        if depth == 0 {
            return None;
        }
        (0..row).rev().find(|i| self.rows[*i].depth < depth)
    }

    /// Directory where a new file should go for the current selection.
    pub fn target_dir(&self) -> Option<(RootId, PathBuf)> {
        let r = self.selected_row()?;
        match r.kind {
            RowKind::Root { ref missing, duplicate } if missing.is_none() && !duplicate => {
                Some((r.root, r.path.clone()))
            }
            RowKind::Dir | RowKind::SymlinkDir => Some((r.root, r.path.clone())),
            RowKind::Root { .. } => None,
            _ => Some((r.root, r.path.parent()?.to_path_buf())),
        }
    }
}

fn is_cycle(path: &Path) -> bool {
    let Ok(target) = fs::canonicalize(path) else { return true };
    let parent = path.parent().and_then(|p| fs::canonicalize(p).ok());
    parent.is_some_and(|p| p.starts_with(&target))
}

fn build_globset(patterns: &[String]) -> GlobSet {
    let mut b = GlobSetBuilder::new();
    for p in patterns {
        if let Ok(g) = Glob::new(p) {
            b.add(g);
        }
    }
    b.build().unwrap_or_else(|_| GlobSet::empty())
}

/// Keep rows matching `filter` plus their ancestors and all roots.
fn filter_rows(rows: Vec<Row>, filter: &str) -> Vec<Row> {
    let f = filter.to_lowercase();
    let mut keep = vec![false; rows.len()];
    let mut want = usize::MAX;
    for i in (0..rows.len()).rev() {
        let r = &rows[i];
        if r.depth == 0 {
            keep[i] = true;
            want = usize::MAX;
            continue;
        }
        if matches!(r.kind, RowKind::Note { .. }) {
            continue;
        }
        let ancestor_of_kept = r.depth < want && want != usize::MAX;
        if ancestor_of_kept || r.label.to_lowercase().contains(&f) {
            keep[i] = true;
            want = r.depth;
        }
    }
    rows.into_iter().zip(keep).filter(|(_, k)| *k).map(|(r, _)| r).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run_jobs(ex: &mut Explorer) {
        loop {
            let jobs = ex.take_jobs();
            if jobs.is_empty() {
                break;
            }
            for j in jobs {
                let res = read_dir_entries(&j.dir);
                ex.on_dir_loaded(&j.dir, res);
            }
        }
    }

    fn setup() -> (tempfile::TempDir, Workspace) {
        let d = tempfile::tempdir().unwrap();
        let p = d.path();
        fs::create_dir_all(p.join("src/nested")).unwrap();
        fs::create_dir_all(p.join(".git")).unwrap();
        fs::write(p.join("src/main.rs"), "").unwrap();
        fs::write(p.join("src/nested/deep.rs"), "").unwrap();
        fs::write(p.join("file10.txt"), "").unwrap();
        fs::write(p.join("file2.txt"), "").unwrap();
        fs::write(p.join(".env"), "").unwrap();
        let ws = Workspace::from_folder(p);
        (d, ws)
    }

    fn labels(ex: &Explorer) -> Vec<String> {
        ex.rows.iter().map(|r| format!("{}{}", "  ".repeat(r.depth), r.label)).collect()
    }

    #[test]
    fn lazy_loading_sorting_and_excludes() {
        let (_d, ws) = setup();
        let mut ex = Explorer::new(&["**/.git".to_string()], true);
        ex.sync_roots(&ws);
        assert_eq!(ex.rows.len(), 2, "root + loading placeholder");
        assert!(matches!(ex.rows[1].kind, RowKind::Note { .. }));
        run_jobs(&mut ex);
        let l = labels(&ex);
        assert_eq!(&l[1..], &["  src", "  .env", "  file2.txt", "  file10.txt"]);
        ex.set_expanded(1, true);
        assert_eq!(ex.take_jobs().len(), 1, "only the expanded dir is read");
        ex.on_dir_loaded(&ex.rows[1].path.clone(), read_dir_entries(&ex.rows[1].path.clone()));
        assert!(labels(&ex).contains(&"    main.rs".to_string()));
        ex.show_hidden = false;
        ex.rebuild_rows();
        assert!(!labels(&ex).contains(&"  .env".to_string()));
    }

    #[test]
    fn reveal_expands_ancestors_and_selects() {
        let (d, ws) = setup();
        let mut ex = Explorer::new(&[], true);
        ex.sync_roots(&ws);
        run_jobs(&mut ex);
        let target = d.path().join("src/nested/deep.rs");
        ex.reveal(&target);
        run_jobs(&mut ex);
        assert_eq!(ex.selected_row().unwrap().path, target);
        let state = ex.expansion_state();
        assert_eq!(state[0].2, vec!["src".to_string(), "src/nested".to_string()]);
    }

    #[test]
    fn filter_keeps_ancestors() {
        let (d, ws) = setup();
        let mut ex = Explorer::new(&[], true);
        ex.sync_roots(&ws);
        run_jobs(&mut ex);
        ex.reveal(&d.path().join("src/nested/deep.rs"));
        run_jobs(&mut ex);
        ex.filter = "deep".into();
        ex.rebuild_rows();
        let l = labels(&ex);
        assert_eq!(l.len(), 4, "{l:?}");
        assert!(l[3].ends_with("deep.rs"));
    }

    #[test]
    fn missing_root_does_not_block_others() {
        let (d, _) = setup();
        let mut ws = Workspace::from_folder(d.path());
        ws.add_root(Path::new("/definitely/missing/root"), Some("Gone".into()));
        let mut ex = Explorer::new(&[], true);
        ex.sync_roots(&ws);
        run_jobs(&mut ex);
        let gone = ex.rows.iter().find(|r| r.label == "Gone").unwrap();
        assert!(matches!(&gone.kind, RowKind::Root { missing: Some(_), .. }));
        assert!(ex.rows.iter().any(|r| r.label == "src"));
    }

    #[test]
    fn natural_ordering() {
        let mut v = vec!["b10", "B2", "a", "b1"];
        v.sort_by(|a, b| natural_cmp(a, b));
        assert_eq!(v, vec!["a", "b1", "B2", "b10"]);
    }

    #[cfg(unix)]
    #[test]
    fn symlink_cycles_are_not_followed() {
        let d = tempfile::tempdir().unwrap();
        fs::create_dir_all(d.path().join("a")).unwrap();
        std::os::unix::fs::symlink(d.path(), d.path().join("a/loop")).unwrap();
        let ws = Workspace::from_folder(d.path());
        let mut ex = Explorer::new(&[], true);
        ex.sync_roots(&ws);
        run_jobs(&mut ex);
        ex.set_expanded(1, true);
        run_jobs(&mut ex);
        let loop_row = ex.rows.iter().position(|r| r.label == "loop").unwrap();
        assert_eq!(ex.rows[loop_row].kind, RowKind::SymlinkDir);
        ex.set_expanded(loop_row, true);
        run_jobs(&mut ex);
        assert!(ex.rows.len() < 20, "cycle not expanded recursively");
    }
}
