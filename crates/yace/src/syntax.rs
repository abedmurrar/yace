//! Per-document syntax state (spec §12.1–12.3, §21.4).
//!
//! Every document keeps a running estimate of what an incremental reparse
//! costs. Cheap reparses (predicted under 0.5 ms) run on the UI thread so the
//! next frame is exactly right; anything else goes to the syntax worker and
//! the frame is drawn from the *edited* old tree (positions already shifted),
//! so input never waits on parsing. Edits made while a job runs are replayed
//! onto its result. Highlight
//! spans are computed for the visible byte range only and cached until the
//! tree changes.

use std::{
    cell::RefCell,
    collections::{BTreeMap, HashMap},
    ops::Range,
    path::PathBuf,
    rc::Rc,
    time::{Duration, Instant},
};

use yace_buffer::{DocEdit, Document, Rope};
use yace_syntax::{Associations, InjectionCache, InputEdit, LanguageConfig, ParseOutcome, Span, Syntax, Tree};

use crate::editor::{DocId, DocumentStore};

/// Files up to this size are parsed on the UI thread when opened.
const SYNC_OPEN_LIMIT: usize = 32 * 1024;
/// Budget for the parse when a small file opens.
const OPEN_BUDGET: Duration = Duration::from_millis(8);
/// Reparse on the UI thread only when it is predicted to cost less.
const SYNC_COST: Duration = Duration::from_micros(500);
/// Safety cap for a predicted-cheap reparse that turns out not to be.
const EDIT_BUDGET: Duration = Duration::from_millis(1);
/// Cached span sets per document (one per visible viewport).
const SPAN_CACHE: usize = 4;

/// A background parse request.
#[derive(Clone)]
pub struct ParseJob {
    pub id: u64,
    pub doc: DocId,
    pub lang: &'static LanguageConfig,
    pub rope: Rope,
    pub old: Option<Tree>,
}

impl std::fmt::Debug for ParseJob {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "ParseJob({} doc {} {})", self.id, self.doc, self.lang.id)
    }
}

/// Run a parse job (worker thread, or inline in tests).
pub fn run_parse(job: &ParseJob) -> crate::app::AppEvent {
    let start = Instant::now();
    let tree = yace_syntax::parse_full(job.lang, &job.rope, job.old.as_ref());
    crate::app::AppEvent::Parsed { doc: job.doc, job: job.id, tree: tree.map(Box::new), cost: start.elapsed() }
}

struct CachedSpans {
    tree_gen: u64,
    range: Range<usize>,
    spans: Rc<[Span]>,
}

pub struct DocSyntax {
    pub lang: &'static LanguageConfig,
    path: Option<PathBuf>,
    syntax: Option<Syntax>,
    /// Bumped whenever the tree changes (edit or parse).
    tree_gen: u64,
    /// The background job whose result we are waiting for.
    in_flight: Option<u64>,
    /// Edits made while a job runs (document version after each).
    edits_since_job: Vec<(u64, InputEdit)>,
    injections: RefCell<InjectionCache>,
    cache: RefCell<Vec<CachedSpans>>,
    /// Running estimate of an incremental reparse (None until measured).
    cost: Option<Duration>,
    /// Whether the last background job was an incremental reparse (only
    /// those feed the cost estimate; a first full parse would skew it).
    job_incremental: bool,
}

impl DocSyntax {
    fn record_cost(&mut self, d: Duration) {
        self.cost = Some(match self.cost {
            Some(c) => (c + d) / 2,
            None => d,
        });
    }

    fn predicted_cheap(&self, len_bytes: usize) -> bool {
        match self.cost {
            Some(c) => c <= SYNC_COST,
            None => len_bytes <= SYNC_OPEN_LIMIT,
        }
    }

