//! The workspace file-name index behind Quick Open (spec §6.5, §23.4).
//!
//! Per root: a flat list of relative paths, filled progressively by the
//! walker. Queries are bounded (top-K) and ranked with [`crate::fuzzy`]
//! plus context: recently opened / picked files and the active root.
//! Lists can be cached on disk for warm starts; cached data is marked
//! stale and always replaced by a fresh walk.

use std::{
    cmp::Ordering,
    collections::{BinaryHeap, HashMap},
    io,
    path::{Path, PathBuf},
    time::Instant,
};

use crate::fuzzy::{Query, match_path_in_root, root_fallbacks};

pub type RootKey = u32;

/// Files per parallel scoring chunk.
const PAR_CHUNK: usize = 16_384;

#[derive(Debug, Clone)]
pub struct RootIndex {
    pub root: RootKey,
    pub name: String,
    pub path: PathBuf,
    pub files: Vec<Box<str>>,
    /// The last walk finished.
    pub complete: bool,
    /// Loaded from cache or pending refresh.
    pub stale: bool,
    pub truncated: bool,
    pub updated: Option<Instant>,
    /// Paths received by an in-progress re-walk (swapped in when done).
    pending: Option<Vec<Box<str>>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hit {
    pub root: RootKey,
    pub rel: Box<str>,
    pub score: i32,
    /// Char indices into `rel` to highlight.
    pub indices: Vec<usize>,
}

/// Ranking context.
#[derive(Debug, Clone, Default)]
pub struct QueryContext {
    /// Recently opened files, most recent first.
    pub recent: Vec<(RootKey, String)>,
    /// Recently chosen Quick Open results, most recent first.
    pub picked: Vec<(RootKey, String)>,
    pub active_root: Option<RootKey>,
}

#[derive(Debug, Default)]
pub struct FileIndex {
    roots: Vec<RootIndex>,
}

/// Heap entry ordered so the *worst* hit is on top (min-heap of the best K).
struct Ranked(Hit, u8);
impl PartialEq for Ranked {
    fn eq(&self, o: &Self) -> bool {
        self.cmp(o) == Ordering::Equal
    }
}
impl Eq for Ranked {}
impl PartialOrd for Ranked {
    fn partial_cmp(&self, o: &Self) -> Option<Ordering> {
        Some(self.cmp(o))
    }
}
impl Ord for Ranked {
    fn cmp(&self, o: &Self) -> Ordering {
        // "Greater" = worse, so BinaryHeap::peek is the worst kept hit.
        o.0.score
            .cmp(&self.0.score)
            .then(self.0.rel.len().cmp(&o.0.rel.len()))
            .then(self.0.rel.cmp(&o.0.rel))
            .then(self.0.root.cmp(&o.0.root))
            .then(self.1.cmp(&o.1))
    }
}

impl FileIndex {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn roots(&self) -> &[RootIndex] {
        &self.roots
    }

    pub fn root(&self, key: RootKey) -> Option<&RootIndex> {
        self.roots.iter().find(|r| r.root == key)
    }

    fn root_mut(&mut self, key: RootKey) -> Option<&mut RootIndex> {
        self.roots.iter_mut().find(|r| r.root == key)
    }

    /// Register (or update) a root; keeps existing data.
    pub fn ensure_root(&mut self, key: RootKey, name: &str, path: &Path) {
        if let Some(r) = self.root_mut(key) {
            r.name = name.to_string();
            if r.path != path {
                r.path = path.to_path_buf();
                r.files.clear();
                r.complete = false;
            }
            return;
        }
        self.roots.push(RootIndex {
            root: key,
            name: name.to_string(),
            path: path.to_path_buf(),
            files: Vec::new(),
            complete: false,
            stale: false,
            truncated: false,
            updated: None,
            pending: None,
        });
    }

    /// Keep only these roots (in this order).
    pub fn retain_roots(&mut self, keys: &[RootKey]) {
        self.roots.retain(|r| keys.contains(&r.root));
        self.roots.sort_by_key(|r| keys.iter().position(|k| *k == r.root));
    }

    /// A walk is starting. For a root that already has data, results are
    /// collected aside and swapped in on completion (no flicker).
    pub fn begin_walk(&mut self, key: RootKey) {
        if let Some(r) = self.root_mut(key) {
            if r.files.is_empty() {
                r.pending = None;
            } else {
                r.pending = Some(Vec::new());
            }
            r.complete = false;
        }
    }

    pub fn add_batch(&mut self, key: RootKey, batch: Vec<Box<str>>) {
        if let Some(r) = self.root_mut(key) {
            match &mut r.pending {
                Some(p) => p.extend(batch),
                None => r.files.extend(batch),
            }
        }
    }

