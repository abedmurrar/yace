//! The event loop (spec §21, §22.1). One channel of [`AppEvent`]s fed by:
//! an input thread, a bounded pool of filesystem workers, the notify
//! watcher and a signal thread. The loop blocks on the channel — with a
//! timeout only when a deadline is pending — so idle CPU is zero, and it
//! renders only after state changed.

use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{Arc, Mutex},
    thread,
    time::Instant,
};

use crossbeam_channel::{Receiver, Sender, TrySendError, bounded, unbounded};
use notify::{RecursiveMode, Watcher};
use ratatui::crossterm::event;

use crate::{
    app::{App, AppEvent, Effect, Severity},
    explorer::read_dir_entries,
    terminal::{Term, write_raw},
};

/// Bounded pool of workers for directory listings (spec §21.5).
///
/// One read per directory at a time. A request for a directory that is
/// queued is dropped (the queued read will see the latest state); a request
/// for one being read marks it to be read once more afterwards, so a change
/// made during a read is never missed and the newest listing arrives last.
pub struct FsPool {
    jobs: Sender<PathBuf>,
    state: Arc<Mutex<HashMap<PathBuf, ReadState>>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ReadState {
    Queued,
    Running { again: bool },
}

impl FsPool {
    pub fn new(threads: usize, events: Sender<AppEvent>) -> Self {
        Self::with_reader(
            threads,
            move |dir, result| {
                let _ = events.send(AppEvent::DirLoaded { dir, result });
            },
            read_dir_entries,
        )
    }

    /// A pool with a custom reader and result sink (tests).
    pub fn with_reader(
        threads: usize,
        sink: impl Fn(PathBuf, Result<Vec<crate::explorer::Entry>, String>) + Send + Sync + Clone + 'static,
        read: impl Fn(&std::path::Path) -> Result<Vec<crate::explorer::Entry>, String> + Send + Sync + Clone + 'static,
    ) -> Self {
        let (tx, rx): (Sender<PathBuf>, Receiver<PathBuf>) = bounded(1024);
        let state: Arc<Mutex<HashMap<PathBuf, ReadState>>> = Arc::default();
        for i in 0..threads.max(1) {
            let rx = rx.clone();
            let tx = tx.clone();
            let state = Arc::clone(&state);
            let sink = sink.clone();
            let read = read.clone();
            thread::Builder::new()
                .name(format!("yace-fs-{i}"))
                .spawn(move || {
                    for dir in rx {
                        if let Ok(mut s) = state.lock() {
                            s.insert(dir.clone(), ReadState::Running { again: false });
                        }
                        let result = read(&dir);
                        let again = state.lock().ok().and_then(|mut s| match s.remove(&dir) {
                            Some(ReadState::Running { again: true }) => {
                                s.insert(dir.clone(), ReadState::Queued);
                                Some(())
                            }
                            _ => None,
                        });
                        sink(dir.clone(), result);
                        if again.is_some()
                            && tx.try_send(dir.clone()).is_err()
                            && let Ok(mut s) = state.lock()
                        {
                            s.remove(&dir);
                        }
                    }
                })
                .expect("spawn fs worker");
        }
        Self { jobs: tx, state }
    }

