//! Headless test harness: drives `App` with synthetic events, runs effects
//! synchronously and renders to a ratatui `TestBackend`.

#![allow(dead_code)]

use std::{
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

use ratatui::{
    Terminal,
    backend::TestBackend,
    crossterm::event::{Event, KeyCode as Ct, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers},
};
use yace_app::{
    app::{App, AppEvent, AppOptions, Effect},
    explorer::read_dir_entries,
};
use yace_config::Paths;
use yace_keymap::{KeyCode, KeyPress, Mods};

pub struct Harness {
    pub app: App,
    pub state: tempfile::TempDir,
    pub term: Terminal<TestBackend>,
    /// The Quick Open index, run synchronously.
    pub engine: yace_app::workers::IndexEngine,
    /// When set, walks are queued in `pending_walks` instead of running.
    pub defer_walks: bool,
    pub pending_walks: Vec<yace_app::workers::WalkJob>,
    /// When set, background file loads are queued instead of completing.
    pub defer_loads: bool,
    pub pending_loads: Vec<AppEvent>,
}

impl Harness {
    pub fn new() -> Self {
        let state = tempfile::tempdir().unwrap();
        Self::with_state(state)
    }

    /// A harness whose user `config.toml` contains `toml`.
    pub fn with_config(toml: &str) -> Self {
        let state = tempfile::tempdir().unwrap();
        let cfg = state.path().join("config");
        std::fs::create_dir_all(&cfg).unwrap();
        std::fs::write(cfg.join("config.toml"), toml).unwrap();
        Self::with_state(state)
    }

    pub fn click(&mut self, x: u16, y: u16, mods: ratatui::crossterm::event::KeyModifiers) {
        use ratatui::crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
        for kind in [MouseEventKind::Down(MouseButton::Left), MouseEventKind::Up(MouseButton::Left)] {
            self.app.handle(AppEvent::Terminal(Event::Mouse(MouseEvent { kind, column: x, row: y, modifiers: mods })));
        }
        self.settle();
    }

    pub fn mouse(&mut self, kind: ratatui::crossterm::event::MouseEventKind, x: u16, y: u16) {
        use ratatui::crossterm::event::MouseEvent;
        self.app.handle(AppEvent::Terminal(Event::Mouse(MouseEvent {
            kind,
            column: x,
            row: y,
            modifiers: KeyModifiers::NONE,
        })));
        self.settle();
    }

    pub fn with_state(state: tempfile::TempDir) -> Self {
        let paths = Paths::in_dir(state.path());
        let mut app = App::new(AppOptions {
            paths,
            system_clipboard: false,
            keyboard_enhanced: true,
            process_start: Instant::now(),
        });
        app.resize(120, 40);
        let term = Terminal::new(TestBackend::new(120, 40)).unwrap();
        Self {
            app,
            state,
            term,
            engine: Default::default(),
            defer_walks: false,
            pending_walks: Vec::new(),
            defer_loads: false,
            pending_loads: Vec::new(),
        }
    }

    pub fn paths(&self) -> Paths {
        Paths::in_dir(self.state.path())
    }

    /// Execute pending effects synchronously until quiescent.
    pub fn settle(&mut self) {
        use yace_app::workers::{self, IndexCmd};
        for _ in 0..200 {
            let effects = self.app.take_effects();
            if effects.is_empty() {
                return;
            }
            let mut events: Vec<AppEvent> = Vec::new();
            for e in effects {
                match e {
                    Effect::ReadDir(d) => {
                        let result = read_dir_entries(&d);
                        events.push(AppEvent::DirLoaded { dir: d, result });
                    }
                    Effect::Index(IndexCmd::Sync { roots, walk, cache_dir, max_age }) => {
                        for job in self.engine.sync(&roots, &walk, cache_dir, max_age) {
                            if self.defer_walks {
                                self.pending_walks.push(job);
                            } else {
                                self.run_walk(&job);
                            }
                        }
                        events.push(AppEvent::IndexProgress {
                            files: self.engine.index.total_files(),
                            complete: !self.engine.is_walking(),
                        });
                    }
                    Effect::Index(IndexCmd::Query { seq, query, ctx, limit }) => {
                        events.push(self.engine.query(seq, &query, &ctx, limit))
                    }
                    Effect::Index(IndexCmd::FsChange { root, rel, exists, is_dir }) => {
                        self.engine.fs_change(root, &rel, exists, is_dir)
                    }
                    Effect::Index(_) => {}
                    Effect::Search(job) => {
                        let out = std::sync::Mutex::new(Vec::new());
                        workers::run_search(&job, &std::sync::atomic::AtomicBool::new(false), &|e| {
                            out.lock().unwrap().push(e)
                        });
                        events.extend(out.into_inner().unwrap());
                    }
                    Effect::Find(job) => events.push(workers::run_find(&job)),
                    Effect::LoadFile { doc, path, eol } => {
                        let ev = workers::load_file(doc, path, eol);
                        if self.defer_loads {
                            self.pending_loads.push(ev);
                        } else {
                            events.push(ev);
                        }
                    }
                    Effect::SearchCancel | Effect::Watch(_) | Effect::Unwatch(_) | Effect::Write(_) => {}
                }
            }
            for ev in events {
                self.app.handle(ev);
            }
        }
    }

    /// Run a walk to completion, feeding the engine.
    pub fn run_walk(&mut self, job: &yace_app::workers::WalkJob) {
        let (files, summary) = yace_search::walk::collect(&job.path, &job.walk);
        self.engine.batch(job.root, files);
        self.engine.walk_done(job.root, summary.truncated);
    }

    /// Complete deferred walks one batch at a time (simulates streaming).
    pub fn feed_walk_batch(&mut self, root: u32, files: &[&str]) {
        self.engine.batch(root, files.iter().map(|f| (*f).into()).collect());
        self.app.handle(AppEvent::IndexProgress { files: self.engine.index.total_files(), complete: false });
        self.settle();
    }

    pub fn key(&mut self, spec: &str) {
        for part in spec.split_whitespace() {
            let kp: KeyPress = part.parse().unwrap();
            self.app.handle(AppEvent::Terminal(Event::Key(to_crossterm(kp))));
            self.settle();
        }
    }

    pub fn type_text(&mut self, s: &str) {
        for c in s.chars() {
            let ev = KeyEvent {
                code: Ct::Char(c),
                modifiers: KeyModifiers::NONE,
                kind: KeyEventKind::Press,
                state: KeyEventState::NONE,
            };
            self.app.handle(AppEvent::Terminal(Event::Key(ev)));
        }
        self.settle();
    }

    pub fn command(&mut self, id: &str) {
        self.app.execute(id, None);
        self.settle();
    }

    /// Simulate filesystem events and let the coalescing window elapse.
    pub fn fs_changed(&mut self, paths: Vec<PathBuf>) {
        self.app.handle(AppEvent::FsChanged(paths));
        self.app.on_tick(Instant::now() + Duration::from_secs(1));
        self.settle();
    }

    pub fn screen(&mut self) -> String {
        let app = &self.app;
        self.term.draw(|f| yace_app::ui::draw(app, f)).unwrap();
        let buf = self.term.backend().buffer().clone();
        let mut out = String::new();
        for y in 0..buf.area.height {
            let mut skip = 0;
            for x in 0..buf.area.width {
                if skip > 0 {
                    skip -= 1;
                    continue;
                }
                let sym = buf[(x, y)].symbol();
                skip = unicode_width::UnicodeWidthStr::width(sym).saturating_sub(1);
                out.push_str(sym);
            }
            out.push('\n');
        }
        out
    }

    pub fn active_text(&self) -> String {
        let (_, _, d) = self.app.active().expect("active editor");
        self.app.docs.doc(d).unwrap().text()
    }

    pub fn explorer_labels(&self) -> Vec<String> {
        self.app.explorer.rows.iter().map(|r| format!("{}{}", "  ".repeat(r.depth), r.label)).collect()
    }

    pub fn select_explorer(&mut self, path: &Path) {
        self.app.explorer.reveal(path);
        self.settle();
        assert!(self.app.explorer.select_path(path), "explorer row for {}", path.display());
    }
}

pub fn to_crossterm(kp: KeyPress) -> KeyEvent {
    let mut m = KeyModifiers::NONE;
    if kp.mods.contains(Mods::CTRL) {
        m |= KeyModifiers::CONTROL;
    }
    if kp.mods.contains(Mods::SHIFT) {
        m |= KeyModifiers::SHIFT;
    }
    if kp.mods.contains(Mods::ALT) {
        m |= KeyModifiers::ALT;
    }
    let code = match kp.code {
        KeyCode::Char(c) => Ct::Char(c),
        KeyCode::F(n) => Ct::F(n),
        KeyCode::Enter => Ct::Enter,
        KeyCode::Tab => Ct::Tab,
        KeyCode::Backspace => Ct::Backspace,
        KeyCode::Escape => Ct::Esc,
        KeyCode::Space => Ct::Char(' '),
        KeyCode::Up => Ct::Up,
        KeyCode::Down => Ct::Down,
        KeyCode::Left => Ct::Left,
        KeyCode::Right => Ct::Right,
        KeyCode::Home => Ct::Home,
        KeyCode::End => Ct::End,
        KeyCode::PageUp => Ct::PageUp,
        KeyCode::PageDown => Ct::PageDown,
        KeyCode::Insert => Ct::Insert,
        KeyCode::Delete => Ct::Delete,
    };
    KeyEvent { code, modifiers: m, kind: KeyEventKind::Press, state: KeyEventState::NONE }
}

/// Three repositories in unrelated locations plus a workspace file stored
/// elsewhere (spec §28.5).
pub struct MultiRoot {
    pub api: PathBuf,
    pub web: PathBuf,
    pub chat: PathBuf,
    pub ws_file: PathBuf,
    _dirs: Vec<tempfile::TempDir>,
}

pub fn multi_root() -> MultiRoot {
    // Different top-level trees: the system temp dir and the cargo target
    // tmp dir, so the roots share no common workspace parent.
    let d1 = tempfile::tempdir().unwrap();
    let d2 = tempfile::tempdir_in(env!("CARGO_TARGET_TMPDIR")).unwrap();
    let d3 = tempfile::tempdir().unwrap();
    let ws_dir = tempfile::tempdir_in(env!("CARGO_TARGET_TMPDIR")).unwrap();

    let api = d1.path().join("global node api");
    let web = d2.path().join("learnapp-nextjs");
    let chat = ws_dir.path().join("services").join("chat-api");
    for (root, files) in [
        (
            &api,
            vec![
                ("src/index.ts", "export const api = 1;\n"),
                ("src/jobs.service.ts", "class Jobs {}\n"),
                ("package.json", "{}\n"),
            ],
        ),
        (&web, vec![("src/index.ts", "export const web = 2;\n"), ("next.config.ts", "export default {};\n")]),
        (&chat, vec![("src/index.ts", "chat\n"), ("README.md", "# chat\n")]),
    ] {
        for (rel, content) in files {
            let p = root.join(rel);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(&p, content).unwrap();
        }
    }
    std::fs::create_dir_all(web.join(".vscode")).unwrap();
    std::fs::write(web.join(".vscode/settings.json"), "{ // web prefers 2\n \"editor.tabSize\": 2, }\n").unwrap();

    let ws_file = ws_dir.path().join("localized.code-workspace");
    let text = format!(
        r#"// Cursor workspace
{{
  "folders": [
    {{ "name": "Global API", "path": "{api}" }},
    {{ "name": "Learn App", "path": "{web}" }},
    {{ "name": "Chat API", "path": "./services/chat-api" }}, // relative to this file
    {{ "name": "Launcher", "path": "/definitely/missing/launcher" }},
  ],
  "settings": {{ "files.autoSave": "off", "editor.fontFamily": "ignored" }},
  "launch": {{ "configurations": [] }},
}}
"#,
        api = api.display(),
        web = web.display()
    );
    std::fs::write(&ws_file, text).unwrap();
    let _ = d3;
    MultiRoot { api, web, chat, ws_file, _dirs: vec![d1, d2, ws_dir] }
}
