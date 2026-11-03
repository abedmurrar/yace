//! Background work for M4 (spec §21.2–§21.4): the Quick Open index actor,
//! the workspace-search worker, the in-file find worker and async file
//! loading. Each request carries a generation number; the app ignores
//! results whose generation is stale.
//!
//! [`IndexEngine`] holds the logic and is used synchronously by tests; the
//! runtime wraps it in a thread ([`spawn_index_actor`]).

use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

use crossbeam_channel::{Receiver, Sender, unbounded};
use yace_buffer::{Document, LineEnding, Rope};
use yace_search::{
    ContentQuery, FileIndex, FindOptions, FindResult, Hit, QueryContext, RootKey, SearchRoot, WalkOptions, content,
    find,
};

use crate::app::AppEvent;

/// Roots to index: (key, display name, absolute path).
pub type IndexRoots = Vec<(RootKey, String, PathBuf)>;

#[derive(Debug, Clone)]
pub enum IndexCmd {
    /// Mirror the workspace roots; (re)walk roots that are missing, stale or
    /// older than `max_age`.
    Sync {
        roots: IndexRoots,
        walk: WalkOptions,
        cache_dir: Option<PathBuf>,
        max_age: Duration,
    },
    Query {
        seq: u64,
        query: String,
        ctx: QueryContext,
        limit: usize,
    },
    FsChange {
        root: RootKey,
        rel: String,
        exists: bool,
        is_dir: bool,
    },
    Batch {
        root: RootKey,
        files: Vec<Box<str>>,
    },
    WalkDone {
        root: RootKey,
        truncated: bool,
    },
}

/// A walk the engine wants started.
#[derive(Debug, Clone)]
pub struct WalkJob {
    pub root: RootKey,
    pub path: PathBuf,
    pub walk: WalkOptions,
    pub cancel: Arc<AtomicBool>,
}

#[derive(Default)]
pub struct IndexEngine {
    pub index: FileIndex,
    walking: HashMap<RootKey, Arc<AtomicBool>>,
    cache_dir: Option<PathBuf>,
    walk: Option<WalkOptions>,
}

impl IndexEngine {
    /// Handle a sync request; returns the walks to start.
    pub fn sync(
        &mut self,
        roots: &IndexRoots,
        walk: &WalkOptions,
        cache_dir: Option<PathBuf>,
        max_age: Duration,
    ) -> Vec<WalkJob> {
        let opts_changed = self.walk.as_ref().is_some_and(|w| {
            w.exclude != walk.exclude
                || w.use_ignore_files != walk.use_ignore_files
                || w.include_hidden != walk.include_hidden
        });
        self.walk = Some(walk.clone());
        self.cache_dir = cache_dir;
        let keys: Vec<RootKey> = roots.iter().map(|r| r.0).collect();
        for (k, c) in &self.walking {
            if !keys.contains(k) {
                c.store(true, Ordering::Relaxed);
            }
        }
        self.walking.retain(|k, _| keys.contains(k));
        self.index.retain_roots(&keys);
        let mut jobs = Vec::new();
        for (key, name, path) in roots {
            self.index.ensure_root(*key, name, path);
            if let Some(dir) = &self.cache_dir {
                self.index.load_cache(dir, *key);
            }
            if self.walking.contains_key(key) {
                continue;
            }
            let r = self.index.root(*key);
            let due = opts_changed
                || r.is_none_or(|r| !r.complete || r.stale || r.updated.is_none_or(|u| u.elapsed() >= max_age));
            if due {
                let cancel = Arc::new(AtomicBool::new(false));
                self.walking.insert(*key, Arc::clone(&cancel));
                self.index.begin_walk(*key);
                jobs.push(WalkJob { root: *key, path: path.clone(), walk: walk.clone(), cancel });
            }
        }
        jobs
    }

    pub fn batch(&mut self, root: RootKey, files: Vec<Box<str>>) {
        if self.walking.contains_key(&root) {
            self.index.add_batch(root, files);
        }
    }