    pub fn finish_walk(&mut self, key: RootKey, truncated: bool) {
        if let Some(r) = self.root_mut(key) {
            if let Some(p) = r.pending.take() {
                r.files = p;
            }
            r.files.sort_unstable();
            r.files.dedup();
            r.complete = true;
            r.stale = false;
            r.truncated = truncated;
            r.updated = Some(Instant::now());
        }
    }

    /// Patch from filesystem events: add created files, drop removed ones.
    pub fn apply_change(&mut self, key: RootKey, rel: &str, exists: bool) {
        if let Some(r) = self.root_mut(key) {
            let list = r.pending.as_mut().unwrap_or(&mut r.files);
            match (list.iter().position(|f| &**f == rel), exists) {
                (None, true) => list.push(rel.into()),
                (Some(i), false) => {
                    list.swap_remove(i);
                }
                _ => {}
            }
        }
    }

    /// Remove every path under `rel_dir` (a deleted directory).
    pub fn remove_prefix(&mut self, key: RootKey, rel_dir: &str) {
        if let Some(r) = self.root_mut(key) {
            let prefix = format!("{}/", rel_dir.trim_end_matches('/'));
            r.files.retain(|f| !f.starts_with(&prefix) && &**f != rel_dir);
        }
    }

    pub fn total_files(&self) -> usize {
        self.roots.iter().map(|r| r.files.len()).sum()
    }

    pub fn is_complete(&self) -> bool {
        self.roots.iter().all(|r| r.complete)
    }

    /// Best `limit` hits for `query`. An empty query returns recent files.
    pub fn query(&self, query: &str, ctx: &QueryContext, limit: usize) -> Vec<Hit> {
        let q = Query::new(query);
        let recent_bonus: HashMap<(RootKey, &str), i32> = ctx
            .recent
            .iter()
            .enumerate()
            .map(|(i, (r, p))| ((*r, p.as_str()), 120 - (i as i32 * 3).min(100)))
            .chain(ctx.picked.iter().enumerate().map(|(i, (r, p))| ((*r, p.as_str()), 100 - (i as i32 * 3).min(80))))
            .fold(HashMap::new(), |mut m, (k, v)| {
                let e = m.entry(k).or_insert(0);
                *e = (*e).max(v);
                m
            });
        if q.is_empty() {
            // Recently opened files that still exist in the index.
            let mut out = Vec::new();
            for (r, p) in ctx.recent.iter().chain(ctx.picked.iter()) {
                if out.len() >= limit {
                    break;
                }
                if out.iter().any(|h: &Hit| h.root == *r && &*h.rel == p.as_str()) {
                    continue;
                }
                if self.root(*r).is_some_and(|ri| ri.files.iter().any(|f| &**f == p.as_str())) {
                    out.push(Hit { root: *r, rel: p.as_str().into(), score: 0, indices: vec![] });
                }
            }
            return out;
        }
        // Score in parallel chunks (each keeps its own top-K), then merge.
        let fallbacks: HashMap<RootKey, Vec<Option<i32>>> =
            self.roots.iter().map(|r| (r.root, root_fallbacks(&q, &r.name))).collect();
        let work: Vec<(RootKey, i32, &[Box<str>])> = self
            .roots
            .iter()
            .flat_map(|r| {
                let rb = if ctx.active_root == Some(r.root) { 5 } else { 0 };
                r.files.chunks(PAR_CHUNK).map(move |c| (r.root, rb, c))
            })
            .collect();
        let threads = std::thread::available_parallelism().map(|n| n.get().min(8)).unwrap_or(1).max(1);
        let score_chunks = |chunks: &[(RootKey, i32, &[Box<str>])]| -> Vec<Ranked> {
            let mut heap: BinaryHeap<Ranked> = BinaryHeap::with_capacity(limit + 1);
            for (root, root_bonus, files) in chunks {
                let fb = fallbacks.get(root).map(Vec::as_slice).unwrap_or(&[]);
                for f in files.iter() {
                    let Some(m) = match_path_in_root(&q, f, fb) else { continue };
                    let score = m.score + root_bonus + recent_bonus.get(&(*root, &**f)).copied().unwrap_or(0);
                    let cand = Ranked(Hit { root: *root, rel: f.clone(), score, indices: m.indices }, 0);
                    if heap.len() < limit {
                        heap.push(cand);
                    } else if let Some(worst) = heap.peek()
                        && cand < *worst
                    {
                        heap.pop();
                        heap.push(cand);
                    }
                }
            }
            heap.into_vec()
        };
        let mut all: Vec<Ranked> = if work.len() <= 1 || threads == 1 {
            score_chunks(&work)
        } else {
            let per = work.len().div_ceil(threads);
            std::thread::scope(|s| {
                let handles: Vec<_> = work.chunks(per).map(|part| s.spawn(move || score_chunks(part))).collect();
                handles.into_iter().flat_map(|h| h.join().unwrap_or_default()).collect()
            })
        };
        all.sort();
        all.truncate(limit);
        all.into_iter().map(|r| r.0).collect()
    }