    /// Reparse on this thread if predicted cheap. Returns false when the work
    /// must go to the worker instead.
    fn try_sync_reparse(&mut self, rope: &Rope, version: u64) -> bool {
        let cheap = self.predicted_cheap(rope.len_bytes());
        let Some(syn) = self.syntax.as_mut() else { return true };
        if !cheap || syn.tree().is_none() {
            return false;
        }
        let start = Instant::now();
        let outcome = syn.parse(rope, version, Some(EDIT_BUDGET));
        let took = start.elapsed();
        self.tree_gen += 1;
        if outcome == ParseOutcome::Done {
            self.record_cost(took);
            true
        } else {
            // Push the estimate over the limit so the next edit goes async.
            self.record_cost(EDIT_BUDGET * 2);
            false
        }
    }
}

/// What a sync pass found, for other incremental consumers (LSP).
#[derive(Default)]
pub struct SyncOutput {
    pub jobs: Vec<ParseJob>,
    /// Documents seen for the first time (or re-detected after a rename).
    pub attached: Vec<DocId>,
    /// Documents that went away.
    pub removed: Vec<DocId>,
    /// Edits drained from each document, oldest first.
    pub edits: Vec<(DocId, Vec<DocEdit>)>,
}

#[derive(Default)]
pub struct SyntaxStore {
    docs: HashMap<DocId, DocSyntax>,
    assoc: Associations,
    assoc_src: BTreeMap<String, String>,
    next_job: u64,
}

fn input_edits(e: &DocEdit) -> Vec<InputEdit> {
    let changes: Vec<(usize, usize, &str)> =
        e.changes.changes().iter().map(|c| (c.from, c.to, c.insert.as_str())).collect();
    yace_syntax::input_edits(&e.before, &changes)
}