    pub fn walk_done(&mut self, root: RootKey, truncated: bool) {
        if self.walking.remove(&root).is_some() {
            self.index.finish_walk(root, truncated);
            if let Some(dir) = &self.cache_dir {
                let _ = self.index.save_cache(dir);
            }
        }
    }

    pub fn fs_change(&mut self, root: RootKey, rel: &str, exists: bool, is_dir: bool) {
        if !exists {
            // Unknown whether it was a file or a directory: drop both.
            self.index.apply_change(root, rel, false);
            self.index.remove_prefix(root, rel);
        } else if !is_dir {
            self.index.apply_change(root, rel, true);
        }
        // A created directory is picked up by the next re-walk.
    }

    pub fn is_walking(&self) -> bool {
        !self.walking.is_empty()
    }

    pub fn query(&self, seq: u64, query: &str, ctx: &QueryContext, limit: usize) -> AppEvent {
        let hits = self.index.query(query, ctx, limit);
        AppEvent::QuickOpenResults {
            seq,
            hits,
            indexed: self.index.total_files(),
            complete: !self.is_walking(),
            truncated: self.index.roots().iter().any(|r| r.truncated),
        }
    }
}

/// Run a walk (on the calling thread), feeding batches to `sink`.
pub fn run_walk(job: &WalkJob, sink: &(dyn Fn(IndexCmd) + Sync)) {
    let summary = yace_search::walk::walk_root(&job.path, &job.walk, &job.cancel, &|files| {
        sink(IndexCmd::Batch { root: job.root, files })
    });
    sink(IndexCmd::WalkDone { root: job.root, truncated: summary.truncated });
}

/// Start the index actor thread.
pub fn spawn_index_actor(events: Sender<AppEvent>) -> Sender<IndexCmd> {
    let (tx, rx): (Sender<IndexCmd>, Receiver<IndexCmd>) = unbounded();
    let self_tx = tx.clone();
    thread::Builder::new()
        .name("yace-index".into())
        .spawn(move || {
            let mut engine = IndexEngine::default();
            let mut last_progress = Instant::now();
            while let Ok(first) = rx.recv() {
                // Coalesce: keep only the latest query in a burst.
                let mut cmds = vec![first];
                cmds.extend(rx.try_iter());
                let last_query = cmds.iter().rposition(|c| matches!(c, IndexCmd::Query { .. }));
                let mut changed = false;
                for (i, cmd) in cmds.into_iter().enumerate() {
                    match cmd {
                        IndexCmd::Sync { roots, walk, cache_dir, max_age } => {
                            for job in engine.sync(&roots, &walk, cache_dir, max_age) {
                                let tx = self_tx.clone();
                                thread::Builder::new()
                                    .name("yace-walk".into())
                                    .spawn(move || run_walk(&job, &|c| drop(tx.send(c))))
                                    .ok();
                            }
                            changed = true;
                        }
                        IndexCmd::Query { seq, query, ctx, limit } => {
                            if Some(i) == last_query && events.send(engine.query(seq, &query, &ctx, limit)).is_err() {
                                return;
                            }
                        }
                        IndexCmd::FsChange { root, rel, exists, is_dir } => {
                            engine.fs_change(root, &rel, exists, is_dir);
                            changed = true;
                        }
                        IndexCmd::Batch { root, files } => {
                            engine.batch(root, files);
                            changed = true;
                        }
                        IndexCmd::WalkDone { root, truncated } => {
                            engine.walk_done(root, truncated);
                            changed = true;
                            last_progress = Instant::now() - Duration::from_secs(1);
                        }
                    }
                }
                // Throttled progress so an open Quick Open can refresh.
                if changed && last_progress.elapsed() >= Duration::from_millis(150) {
                    last_progress = Instant::now();
                    let _ = events.send(AppEvent::IndexProgress {
                        files: engine.index.total_files(),
                        complete: !engine.is_walking(),
                    });
                }
            }
        })
        .expect("spawn index actor");
    tx
}

// ---------------------------------------------------------------------------
// Workspace search