    pub fn read_dir(&self, dir: PathBuf) {
        if let Ok(mut s) = self.state.lock() {
            match s.get_mut(&dir) {
                Some(ReadState::Queued) => return,
                Some(ReadState::Running { again }) => {
                    *again = true;
                    return;
                }
                None => {
                    s.insert(dir.clone(), ReadState::Queued);
                }
            }
        }
        if let Err(TrySendError::Full(d) | TrySendError::Disconnected(d)) = self.jobs.try_send(dir) {
            tracing::warn!("fs queue full, dropping read of {}", d.display());
            if let Ok(mut s) = self.state.lock() {
                s.remove(&d);
            }
        }
    }
}

/// File watching on its own thread: adding thousands of watches (one per
/// indexed folder) never blocks the UI. Failures come back as
/// `AppEvent::WatchFailed`.
pub struct FsWatcher {
    tx: Option<Sender<WatchCmd>>,
}

enum WatchCmd {
    Watch(PathBuf),
    Unwatch(PathBuf),
}

impl FsWatcher {
    pub fn new(events: Sender<AppEvent>) -> (Self, Option<String>) {
        let sink = events.clone();
        let handler = move |res: notify::Result<notify::Event>| {
            if let Ok(ev) = res {
                use notify::EventKind::*;
                if ev.need_rescan() {
                    let _ = sink.send(AppEvent::FsRescan);
                }
                if matches!(ev.kind, Access(_)) {
                    return;
                }
                let _ = sink.send(AppEvent::FsChanged(ev.paths));
            }
        };
        let mut watcher = match notify::recommended_watcher(handler) {
            Ok(w) => w,
            Err(e) => {
                return (
                    Self { tx: None },
                    Some(format!("File watching is unavailable ({e}); use \"Refresh Explorer\"")),
                );
            }
        };
        let (tx, rx) = unbounded::<WatchCmd>();
        thread::Builder::new()
            .name("yace-watch".into())
            .spawn(move || {
                for cmd in rx {
                    match cmd {
                        WatchCmd::Watch(p) => {
                            if let Err(e) = watcher.watch(&p, RecursiveMode::NonRecursive) {
                                let _ = events.send(AppEvent::WatchFailed { path: p, error: e.to_string() });
                            }
                        }
                        WatchCmd::Unwatch(p) => {
                            let _ = watcher.unwatch(&p);
                        }
                    }
                }
            })
            .ok();
        (Self { tx: Some(tx) }, None)
    }
    pub fn watch(&mut self, dir: &std::path::Path) {
        if let Some(tx) = &self.tx {
            let _ = tx.send(WatchCmd::Watch(dir.to_path_buf()));
        }
    }
    pub fn unwatch(&mut self, dir: &std::path::Path) {
        if let Some(tx) = &self.tx {
            let _ = tx.send(WatchCmd::Unwatch(dir.to_path_buf()));
        }
    }
}

pub struct Runtime {
    pub tx: Sender<AppEvent>,
    rx: Receiver<AppEvent>,
    pool: FsPool,
    watcher: FsWatcher,
    index: Sender<crate::workers::IndexCmd>,
    search: crate::workers::SearchRunner,
    /// Started on the first live search update.
    search_updates: Option<crate::workers::SearchUpdater>,
    find: Sender<crate::workers::FindJob>,
    /// Started on the first terminal search.
    term_find: Option<Sender<crate::workers::TermFindJob>>,
    /// Started on the first background parse.
    syntax: Option<Sender<crate::syntax::ParseJob>>,
    git: Option<Sender<crate::git::GitTask>>,
    /// Writes backups of unsaved documents, in order; joined before exit.
    backups: Option<(Sender<crate::app::recovery::BackupJob>, thread::JoinHandle<()>)>,
}

impl Runtime {
    pub fn new(app: &mut App) -> Self {
        let (tx, rx) = unbounded();
        let threads = thread::available_parallelism().map(|n| n.get().clamp(2, 4)).unwrap_or(2);
        let pool = FsPool::new(threads, tx.clone());
        let (watcher, err) = FsWatcher::new(tx.clone());
        if let Some(e) = err {
            app.notify(Severity::Warning, e);
        }
        // Terminal sessions notify through the app's channel; forward them.
        let notices = app.terminal_notices();
        let ntx = tx.clone();
        thread::Builder::new()
            .name("yace-term-notices".into())
            .spawn(move || {
                for n in notices {
                    if ntx.send(AppEvent::TerminalNotice(n)).is_err() {
                        break;
                    }
                }
            })
            .expect("spawn notice forwarder");
        // Language server messages arrive decoded; forward them too.
        let lsp = app.lsp_events();
        let ltx = tx.clone();
        thread::Builder::new()
            .name("yace-lsp-events".into())
            .spawn(move || {
                for (server, msg) in lsp {
                    if ltx.send(AppEvent::Lsp { server, msg: Box::new(msg) }).is_err() {
                        break;
                    }
                }
            })
            .expect("spawn lsp forwarder");
        // Messages from plugin jobs.
        let plugin_msgs = app.plugins.messages();
        let ptx = tx.clone();
        thread::Builder::new()
            .name("yace-plugin-msgs".into())
            .spawn(move || {
                for m in plugin_msgs {
                    if ptx.send(AppEvent::Plugin(m)).is_err() {
                        break;
                    }
                }
            })
            .expect("spawn plugin forwarder");
        let index = crate::workers::spawn_index_actor(tx.clone());
        let find = crate::workers::spawn_find_worker(tx.clone());
        Self {
            tx,
            rx,
            pool,
            watcher,
            index,
            search: Default::default(),
            search_updates: None,
            term_find: None,
            find,
            syntax: None,
            backups: None,
            git: None,
        }
    }

    pub fn spawn_input_thread(&self) {
        let tx = self.tx.clone();
        thread::Builder::new()
            .name("yace-input".into())
            .spawn(move || {
                while let Ok(ev) = event::read() {
                    if tx.send(AppEvent::Terminal(ev)).is_err() {
                        break;
                    }
                }
            })
            .expect("spawn input thread");
    }