    // --- warm-start cache ---------------------------------------------------

    fn cache_file(dir: &Path, root_path: &Path) -> PathBuf {
        let mut h: u64 = 0xcbf29ce484222325;
        for b in root_path.to_string_lossy().bytes() {
            h ^= u64::from(b);
            h = h.wrapping_mul(0x100000001b3);
        }
        dir.join("index").join(format!("{h:016x}.txt"))
    }

    pub fn save_cache(&self, dir: &Path) -> io::Result<()> {
        for r in self.roots.iter().filter(|r| r.complete && !r.truncated) {
            let mut text = String::with_capacity(r.files.len() * 32);
            for f in &r.files {
                if !f.contains('\n') {
                    text.push_str(f);
                    text.push('\n');
                }
            }
            yace_config::atomic_write(&Self::cache_file(dir, &r.path), text.as_bytes())?;
        }
        Ok(())
    }

    /// Load cached paths for a root with no data yet; marks it stale.
    pub fn load_cache(&mut self, dir: &Path, key: RootKey) -> bool {
        let Some(path) = self.root(key).filter(|r| r.files.is_empty()).map(|r| r.path.clone()) else { return false };
        let Ok(text) = std::fs::read_to_string(Self::cache_file(dir, &path)) else { return false };
        if let Some(r) = self.root_mut(key) {
            r.files = text.lines().filter(|l| !l.is_empty()).map(Into::into).collect();
            r.stale = true;
            return true;
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn index() -> FileIndex {
        let mut ix = FileIndex::new();
        ix.ensure_root(1, "API", Path::new("/api"));
        ix.ensure_root(2, "Web", Path::new("/web"));
        ix.add_batch(1, vec!["src/index.ts".into(), "src/jobs.service.ts".into(), "package.json".into()]);
        ix.add_batch(2, vec!["src/index.ts".into(), "next.config.ts".into(), "package.json".into()]);
        ix.finish_walk(1, false);
        ix.finish_walk(2, false);
        ix
    }

    #[test]
    fn query_across_roots_bounded_and_deterministic() {
        let ix = index();
        let hits = ix.query("index", &QueryContext::default(), 10);
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].rel, hits[1].rel);
        assert_ne!(hits[0].root, hits[1].root);
        assert_eq!(hits, ix.query("index", &QueryContext::default(), 10));
        assert_eq!(ix.query("s", &QueryContext::default(), 2).len(), 2, "bounded");
    }

    #[test]
    fn recency_and_active_root_influence_ranking() {
        let ix = index();
        let ctx = QueryContext { recent: vec![(2, "package.json".into())], ..Default::default() };
        assert_eq!(ix.query("package", &ctx, 10)[0].root, 2);
        let ctx = QueryContext { active_root: Some(1), ..Default::default() };
        assert_eq!(ix.query("package", &ctx, 10)[0].root, 1);
        let ctx = QueryContext { recent: vec![(2, "next.config.ts".into())], ..Default::default() };
        assert_eq!(ix.query("", &ctx, 10)[0].rel.as_ref(), "next.config.ts");
    }

    #[test]
    fn rewalk_swaps_atomically_and_events_patch() {
        let mut ix = index();
        ix.begin_walk(1);
        ix.add_batch(1, vec!["only.rs".into()]);
        assert!(ix.root(1).unwrap().files.len() == 3, "old data stays visible during re-walk");
        ix.finish_walk(1, false);
        assert_eq!(ix.root(1).unwrap().files.len(), 1);
        ix.apply_change(1, "new.rs", true);
        ix.apply_change(1, "only.rs", false);
        assert_eq!(ix.root(1).unwrap().files.iter().map(|f| &**f).collect::<Vec<_>>(), vec!["new.rs"]);
        ix.remove_prefix(2, "src");
        assert!(!ix.root(2).unwrap().files.iter().any(|f| f.starts_with("src/")));
    }

    #[test]
    fn cache_roundtrip_is_stale() {
        let d = tempfile::tempdir().unwrap();
        let ix = index();
        ix.save_cache(d.path()).unwrap();
        let mut fresh = FileIndex::new();
        fresh.ensure_root(7, "API", Path::new("/api"));
        assert!(fresh.load_cache(d.path(), 7));
        let r = fresh.root(7).unwrap();
        assert!(r.stale && !r.complete);
        assert_eq!(r.files.len(), 3);
    }
}