#[derive(Debug, Clone)]
pub struct SearchJob {
    pub seq: u64,
    pub roots: Vec<SearchRoot>,
    pub query: ContentQuery,
    pub overlays: HashMap<PathBuf, Arc<str>>,
}

/// Run a search on the calling thread, sending events.
pub fn run_search(job: &SearchJob, cancel: &AtomicBool, send: &(dyn Fn(AppEvent) + Sync)) {
    let seq = job.seq;
    let result = content::search(&job.roots, &job.query, &job.overlays, cancel, &|file| {
        send(AppEvent::SearchFile { seq, file: Box::new(file) })
    });
    match result {
        Ok(summary) => send(AppEvent::SearchDone { seq, summary, error: None }),
        Err(e) => send(AppEvent::SearchDone { seq, summary: Default::default(), error: Some(e) }),
    }
}

/// Owns the cancel flag of the running search.
#[derive(Default)]
pub struct SearchRunner {
    current: Option<Arc<AtomicBool>>,
}

impl SearchRunner {
    pub fn start(&mut self, job: SearchJob, events: Sender<AppEvent>) {
        self.cancel();
        let cancel = Arc::new(AtomicBool::new(false));
        self.current = Some(Arc::clone(&cancel));
        thread::Builder::new()
            .name("yace-search".into())
            .spawn(move || run_search(&job, &cancel, &|e| drop(events.send(e))))
            .ok();
    }
    pub fn cancel(&mut self) {
        if let Some(c) = self.current.take() {
            c.store(true, Ordering::Relaxed);
        }
    }
}

// ---------------------------------------------------------------------------
// In-file find

#[derive(Debug, Clone)]
pub struct FindJob {
    pub seq: u64,
    pub rope: Rope,
    pub query: String,
    pub opts: FindOptions,
    pub within: Option<(usize, usize)>,
}

pub fn run_find(job: &FindJob) -> AppEvent {
    let result = match find::build_regex(&job.query, job.opts) {
        Ok(re) => Ok(find::find_in_text(&job.rope.to_string(), &re, job.within)),
        Err(e) => Err(e),
    };
    AppEvent::FindDone { seq: job.seq, result }
}

/// A find worker that only runs the latest job of a burst.
pub fn spawn_find_worker(events: Sender<AppEvent>) -> Sender<FindJob> {
    let (tx, rx): (Sender<FindJob>, Receiver<FindJob>) = unbounded();
    thread::Builder::new()
        .name("yace-find".into())
        .spawn(move || {
            while let Ok(mut job) = rx.recv() {
                for newer in rx.try_iter() {
                    job = newer;
                }
                if events.send(run_find(&job)).is_err() {
                    break;
                }
            }
        })
        .expect("spawn find worker");
    tx
}

/// The syntax worker: parses documents off the UI thread. Queued jobs for
/// the same document collapse to the newest.
pub fn spawn_syntax_worker(events: Sender<AppEvent>) -> Sender<crate::syntax::ParseJob> {
    let (tx, rx): (Sender<crate::syntax::ParseJob>, Receiver<crate::syntax::ParseJob>) = unbounded();
    thread::Builder::new()
        .name("yace-syntax".into())
        .spawn(move || {
            while let Ok(first) = rx.recv() {
                let mut batch = vec![first];
                for job in rx.try_iter() {
                    batch.retain(|j| j.doc != job.doc);
                    batch.push(job);
                }
                for job in &batch {
                    if events.send(crate::syntax::run_parse(job)).is_err() {
                        return;
                    }
                }
            }
        })
        .expect("spawn syntax worker");
    tx
}

// ---------------------------------------------------------------------------
// Async file loading (large files)

pub fn load_file(id: crate::editor::DocId, path: PathBuf, eol: LineEnding) -> AppEvent {
    let result = Document::open(&path, eol).map(Box::new).map_err(|e| e.to_string());
    AppEvent::FileLoaded { doc: id, path, result }
}

pub type FindResultOr = Result<FindResult, String>;
pub type Hits = Vec<Hit>;