    pub fn spawn_signal_thread(&self) {
        use signal_hook::{consts::*, iterator::Signals};
        let tx = self.tx.clone();
        if let Ok(mut signals) = Signals::new([SIGTERM, SIGHUP]) {
            thread::Builder::new()
                .name("yace-signals".into())
                .spawn(move || {
                    if signals.forever().next().is_some() {
                        let _ = tx.send(AppEvent::Terminate);
                    }
                })
                .expect("spawn signal thread");
        }
    }

    fn apply_effects(&mut self, app: &mut App) {
        for e in app.take_effects() {
            match e {
                Effect::ReadDir(d) => self.pool.read_dir(d),
                Effect::Watch(d) => self.watcher.watch(&d),
                Effect::Unwatch(d) => self.watcher.unwatch(&d),
                Effect::Write(bytes) => write_raw(&bytes),
                Effect::Index(cmd) => {
                    let _ = self.index.send(cmd);
                }
                Effect::Search(job) => self.search.start(*job, self.tx.clone()),
                Effect::SearchCancel => self.search.cancel(),
                Effect::SearchUpdate(job) => {
                    let tx = self.tx.clone();
                    self.search_updates.get_or_insert_with(|| crate::workers::SearchUpdater::spawn(tx)).run(*job);
                }
                Effect::Find(job) => {
                    let _ = self.find.send(*job);
                }
                Effect::TermFind(job) => {
                    let tx = self.tx.clone();
                    let w = self.term_find.get_or_insert_with(|| crate::workers::spawn_term_find_worker(tx));
                    let _ = w.send(*job);
                }
                Effect::TrimMemory => crate::mem::trim(),
                Effect::Parse(job) => {
                    let tx = self.tx.clone();
                    let worker = self.syntax.get_or_insert_with(|| crate::workers::spawn_syntax_worker(tx));
                    let _ = worker.send(*job);
                }
                Effect::LoadFile { doc, path, eol } => {
                    let tx = self.tx.clone();
                    thread::Builder::new()
                        .name("yace-load".into())
                        .spawn(move || drop(tx.send(crate::workers::load_file(doc, path, eol))))
                        .ok();
                }
                Effect::Git(git, job) => {
                    let tx = self.tx.clone();
                    let workers = self.git.get_or_insert_with(|| crate::git::spawn_git_workers(tx));
                    let _ = workers.send((git, job));
                }
                Effect::CopyPaths(pairs) => {
                    let tx = self.tx.clone();
                    thread::Builder::new()
                        .name("yace-copy".into())
                        .spawn(move || {
                            drop(tx.send(AppEvent::PathsCopied(crate::app::explorer_ops::copy_paths(pairs))))
                        })
                        .ok();
                }
                Effect::OpenExternal(dir) => {
                    if let Err(e) = crate::app::explorer_ops::open_external(&dir) {
                        let _ = self.tx.send(AppEvent::OpenExternalFailed(e));
                    }
                }
                Effect::FetchPluginIndex(url) => {
                    let tx = self.tx.clone();
                    thread::Builder::new()
                        .name("yace-plugin-index".into())
                        .spawn(move || {
                            drop(tx.send(AppEvent::PluginIndex(yace_plugin_host::registry::load_index(&url))))
                        })
                        .ok();
                }
                Effect::DownloadPlugin { entry, plugins_dir } => {
                    let tx = self.tx.clone();
                    thread::Builder::new()
                        .name("yace-plugin-download".into())
                        .spawn(move || {
                            let r = yace_plugin_host::registry::plan_download(
                                &entry,
                                &plugins_dir,
                                crate::app::App::yace_version(),
                            );
                            drop(tx.send(AppEvent::PluginDownloaded(r.map(Box::new))))
                        })
                        .ok();
                }
                Effect::Backups(job) => {
                    let (tx, _) = self.backups.get_or_insert_with(|| {
                        let (tx, rx) = unbounded::<crate::app::recovery::BackupJob>();
                        let h = thread::Builder::new()
                            .name("yace-backups".into())
                            .spawn(move || {
                                for job in rx {
                                    job.run();
                                }
                            })
                            .expect("spawn the backup thread");
                        (tx, h)
                    });
                    let _ = tx.send(job);
                }
                Effect::DiscoverPlugins(dir) => {
                    let tx = self.tx.clone();
                    thread::Builder::new()
                        .name("yace-plugins".into())
                        .spawn(move || {
                            drop(tx.send(AppEvent::PluginsDiscovered(yace_plugin_host::store::discover(&dir))))
                        })
                        .ok();
                }
            }
        }
    }