impl SyntaxStore {
    pub fn language(&self, d: DocId) -> &'static LanguageConfig {
        self.docs.get(&d).map(|s| s.lang).unwrap_or_else(yace_syntax::plain_text)
    }

    /// Whether `d` has a tree (for tests and the status bar).
    pub fn has_tree(&self, d: DocId) -> bool {
        self.docs.get(&d).and_then(|s| s.syntax.as_ref()).is_some_and(|s| s.tree().is_some())
    }

    pub fn tree(&self, d: DocId) -> Option<&Tree> {
        self.docs.get(&d)?.syntax.as_ref()?.tree()
    }

    /// Highlight spans for `range` (bytes), from the cache when possible.
    pub fn spans(&self, d: DocId, rope: &Rope, range: Range<usize>) -> Option<Rc<[Span]>> {
        let ds = self.docs.get(&d)?;
        let tree = ds.syntax.as_ref()?.tree()?;
        let mut cache = ds.cache.borrow_mut();
        if let Some(c) = cache.iter().find(|c| c.tree_gen == ds.tree_gen && c.range == range) {
            return Some(c.spans.clone());
        }
        let spans: Rc<[Span]> =
            yace_syntax::highlight(ds.lang, tree, rope, range.clone(), ds.tree_gen, &mut ds.injections.borrow_mut())
                .into();
        cache.retain(|c| c.tree_gen == ds.tree_gen);
        if cache.len() >= SPAN_CACHE {
            cache.remove(0);
        }
        cache.push(CachedSpans { tree_gen: ds.tree_gen, range, spans: spans.clone() });
        Some(spans)
    }

    fn job(&mut self, d: DocId, rope: &Rope) -> Option<ParseJob> {
        let ds = self.docs.get_mut(&d)?;
        let syn = ds.syntax.as_ref()?;
        self.next_job += 1;
        ds.in_flight = Some(self.next_job);
        ds.edits_since_job.clear();
        let old = syn.tree_for_worker();
        ds.job_incremental = old.is_some();
        Some(ParseJob { id: self.next_job, doc: d, lang: ds.lang, rope: rope.clone(), old })
    }

    fn attach(&mut self, d: DocId, doc: &mut Document) -> Option<ParseJob> {
        let first: String = doc.rope().lines().next().map(|l| l.chars().take(200).collect()).unwrap_or_default();
        let lang = yace_syntax::detect(doc.path(), &first, &self.assoc);
        doc.track_edits();
        doc.take_edits();
        let syntax = if doc.large_file { None } else { Syntax::new(lang) };
        let has_syntax = syntax.is_some();
        self.docs.insert(
            d,
            DocSyntax {
                lang,
                path: doc.path().map(PathBuf::from),
                syntax,
                tree_gen: 0,
                in_flight: None,
                edits_since_job: Vec::new(),
                injections: RefCell::default(),
                cache: RefCell::default(),
                cost: None,
                job_incremental: false,
            },
        );
        if !has_syntax {
            return None;
        }
        let rope = doc.rope();
        if rope.len_bytes() <= SYNC_OPEN_LIMIT {
            let ds = self.docs.get_mut(&d)?;
            let outcome = ds.syntax.as_mut()?.parse(rope, doc.version(), Some(OPEN_BUDGET));
            ds.tree_gen += 1;
            if outcome == ParseOutcome::Done {
                return None;
            }
        }
        self.job(d, rope)
    }

    /// Bring syntax state in line with the documents: attach new ones,
    /// re-detect after a rename, feed edits. Returns background jobs.
    pub fn sync(&mut self, docs: &mut DocumentStore, associations: &BTreeMap<String, String>) -> SyncOutput {
        let mut out = SyncOutput::default();
        let assoc_changed = *associations != self.assoc_src;
        if assoc_changed {
            self.assoc_src = associations.clone();
            let pairs: Vec<(String, String)> = associations.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
            self.assoc = Associations::new(&pairs);
        }
        self.docs.retain(|id, _| {
            let keep = docs.get(*id).is_some();
            if !keep {
                out.removed.push(*id);
            }
            keep
        });
        for id in docs.ids() {
            let Some(entry) = docs.get_mut(id) else { continue };
            if entry.loading {
                continue;
            }
            let doc = &mut entry.doc;
            let stale_attach = match self.docs.get(&id) {
                None => true,
                Some(ds) => assoc_changed || ds.path.as_deref() != doc.path() || !doc.is_tracking_edits(),
            };
            if stale_attach {
                out.jobs.extend(self.attach(id, doc));
                out.attached.push(id);
                continue;
            }
            let edits = doc.take_edits();
            if edits.is_empty() {
                continue;
            }
            let Some(ds) = self.docs.get_mut(&id) else { continue };
            let Some(syn) = ds.syntax.as_mut() else {
                out.edits.push((id, edits));
                continue;
            };
            for e in &edits {
                let ies = input_edits(e);
                syn.edit(&ies);
                if ds.in_flight.is_some() {
                    ds.edits_since_job.extend(ies.into_iter().map(|ie| (e.version, ie)));
                }
            }
            ds.tree_gen += 1;
            if ds.in_flight.is_none() && !ds.try_sync_reparse(doc.rope(), doc.version()) {
                out.jobs.extend(self.job(id, doc.rope()));
            }
            out.edits.push((id, edits));
        }
        out
    }

    /// A background parse finished. Edits made since the job started are
    /// replayed onto the new tree, then it is brought up to date (within the
    /// edit budget, or by another job).
    pub fn on_parsed(
        &mut self,
        docs: &DocumentStore,
        d: DocId,
        job: u64,
        tree: Option<Tree>,
        cost: Duration,
    ) -> Option<ParseJob> {
        let doc = docs.doc(d)?;
        let ds = self.docs.get_mut(&d)?;
        if ds.in_flight != Some(job) {
            return None;
        }
        ds.in_flight = None;
        if ds.job_incremental {
            ds.record_cost(cost);
        }
        let mut tree = tree?;
        let replay = std::mem::take(&mut ds.edits_since_job);
        let syn = ds.syntax.as_mut()?;
        for (_, e) in &replay {
            tree.edit(e);
        }
        syn.set_tree(tree, doc.version());
        ds.tree_gen += 1;
        if replay.is_empty() {
            return None;
        }
        syn.stale = true;
        if ds.try_sync_reparse(doc.rope(), doc.version()) { None } else { self.job(d, doc.rope()) }
    }
}
