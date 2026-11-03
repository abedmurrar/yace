//! The event loop (spec §21, §22.1). One channel of [`AppEvent`]s fed by:
//! an input thread, a bounded pool of filesystem workers, the notify
//! watcher and a signal thread. The loop blocks on the channel — with a
//! timeout only when a deadline is pending — so idle CPU is zero, and it
//! renders only after state changed.

use std::{
    collections::HashSet,
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
pub struct FsPool {
    jobs: Sender<PathBuf>,
    in_flight: Arc<Mutex<HashSet<PathBuf>>>,
}

impl FsPool {
    pub fn new(threads: usize, events: Sender<AppEvent>) -> Self {
        let (tx, rx): (Sender<PathBuf>, Receiver<PathBuf>) = bounded(1024);
        let in_flight = Arc::new(Mutex::new(HashSet::new()));
        for i in 0..threads.max(1) {
            let rx = rx.clone();
            let events = events.clone();
            let in_flight = Arc::clone(&in_flight);
            thread::Builder::new()
                .name(format!("yace-fs-{i}"))
                .spawn(move || {
                    for dir in rx {
                        let result = read_dir_entries(&dir);
                        if let Ok(mut f) = in_flight.lock() {
                            f.remove(&dir);
                        }
                        if events.send(AppEvent::DirLoaded { dir, result }).is_err() {
                            break;
                        }
                    }
                })
                .expect("spawn fs worker");
        }
        Self { jobs: tx, in_flight }
    }

    pub fn read_dir(&self, dir: PathBuf) {
        if let Ok(mut f) = self.in_flight.lock()
            && !f.insert(dir.clone())
        {
            return; // already queued
        }
        if let Err(TrySendError::Full(d) | TrySendError::Disconnected(d)) = self.jobs.try_send(dir) {
            tracing::warn!("fs queue full, dropping read of {}", d.display());
            if let Ok(mut f) = self.in_flight.lock() {
                f.remove(&d);
            }
        }
    }
}

pub struct FsWatcher {
    inner: Option<notify::RecommendedWatcher>,
}

impl FsWatcher {
    pub fn new(events: Sender<AppEvent>) -> (Self, Option<String>) {
        let handler = move |res: notify::Result<notify::Event>| {
            if let Ok(ev) = res {
                use notify::EventKind::*;
                if matches!(ev.kind, Access(_)) {
                    return;
                }
                let _ = events.send(AppEvent::FsChanged(ev.paths));
            }
        };
        match notify::recommended_watcher(handler) {
            Ok(w) => (Self { inner: Some(w) }, None),
            Err(e) => {
                (Self { inner: None }, Some(format!("File watching is unavailable ({e}); use \"Refresh Explorer\"")))
            }
        }
    }
    pub fn watch(&mut self, dir: &std::path::Path) -> Result<(), String> {
        match &mut self.inner {
            Some(w) => w.watch(dir, RecursiveMode::NonRecursive).map_err(|e| e.to_string()),
            None => Ok(()),
        }
    }
    pub fn unwatch(&mut self, dir: &std::path::Path) {
        if let Some(w) = &mut self.inner {
            let _ = w.unwatch(dir);
        }
    }
}

pub struct Runtime {
    pub tx: Sender<AppEvent>,
    rx: Receiver<AppEvent>,
    pool: FsPool,
    watcher: FsWatcher,
    watch_errors_reported: bool,
    index: Sender<crate::workers::IndexCmd>,
    search: crate::workers::SearchRunner,
    find: Sender<crate::workers::FindJob>,
    /// Started on the first background parse.
    syntax: Option<Sender<crate::syntax::ParseJob>>,
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
            watch_errors_reported: false,
            index,
            search: Default::default(),
            find,
            syntax: None,
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
                Effect::Watch(d) => {
                    if let Err(err) = self.watcher.watch(&d)
                        && !self.watch_errors_reported
                    {
                        self.watch_errors_reported = true;
                        app.notify(
                            Severity::Warning,
                            format!("Could not watch {} ({err}); use \"Refresh Explorer\"", d.display()),
                        );
                    }
                }
                Effect::Unwatch(d) => self.watcher.unwatch(&d),
                Effect::Write(bytes) => write_raw(&bytes),
                Effect::Index(cmd) => {
                    let _ = self.index.send(cmd);
                }
                Effect::Search(job) => self.search.start(*job, self.tx.clone()),
                Effect::SearchCancel => self.search.cancel(),
                Effect::Find(job) => {
                    let _ = self.find.send(*job);
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
                return Ok(());
            }
            let ev = match app.next_deadline() {
                Some(d) => {
                    let now = Instant::now();
                    if d <= now {
                        app.on_tick(now);
                        continue;
                    }
                    match self.rx.recv_timeout(d - now) {
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