    /// Run until the app quits. `exit_after_first_frame` supports
    /// `--startup-time`.
    pub fn run(&mut self, app: &mut App, term: &mut Term, exit_after_first_frame: bool) -> std::io::Result<()> {
        let size = term.size()?;
        app.resize(size.width, size.height);
        let mut warned_stuck = false;
        loop {
            self.apply_effects(app);
            if app.needs_render {
                let start = Instant::now();
                term.draw(|f| crate::ui::draw(app, f))?;
                app.metrics.render.record(start.elapsed());
                app.metrics.frames += 1;
                app.needs_render = false;
                if app.metrics.first_frame.is_none() {
                    app.metrics.first_frame = Some(app.metrics.process_start.elapsed());
                    if exit_after_first_frame {
                        return Ok(());
                    }
                }
            }
            if app.quit {
                // `shutdown`/`exit` to every server; closing their stdin
                // when we exit stops any that ignore it.
                app.lsp.shutdown_all();
                // Finish writing backups (a closed terminal's unsaved work).
                if let Some((tx, h)) = self.backups.take() {
                    drop(tx);
                    let _ = h.join();
                }
                return Ok(());
            }
            let ev = match app.next_deadline() {
                Some(d) => {
                    let now = Instant::now();
                    // Never skip the channel: input and signals must get
                    // through even if a due time keeps coming back. If
                    // ticking could not clear it, wait a little instead
                    // of spinning (a bug elsewhere; logged once).
                    let wait = if d <= now {
                        app.on_tick(now);
                        let stuck = app.next_deadline().is_some_and(|d| d <= Instant::now());
                        if stuck && !warned_stuck {
                            warned_stuck = true;
                            tracing::warn!("a deadline stays due after ticking; throttling the event loop");
                        }
                        if stuck { std::time::Duration::from_millis(5) } else { std::time::Duration::ZERO }
                    } else {
                        d - now
                    };
                    match self.rx.recv_timeout(wait) {
                        Ok(ev) => Some(ev),
                        Err(crossbeam_channel::RecvTimeoutError::Timeout) => None,
                        Err(_) => return Ok(()),
                    }
                }
                None => match self.rx.recv() {
                    Ok(ev) => Some(ev),
                    Err(_) => return Ok(()),
                },
            };
            if let Some(ev) = ev {
                app.handle(ev);
                // Drain whatever else is queued before rendering, input
                // first, so bursts (paste, fast typing, mass fs events)
                // cost a single frame.
                let mut deferred = Vec::new();
                for _ in 0..512 {
                    match self.rx.try_recv() {
                        Ok(e @ AppEvent::Terminal(_)) => app.handle(e),
                        Ok(e) => deferred.push(e),
                        Err(_) => break,
                    }
                }
                for e in deferred {
                    app.handle(e);
                }
            }
            app.on_tick(Instant::now());
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{
        sync::{
            Arc, Mutex,
            atomic::{AtomicUsize, Ordering},
        },
        time::{Duration, Instant},
    };

    use super::*;

    /// A request made while a directory is being read causes exactly one
    /// more read, and the last result delivered is the newest.
    #[test]
    fn requests_during_a_read_are_not_lost() {
        let d = tempfile::tempdir().unwrap();
        let reads = Arc::new(AtomicUsize::new(0));
        let results: Arc<Mutex<Vec<usize>>> = Arc::default();
        let (r2, res2) = (Arc::clone(&reads), Arc::clone(&results));
        let pool = FsPool::with_reader(
            2,
            move |_dir, result: Result<Vec<crate::explorer::Entry>, String>| {
                res2.lock().unwrap().push(result.map(|e| e.len()).unwrap_or(0));
            },
            move |dir| {
                let n = r2.fetch_add(1, Ordering::SeqCst);
                if n == 0 {
                    // The first read is slow; a file appears meanwhile.
                    std::thread::sleep(Duration::from_millis(150));
                }
                read_dir_entries(dir)
            },
        );
        pool.read_dir(d.path().to_path_buf());
        std::thread::sleep(Duration::from_millis(30)); // the first read is running
        std::fs::write(d.path().join("new.txt"), "").unwrap();
        pool.read_dir(d.path().to_path_buf()); // marks "again"
        pool.read_dir(d.path().to_path_buf()); // coalesced
        let start = Instant::now();
        while results.lock().unwrap().len() < 2 && start.elapsed() < Duration::from_secs(5) {
            std::thread::sleep(Duration::from_millis(5));
        }
        std::thread::sleep(Duration::from_millis(100));
        assert_eq!(reads.load(Ordering::SeqCst), 2, "exactly one more read");
        let r = results.lock().unwrap().clone();
        assert_eq!(r.len(), 2);
        assert_eq!(*r.last().unwrap(), 1, "the newest listing (with new.txt) arrives last");
    }
}
