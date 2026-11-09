//! Application state and event handling. `App` is terminal-agnostic: it
//! consumes [`AppEvent`]s, mutates state and emits [`Effect`]s (directory
//! reads, watch changes, raw terminal output) that the event loop performs.
//! Tests drive it directly with synthetic events.

mod completion;
mod edit;
pub mod explorer_ops;
mod files;
mod find;
mod folding;
mod git;
mod git_ops;
mod history;
mod keybindings;
mod live_reload;
mod lsp;
mod mouse;
mod plugin_manager;
mod plugins;
mod quick_open;
mod search_view;
mod session;
mod terminal;
mod theme;
mod workspace_ops;

use std::{
    collections::{BTreeSet, HashMap, HashSet},
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

use ratatui::{
    crossterm::event::{Event, KeyCode as CtKey, KeyEvent, KeyEventKind, KeyModifiers},
    layout::Rect,
};
use yace_config::{ConfigError, Layer, Level, Paths, Settings};
use yace_keymap::{ContextSet, KeyCode, KeyPress, Keymap, Mods, Resolution, Resolver, Source, ctx};
use yace_workspace::{RootId, Workspace, session::RecentList};

use crate::{
    actions::{CloseFlow, ConfirmAction, PickerAction, PromptAction},
    clipboard::Clipboard,
    commands::Registry,
    editor::{DocId, DocumentStore, GroupId, Layout},
    explorer::{Entry, Explorer},
    metrics::Metrics,
    overlay::{Overlay, OverlayOutcome},
    theme::{Glyphs, Theme},
};

pub use completion::{completion_details, completion_kind, rank_completions};
pub use edit::{Move, Op};
pub use find::FindField;
pub use git_ops::{RepoCmd, ScmGroup, ScmRow, ScmUi};
pub use mouse::MouseState;
pub use search_view::{Row as SearchRow, SearchField};
pub use terminal::{DEFAULT_PANEL_HEIGHT, PanelState, TermFindState, TermTab, snapshot as terminal_snapshot};

/// Inputs to the application.
/// Least time between two heap trims.
const TRIM_EVERY: Duration = Duration::from_secs(2);

#[derive(Debug)]
pub enum AppEvent {
    Terminal(Event),
    /// Background copies finished: each destination and its outcome.
    PathsCopied(Vec<(PathBuf, Result<(), String>)>),
    /// The file manager could not be started.
    OpenExternalFailed(String),
    DirLoaded {
        dir: PathBuf,
        result: Result<Vec<Entry>, String>,
    },
    FsChanged(Vec<PathBuf>),
    /// SIGTERM / SIGHUP: persist and exit without discarding anything.
    Terminate,
    /// The Quick Open index grew or finished.
    IndexProgress {
        files: usize,
        complete: bool,
    },
    QuickOpenResults {
        seq: u64,
        hits: Vec<yace_search::Hit>,
        indexed: usize,
        complete: bool,
        truncated: bool,
    },
    SearchFile {
        seq: u64,
        file: Box<yace_search::FileMatches>,
    },
    SearchDone {
        seq: u64,
        summary: yace_search::SearchSummary,
        error: Option<String>,
    },
    /// The folders the Quick Open index covers changed (watch them).
    IndexDirs {
        root: yace_search::RootKey,
        change: crate::workers::DirChange,
    },
    /// The watcher lost events (queue overflow): rescan everything.
    FsRescan,
    /// A folder could not be watched.
    WatchFailed {
        path: PathBuf,
        error: String,
    },
    /// A terminal search finished: the match found and the match count
    /// (with the 1-based number of the match).
    TermFindDone {
        seq: u64,
        session: yace_terminal::SessionId,
        found: Option<yace_terminal::TermMatch>,
        count: (usize, Option<usize>),
    },
    /// A live update of one file's results (`None`: no longer matches).
    SearchFileUpdate {
        seq: u64,
        path: PathBuf,
        file: Option<Box<yace_search::FileMatches>>,
    },
    FindDone {
        seq: u64,
        result: Result<yace_search::FindResult, String>,
    },
    FileLoaded {
        doc: DocId,
        path: PathBuf,
        result: Result<Box<yace_buffer::Document>, String>,
    },
    /// From a terminal session (output, title, exit, bell).
    TerminalNotice(yace_terminal::Notice),
    /// Plugin manifests were read (on a worker).
    PluginsDiscovered(Vec<yace_plugin_host::Discovered>),
    /// The plugin index was downloaded (Extensions: Browse Plugins…).
    PluginIndex(Result<yace_plugin_host::registry::Index, String>),
    /// A plugin from the index was downloaded and checked, ready to confirm.
    PluginDownloaded(Result<Box<yace_plugin_host::InstallPlan>, String>),
    /// A plugin job posted a message.
    Plugin(crate::plugins::PluginMsg),
    /// A git job finished.
    Git(Box<crate::git::GitResult>),
    /// A message from a language server.
    Lsp {
        server: u32,
        msg: Box<yace_lsp::Incoming>,
    },
    /// A background parse finished.
    Parsed {
        doc: DocId,
        job: u64,
        tree: Option<Box<yace_syntax::Tree>>,
        /// Time the parse took (feeds the sync/async prediction).
        cost: Duration,
    },
}

/// Side effects requested by the application.
#[derive(Debug, Clone)]
pub enum Effect {
    ReadDir(PathBuf),
    Watch(PathBuf),
    Unwatch(PathBuf),
    /// Raw bytes for the terminal (OSC 52 clipboard, title).
    Write(Vec<u8>),
    /// A command for the Quick Open index actor.
    Index(crate::workers::IndexCmd),
    /// Start a workspace search (cancels the previous one).
    Search(Box<crate::workers::SearchJob>),
    SearchCancel,
    /// Re-search some files of the shown results (runs in order, never
    /// cancels the main search).
    SearchUpdate(Box<crate::workers::SearchJob>),
    /// Search a terminal's scrollback.
    TermFind(Box<crate::workers::TermFindJob>),
    /// Find matches in a document snapshot.
    Find(Box<crate::workers::FindJob>),
    /// Load a (large) file in the background.
    LoadFile {
        doc: DocId,
        path: PathBuf,
        eol: yace_buffer::LineEnding,
    },
    /// Return freed heap memory to the OS (after background work).
    TrimMemory,
    /// Parse a document on the syntax worker.
    Parse(Box<crate::syntax::ParseJob>),
    /// Read plugin manifests in this folder (no code is loaded).
    DiscoverPlugins(PathBuf),
    /// Download the plugin index from this URL (only on request).
    FetchPluginIndex(String),
    /// Download a plugin from the index and plan installing it into the
    /// plugins folder; nothing is installed before the user confirms.
    DownloadPlugin {
        entry: Box<yace_plugin_host::registry::Entry>,
        plugins_dir: PathBuf,
    },
    /// Run git (on a git worker).
    Git(yace_git::Git, crate::git::GitJob),
    /// Copy files and folders in the background (Explorer paste).
    CopyPaths(Vec<(PathBuf, PathBuf)>),
    /// Open a folder in the system file manager.
    OpenExternal(PathBuf),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Focus {
    /// The sidebar (Explorer or Search view, see `sidebar_view`).
    Explorer,
    Editor,
    /// The find/replace bar of the active editor group.
    Find,
    /// The active terminal in the bottom panel.
    Terminal,
    /// The find bar of the terminal panel.
    TerminalFind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SidebarView {
    #[default]
    Explorer,
    Search,
    /// A plugin view (`App::plugins.active_view`).
    Plugin,
    /// Source Control.
    Git,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Severity {
    Info,
    Warning,
    Error,
}

#[derive(Debug, Clone)]
pub struct Notification {
    pub level: Severity,
    pub text: String,
    pub expires: Option<Instant>,
}

#[derive(Debug, Clone)]
pub struct ClosedTab {
    pub path: PathBuf,
    pub group: GroupId,
}

pub struct AppOptions {
    pub paths: Paths,
    pub system_clipboard: bool,
    pub keyboard_enhanced: bool,
    pub process_start: Instant,
    /// `--safe-mode` / `--no-plugins`: load no plugin code.
    pub safe_mode: bool,
    /// The colour depth the terminal reports (`theme::terminal_depth()`),
    /// used when `workbench.colorDepth` is `"auto"`.
    pub terminal_depth: crate::theme::Depth,
}

pub struct App {
    pub paths: Paths,
    user_layer: Layer,
    /// Last good per-root settings layers (kept when a file breaks).
    root_layers: HashMap<RootId, Layer>,
    /// Last good user keybindings.
    user_bindings: Vec<yace_keymap::BindingSpec>,
    /// Content hashes of loaded config files, so watcher events for
    /// unchanged files (our own writes) don't reload.
    config_hashes: HashMap<PathBuf, u64>,
    pub config_errors: Vec<ConfigError>,
    /// Settings for the window (user + workspace).
    pub settings: Settings,
    root_settings: HashMap<RootId, Settings>,
    pub keymap: Keymap,
    pub resolver: Resolver,
    pub commands: Registry,
    pub workspace: Workspace,
    pub explorer: Explorer,
    pub docs: DocumentStore,
    pub layout: Layout,
    pub focus: Focus,
    pub overlays: Vec<Overlay>,
    pub notifications: Vec<Notification>,
    pub notification_log: Vec<(Severity, String)>,
    pub sidebar_visible: bool,
    pub sidebar_width: u16,
    pub theme: Theme,
    pub glyphs: Glyphs,
    pub metrics: Metrics,
    pub clipboard: Clipboard,
    pub closed_tabs: Vec<ClosedTab>,
    pub recent: RecentList,
    pub recent_files: Vec<PathBuf>,
    pub close_flow: Option<CloseFlow>,
    pub quit: bool,
    pub keyboard_enhanced: bool,
    /// The terminal's colour depth (for `workbench.colorDepth = "auto"`).
    pub terminal_depth: crate::theme::Depth,
    effects: Vec<Effect>,
    /// Watched (canonical) directories for the Explorer, settings, Git and
    /// open files; `index_watch` has the Quick Open index's folders.
    ui_watched: BTreeSet<PathBuf>,
    /// The folders of the Quick Open index (watch counts: nested roots
    /// can share folders), per root, and the roots fully covered.
    pub(crate) index_watch: quick_open::IndexWatch,
    /// Terminal size.
    pub area: Rect,
    pub needs_render: bool,
    pub mouse: MouseState,
    autosave_deadline: Option<Instant>,
    pending_fs: HashSet<PathBuf>,
    fs_deadline: Option<Instant>,
    /// Scroll the active view to its cursor after this event.
    reveal_cursor: bool,
    /// Docs with an external-change dialog already shown.
    conflict_prompted: HashSet<DocId>,
    /// Session restore enabled for workspace switches.
    pub restore_sessions: bool,
    /// Set while "Save All" runs inside a close flow.
    suppress_flow: bool,
    pub sidebar_view: SidebarView,
    /// Quick Open state (results map, generation, options).
    pub quick_open: quick_open::QuickOpenState,
    /// In-file find state.
    pub find: find::FindState,
    /// Workspace search view state.
    pub search: search_view::SearchState,
    /// Back/forward navigation history.
    pub nav: history::NavHistory,
    last_trim: Option<Instant>,
    trim_due: Option<Instant>,
    /// The bottom panel and its terminals.
    pub panel: terminal::PanelState,
    /// Find in the terminal's scrollback.
    pub term_find: terminal::TermFindState,
    /// Per-document syntax trees and highlighting.
    pub syntax: crate::syntax::SyntaxStore,
    /// Alt+Z: word wrap toggled for this window (overrides the setting).
    pub word_wrap_override: Option<bool>,
    /// Language servers, synced documents and diagnostics.
    pub lsp: crate::lsp::LspState,
    /// Targets of the last locations / problems picker.
    pub nav_targets: Vec<lsp::NavTarget>,
    pub code_actions: Vec<(u32, yace_lsp::CodeAction)>,
    pub completion: Option<completion::Completion>,
    /// The details panel beside the completion list is shown.
    pub suggest_details: bool,
    pub hover: Option<completion::HoverPopup>,
    pub signature: Option<completion::SignaturePopup>,
    /// Quick suggestions fire when this passes (typing pause).
    completion_deadline: Option<Instant>,
    /// The completion request in flight: (seq, doc, word start).
    completion_req: Option<(u64, DocId, usize)>,
    signature_seq: u64,
    /// Ctrl+S waiting for format-on-save (saved when it answers or by then).
    format_save: Option<(DocId, Instant)>,
    /// Document symbols of the active document: (doc, version, symbols, server).
    pub doc_symbols: Option<(DocId, u64, Vec<yace_lsp::Symbol>, u32)>,
    /// Workspace symbol results for the current query: (seq, results).
    pub workspace_symbols: (u64, Vec<(yace_lsp::Symbol, u32)>),
    /// The theme picker's live preview.
    pub(crate) theme_preview: Option<theme::ThemePreview>,
    pub(crate) theme_key: Option<theme::ThemeKey>,
    /// VS Code themes listed by the import picker.
    pub(crate) vscode_themes: Vec<crate::theme::vscode::VsTheme>,
    /// User theme files listed by "Edit User Theme".
    pub(crate) user_theme_files: Vec<(String, PathBuf)>,
    /// Native plugins (spec §18).
    pub plugins: crate::plugins::PluginHost,
    /// Git repositories of the workspace (spec §15).
    pub git: crate::git::GitState,
    /// Source Control view state.
    pub scm: ScmUi,
    /// Plugins enabled / disabled for this workspace only (session state).
    pub plugin_ws_enabled: Vec<String>,
    pub plugin_ws_disabled: Vec<String>,
    /// Explorer badges from plugins: path key → (badge, colour token).
    pub plugin_decorations: HashMap<PathBuf, (String, String)>,
    /// Files copied or cut in the Explorer, for Paste.
    pub file_clipboard: Option<explorer_ops::FileClipboard>,
    /// Path → canonical path, for watches (filled once per path).
    pub(crate) canon_cache: HashMap<PathBuf, PathBuf>,
    /// Watched (canonical) directory → the paths it is shown under.
    pub(crate) watch_alias: HashMap<PathBuf, BTreeSet<PathBuf>>,
    /// Plugin ids listed by the last manager picker.
    pub(crate) plugin_pick: Vec<String>,
    /// A plugin install waiting for confirmation.
    pub(crate) pending_install: Option<yace_plugin_host::InstallPlan>,
    /// Where Extensions: Browse Plugins… reads the plugin index.
    pub plugin_index_url: String,
    /// The plugins listed by the last Browse Plugins picker.
    pub(crate) plugin_index: Vec<yace_plugin_host::registry::Entry>,
    /// A plugin index or plugin download is running.
    pub(crate) plugin_fetching: bool,
    /// Plugin grammars not loaded yet: language id → (plugin, declaration).
    pub(crate) pending_grammars: HashMap<String, (usize, yace_plugin_host::manifest::GrammarDecl)>,
    /// Languages of open documents already handled by `check_new_languages`.
    pub(crate) languages_seen: std::collections::HashSet<String>,
}

const FS_COALESCE: Duration = Duration::from_millis(100);

impl App {
    pub fn new(opts: AppOptions) -> Self {
        let (user_layer, mut config_errors) = yace_config::load_toml_layer(&opts.paths.user_settings(), Level::User);
        let (settings, errs) = yace_config::resolve(std::slice::from_ref(&user_layer));
        config_errors.extend(errs);
        // Loaded from settings below (one parse); blank until then.
        let theme = Theme::blank(crate::theme::ThemeKind::Dark);
        let glyphs = Glyphs::from_setting(settings.workbench.glyphs);
        let (recent, recent_err) = RecentList::load(&opts.paths.recent_file());
        let mut app = Self {
            user_layer,
            root_layers: HashMap::new(),
            user_bindings: Vec::new(),
            config_hashes: HashMap::new(),
            config_errors: Vec::new(),
            root_settings: HashMap::new(),
            keymap: Keymap::new(),
            resolver: Resolver::default(),
            commands: Registry::builtin(),
            workspace: Workspace::empty_untitled(opts.paths.state_dir.join("untitled").join("empty.code-workspace")),
            explorer: Explorer::new(&settings.files.exclude, settings.explorer.show_hidden),
            docs: DocumentStore::default(),
            layout: Layout::default(),
            focus: Focus::Explorer,
            overlays: Vec::new(),
            notifications: Vec::new(),
            notification_log: Vec::new(),
            sidebar_visible: true,
            sidebar_width: settings.workbench.sidebar_width,
            theme,
            glyphs,
            metrics: Metrics::new(opts.process_start),
            clipboard: Clipboard::new(opts.system_clipboard),
            closed_tabs: Vec::new(),
            recent,
            recent_files: Vec::new(),
            close_flow: None,
            quit: false,
            keyboard_enhanced: opts.keyboard_enhanced,
            terminal_depth: opts.terminal_depth,
            effects: Vec::new(),
            ui_watched: BTreeSet::new(),
            index_watch: quick_open::IndexWatch::new(),
            area: Rect::new(0, 0, 120, 40),
            needs_render: true,
            mouse: MouseState::default(),
            autosave_deadline: None,
            pending_fs: HashSet::new(),
            fs_deadline: None,
            reveal_cursor: false,
            conflict_prompted: HashSet::new(),
            restore_sessions: true,
            suppress_flow: false,
            sidebar_view: SidebarView::Explorer,
            quick_open: Default::default(),
            find: Default::default(),
            search: Default::default(),
            nav: Default::default(),
            last_trim: None,
            trim_due: None,
            panel: Default::default(),
            term_find: Default::default(),
            syntax: Default::default(),
            word_wrap_override: None,
            lsp: Default::default(),
            nav_targets: Vec::new(),
            code_actions: Vec::new(),
            completion: None,
            suggest_details: true,
            hover: None,
            signature: None,
            completion_deadline: None,
            completion_req: None,
            signature_seq: 0,
            format_save: None,
            doc_symbols: None,
            workspace_symbols: (0, Vec::new()),
            theme_preview: None,
            theme_key: None,
            vscode_themes: Vec::new(),
            user_theme_files: Vec::new(),
            plugins: Default::default(),
            git: Default::default(),
            scm: Default::default(),
            plugin_ws_enabled: Vec::new(),
            plugin_ws_disabled: Vec::new(),
            plugin_decorations: HashMap::new(),
            plugin_pick: Vec::new(),
            canon_cache: HashMap::new(),
            file_clipboard: None,
            watch_alias: HashMap::new(),
            pending_install: None,
            plugin_index_url: yace_plugin_host::registry::index_url(),
            plugin_index: Vec::new(),
            plugin_fetching: false,
            pending_grammars: HashMap::new(),
            languages_seen: Default::default(),
            settings,
            paths: opts.paths,
        };
        app.reload_keymap();
        config_errors.extend(app.load_theme_from_settings());
        app.report_config_errors(config_errors);
        app.plugins.safe_mode = opts.safe_mode;
        app.check_crash_markers();
        app.discover_plugins();
        if let Some(e) = recent_err {
            app.notify(Severity::Warning, e);
        }
        app
    }

    // ------------------------------------------------------------------
    // Effects and deadlines (used by the event loop)

    pub fn take_effects(&mut self) -> Vec<Effect> {
        for job in self.explorer.take_jobs() {
            self.effects.push(Effect::ReadDir(job.dir));
        }
        std::mem::take(&mut self.effects)
    }

    pub(crate) fn emit(&mut self, e: Effect) {
        self.effects.push(e);
    }

    /// Ask the runtime to trim the heap, at most every 2 s; a request
    /// that comes too soon is kept and done when the 2 s are up.
    pub(crate) fn request_trim(&mut self) {
        let now = Instant::now();
        match self.last_trim {
            Some(t) if now.duration_since(t) < TRIM_EVERY => {
                self.trim_due.get_or_insert(t + TRIM_EVERY);
            }
            _ => {
                self.last_trim = Some(now);
                self.trim_due = None;
                self.effects.push(Effect::TrimMemory);
            }
        }
    }

    /// The earliest moment `on_tick` has work to do.
    pub fn next_deadline(&self) -> Option<Instant> {
        [
            self.resolver.deadline(),
            self.fs_deadline,
            self.autosave_deadline,
            self.search.debounce,
            self.search.update_due,
            self.trim_due,
            self.notifications.iter().filter_map(|n| n.expires).min(),
            self.lsp_deadline(),
            self.git_deadline(),
            self.plugins.pending_deadline,
        ]
        .into_iter()
        .flatten()
        .min()
    }

    pub fn on_tick(&mut self, now: Instant) {
        if self.resolver.expire(now) {
            self.needs_render = true;
        }
        if self.fs_deadline.is_some_and(|d| now >= d) {
            self.fs_deadline = None;
            let paths: Vec<PathBuf> = self.pending_fs.drain().collect();
            self.process_fs_changes(paths);
        }
        if self.search.debounce.is_some_and(|d| now >= d) {
            self.search.debounce = None;
            self.run_search();
        }
        if self.trim_due.is_some_and(|d| now >= d) {
            self.request_trim();
        }
        self.search_track_edits();
        if self.search.update_due.is_some_and(|d| now >= d) {
            self.search.update_due = None;
            self.run_search_update();
        }
        if self.autosave_deadline.is_some_and(|d| now >= d) {
            self.autosave_deadline = None;
            self.autosave_all();
        }
        let before = self.notifications.len();
        self.notifications.retain(|n| n.expires.is_none_or(|e| e > now));
        if self.notifications.len() != before {
            self.needs_render = true;
        }
        self.lsp_on_tick(now);
        self.sync_documents();
        self.check_new_languages();
        self.deliver_plugin_events(now);
        self.git_pump(now);
        self.sync_watches();
    }

    // ------------------------------------------------------------------
    // Notifications

    pub fn notify(&mut self, level: Severity, text: impl Into<String>) {
        let text = text.into();
        tracing::info!(?level, "{text}");
        let ttl = match level {
            Severity::Info => Some(Duration::from_secs(4)),
            Severity::Warning => Some(Duration::from_secs(8)),
            Severity::Error => None,
        };
        self.notification_log.push((level, text.clone()));
        if self.notification_log.len() > 200 {
            self.notification_log.remove(0);
        }
        self.notifications.retain(|n| n.text != text);
        self.notifications.push(Notification { level, text, expires: ttl.map(|t| Instant::now() + t) });
        if self.notifications.len() > 5 {
            self.notifications.remove(0);
        }
        self.needs_render = true;
    }

    /// The notification history as a searchable picker (newest first).
    pub(crate) fn show_notification_list(&mut self) {
        if self.notification_log.is_empty() {
            self.notify(Severity::Info, "No notifications");
            return;
        }
        let items: Vec<crate::overlay::PickerItem> = self
            .notification_log
            .iter()
            .enumerate()
            .rev()
            .map(|(i, (l, t))| crate::overlay::PickerItem::new(t.clone(), "", format!("{l:?}").to_lowercase(), i))
            .collect();
        let mut p = crate::overlay::Picker::new(
            "Notifications — Enter copies the message",
            items,
            crate::actions::PickerAction::CopyNotification,
        );
        p.empty_text = "No matching notifications".into();
        self.open_overlay(Overlay::Picker(p));
    }

    /// A notification that stays until dismissed (one-time hints the user
    /// must be able to read, e.g. how to install a language server).
    pub fn notify_sticky(&mut self, level: Severity, text: impl Into<String>) {
        self.notify(level, text);
        if let Some(n) = self.notifications.last_mut() {
            n.expires = None;
        }
    }

    pub fn dismiss_notification(&mut self) {
        self.notifications.pop();
        self.needs_render = true;
    }

    pub(crate) fn report_config_errors(&mut self, errors: Vec<ConfigError>) {
        if errors.is_empty() {
            return;
        }
        let first = errors[0].to_string();
        let n = errors.len();
        self.config_errors.extend(errors);
        let msg = if n == 1 {
            format!("Settings problem: {first}")
        } else {
            format!("{n} settings problems; first: {first} (run \"Developer: Show Configuration Errors\")")
        };
        self.notify(Severity::Warning, msg);
    }

    // ------------------------------------------------------------------
    // Settings and keymap

    pub(crate) fn reload_keymap(&mut self) {
        let mut km = yace_keymap::default_keymap();
        let path = self.paths.keybindings();
        let (mut specs, mut errors) = yace_config::load_keybindings(&path);
        if errors.is_empty() {
            self.user_bindings = specs.clone();
        } else {
            // A broken file keeps the last good bindings (spec §17.4).
            specs = self.user_bindings.clone();
            for e in &mut errors {
                e.message.push_str(" (the previous keybindings stay in effect)");
            }
        }
        self.remember_config_file(&path);
        let plugin_specs = self.plugin_keybinding_specs();
        for e in km.apply_specs(&plugin_specs, Source::Plugin) {
            tracing::warn!("plugin keybinding: {e}");
        }
        let binding_errors = km.apply_specs(&specs, Source::User);
        errors.extend(binding_errors.into_iter().map(|e| ConfigError {
            origin: path.display().to_string(),
            key: None,
            message: e.to_string(),
        }));
        self.keymap = km;
        self.resolver.timeout = Duration::from_millis(self.settings.keyboard.chord_timeout_ms.max(200));
        self.report_config_errors(errors);
    }

    /// Recompute window and per-root settings.
    pub(crate) fn reload_settings(&mut self) {
        let old_git = (self.settings.git.enabled, self.settings.git.path.clone());
        let user_path = self.paths.user_settings();
        let (user_layer, mut errors) = yace_config::load_toml_layer(&user_path, Level::User);
        if errors.is_empty() {
            self.user_layer = user_layer;
        } else {
            // A file that doesn't parse keeps its last good values (§17.4).
            for e in &mut errors {
                e.message.push_str(" (the previous settings stay in effect)");
            }
        }
        self.remember_config_file(&user_path);
        let ws_layer = Layer::new(
            Level::Workspace,
            self.workspace.file().map(|p| p.display().to_string()).unwrap_or_else(|| "workspace".into()),
            yace_config::from_vscode_settings(&self.workspace.settings),
        );
        let (settings, e) = yace_config::resolve(&[self.user_layer.clone(), ws_layer.clone()]);
        errors.extend(e);
        self.settings = settings;
        self.root_settings.clear();
        let roots: Vec<(RootId, PathBuf, bool)> =
            self.workspace.roots().iter().map(|r| (r.id, r.path.clone(), r.is_available())).collect();
        for (id, path, available) in roots {
            if !available {
                continue;
            }
            let (mut layer, e) = yace_config::load_root_layer(&path);
            if e.is_empty() {
                match &layer {
                    Some(l) => {
                        self.root_layers.insert(id, l.clone());
                    }
                    None => {
                        self.root_layers.remove(&id);
                    }
                }
            } else {
                layer = self.root_layers.get(&id).cloned();
            }
            errors.extend(e);
            self.remember_config_file(&path.join(".vscode").join("settings.json"));
            if let Some(layer) = layer {
                let (s, e) = yace_config::resolve(&[self.user_layer.clone(), ws_layer.clone(), layer]);
                errors.extend(e);
                self.root_settings.insert(id, s);
            }
        }
        self.explorer.show_hidden = self.settings.explorer.show_hidden;
        self.explorer.set_excludes(&self.settings.files.exclude);
        self.glyphs = Glyphs::from_setting(self.settings.workbench.glyphs);
        errors.extend(self.load_theme_from_settings());
        self.plugin_event("configurationChanged", serde_json::json!({}));
        if old_git != (self.settings.git.enabled, self.settings.git.path.clone()) && !self.workspace.roots().is_empty()
        {
            self.git.missing = false;
            self.git_workspace_changed();
        }
        self.config_errors.clear();
        self.report_config_errors(errors);
        // Once Quick Open has been used, keep its index (and the folder
        // watches) current with the roots and settings.
        if self.index_watch.started {
            self.index_sync();
        }
        self.needs_render = true;
    }

    /// Effective settings for a file (root-level overrides applied).
    pub fn settings_for(&self, path: Option<&Path>) -> &Settings {
        path.and_then(|p| self.workspace.root_for_path(p))
            .and_then(|r| self.root_settings.get(&r.id))
            .unwrap_or(&self.settings)
    }

    // ------------------------------------------------------------------
    // Context for keybindings

    pub fn context(&self) -> ContextSet {
        let mut c = ContextSet::new();
        c.set(ctx::KEYBOARD_ENHANCED, self.keyboard_enhanced);
        c.set(ctx::NOTIFICATION_VISIBLE, !self.notifications.is_empty());
        c.set(ctx::MULTIPLE_EDITOR_GROUPS, self.layout.groups().len() > 1);
        c.set(ctx::FIND_WIDGET_VISIBLE, self.find.visible);
        c.set(ctx::PANEL_VISIBLE, self.panel.visible);
        c.set(ctx::TERMINAL_FIND_VISIBLE, self.term_find.visible && self.panel.visible);
        c.set(ctx::SUGGEST_WIDGET_VISIBLE, self.completion.is_some());
        c.set(ctx::PARAMETER_HINTS_VISIBLE, self.signature.is_some());
        c.set(ctx::EDITOR_HOVER_VISIBLE, self.hover.is_some());
        match self.focus {
            Focus::Explorer if self.sidebar_visible && self.sidebar_view == SidebarView::Search => {
                c.set(ctx::SEARCH_VIEW_FOCUS, true);
                c.set(ctx::SEARCH_INPUT_FOCUS, self.search.input_focused());
            }
            Focus::Explorer if self.sidebar_visible => {
                c.set(ctx::EXPLORER_FOCUS, true);
                c.set(ctx::EXPLORER_FILTER_ACTIVE, !self.explorer.filter.is_empty());
            }
            Focus::Find => {
                c.set(ctx::FIND_WIDGET_FOCUS, true);
            }
            Focus::Terminal => {
                c.set(ctx::TERMINAL_FOCUS, true);
            }
            Focus::TerminalFind => {
                c.set(ctx::TERMINAL_FIND_FOCUSED, true);
            }
            _ => {
                if let Some(v) = self.layout.active_group().active_view() {
                    c.set(ctx::EDITOR_FOCUS, true);
                    c.set(ctx::EDITOR_HAS_SELECTION, v.sel.has_non_empty());
                    c.set(ctx::EDITOR_HAS_MULTIPLE_SELECTIONS, v.sel.is_multi());
                }
            }
        }
        c
    }

    // ------------------------------------------------------------------
    // Event dispatch

    pub fn handle(&mut self, event: AppEvent) {
        let start = Instant::now();
        let is_input = matches!(event, AppEvent::Terminal(Event::Key(_) | Event::Mouse(_) | Event::Paste(_)));
        // Documents opened outside an event get their language first.
        self.sync_documents();
        let big_list = self.completion.as_ref().filter(|c| c.items.len() >= 1000).map(|c| c.seq);
        match event {
            AppEvent::Terminal(Event::Key(k)) => self.on_key(k),
            AppEvent::Terminal(Event::Mouse(m)) => self.on_mouse(m),
            AppEvent::Terminal(Event::Paste(s)) => self.on_paste(&s),
            AppEvent::Terminal(Event::Resize(w, h)) => self.resize(w, h),
            AppEvent::Terminal(Event::FocusGained) => {
                self.check_open_docs_on_disk();
                if self.settings.git.auto_refresh {
                    self.git_schedule_all(git::STATUS_DEBOUNCE);
                }
            }
            AppEvent::Terminal(Event::FocusLost) => {
                if self.settings.files.auto_save == yace_config::AutoSave::OnFocusChange {
                    self.autosave_all();
                }
            }
            AppEvent::DirLoaded { dir, result } => {
                self.explorer.on_dir_loaded(&dir, result);
            }
            AppEvent::PathsCopied(results) => self.on_paths_copied(results),
            AppEvent::OpenExternalFailed(e) => self.notify(Severity::Error, e),
            AppEvent::FsChanged(paths) => {
                self.pending_fs.extend(paths);
                if self.fs_deadline.is_none() {
                    self.fs_deadline = Some(Instant::now() + FS_COALESCE);
                }
                return;
            }
            AppEvent::Terminate => {
                self.save_session();
                self.quit = true;
            }
            AppEvent::IndexDirs { root, change } => {
                self.on_index_dirs(root, change);
                // Adding thousands of watches leaves freed buffers behind.
                self.request_trim();
            }
            AppEvent::FsRescan => self.fs_rescan(),
            AppEvent::WatchFailed { path, error } => self.watch_failed(&path, &error),
            AppEvent::IndexProgress { files, complete } => {
                self.on_index_progress(files, complete);
                if complete {
                    self.request_trim();
                }
            }
            AppEvent::QuickOpenResults { seq, hits, indexed, complete, truncated } => {
                self.on_quick_open_results(seq, hits, indexed, complete, truncated)
            }
            AppEvent::SearchFile { seq, file } => self.on_search_file(seq, *file),
            AppEvent::SearchFileUpdate { seq, path, file } => self.on_search_file_update(seq, path, file.map(|f| *f)),
            AppEvent::SearchDone { seq, summary, error } => {
                self.on_search_done(seq, summary, error);
                self.request_trim();
            }
            AppEvent::FindDone { seq, result } => self.on_find_done(seq, result),
            AppEvent::TermFindDone { seq, session, found, count } => self.on_term_find_done(seq, session, found, count),
            AppEvent::FileLoaded { doc, path, result } => self.on_file_loaded(doc, path, result),
            AppEvent::TerminalNotice(n) => self.on_terminal_notice(n),
            AppEvent::Lsp { server, msg } => self.on_lsp(server, *msg),
            AppEvent::PluginsDiscovered(found) => self.on_plugins_discovered(found),
            AppEvent::PluginIndex(result) => self.on_plugin_index(result),
            AppEvent::PluginDownloaded(result) => self.on_plugin_downloaded(result),
            AppEvent::Plugin(msg) => self.on_plugin_message(msg),
            AppEvent::Git(r) => self.on_git(*r),
            AppEvent::Parsed { doc, job, tree, cost } => {
                if let Some(job) = self.syntax.on_parsed(&self.docs, doc, job, tree.map(|t| *t), cost) {
                    self.emit(Effect::Parse(Box::new(job)));
                }
            }
        }
        self.sync_documents();
        self.sync_theme_preview();
        self.check_new_languages();
        self.observe_for_plugins();
        self.deliver_plugin_events(Instant::now());
        self.git_pump(Instant::now());
        self.refresh_find_if_stale();
        self.search_track_edits();
        self.ensure_active_started();
        self.sync_terminal_size();
        if self.reveal_cursor {
            self.reveal_cursor = false;
            self.scroll_active_to_cursor();
        }
        self.clamp_explorer_scroll();
        self.sync_watches();
        // A large completion list was dropped: give its memory back.
        if big_list.is_some() && self.completion.as_ref().map(|c| c.seq) != big_list {
            self.request_trim();
        }
        self.needs_render = true;
        if is_input {
            self.metrics.input.record(start.elapsed());
        }
    }

    pub fn resize(&mut self, w: u16, h: u16) {
        // The preferred sidebar width is kept; layout clamps it per frame.
        self.area = Rect::new(0, 0, w, h);
        self.reveal_cursor = true;
    }

    fn on_key(&mut self, k: KeyEvent) {
        if k.kind == KeyEventKind::Release {
            return;
        }
        let (key, ch) = convert_key(&k);
        let Some(key) = key else { return };

        if let Some(top) = self.overlays.last_mut() {
            let outcome = top.handle_key(key, ch);
            self.on_overlay_outcome(outcome);
            return;
        }

        if self.focus == Focus::Terminal && self.panel.visible {
            self.resolver.cancel();
            self.terminal_key(key, ch);
            return;
        }
        if self.focus == Focus::Explorer
            && self.sidebar_view == SidebarView::Plugin
            && !self.resolver.is_pending()
            && self.plugin_view_key(key)
        {
            return;
        }
        if self.focus == Focus::Explorer
            && self.sidebar_view == SidebarView::Git
            && self.sidebar_visible
            && !self.resolver.is_pending()
            && self.scm_key(key, ch)
        {
            return;
        }
        if self.focus == Focus::Editor
            && key.code == yace_keymap::KeyCode::Enter
            && key.mods.is_empty()
            && !self.resolver.is_pending()
            && self.active().is_some_and(|(_, _, d)| self.docs.is_read_only(d))
            && self.git_diff_jump()
        {
            return;
        }
        let ctx = self.context();
        match self.resolver.feed(&self.keymap, key, &ctx) {
            Resolution::Command { command, args } => {
                if let Err(e) = self.run_command(&command, args.as_ref()) {
                    let src = args.is_some().then(|| self.paths.keybindings().display().to_string());
                    self.report_command_error(&command, &e, src.as_deref());
                }
            }
            Resolution::Pending(_) => {}
            Resolution::InvalidChord(seq) => {
                self.notify(Severity::Info, format!("The key combination ({seq}) is not a command."));
            }
            Resolution::Unbound => {
                // Text inputs (find bar, search view) take editing keys.
                if self.focus == Focus::Find {
                    self.find_input_key(key, ch);
                } else if self.focus == Focus::TerminalFind {
                    self.terminal_find_input_key(key, ch);
                } else if self.focus == Focus::Explorer && self.sidebar_view == SidebarView::Search {
                    self.search_input_key(key, ch);
                } else {
                    self.on_unbound_key(key, ch);
                }
            }
        }
    }

    fn on_unbound_key(&mut self, key: KeyPress, ch: Option<char>) {
        let Some(c) = key.text_char(ch) else { return };
        match self.focus {
            Focus::Editor => self.type_char(c),
            Focus::Find | Focus::Terminal | Focus::TerminalFind => {}
            Focus::Explorer => {
                if !(c == ' ' && self.explorer.filter.is_empty()) {
                    self.explorer.filter.push(c);
                    self.explorer.rebuild_rows();
                    // Jump to the first match below the root.
                    let f = self.explorer.filter.to_lowercase();
                    if let Some(i) =
                        self.explorer.rows.iter().position(|r| r.depth > 0 && r.label.to_lowercase().contains(&f))
                    {
                        self.explorer.selected = i;
                    }
                }
            }
        }
    }

    fn on_paste(&mut self, text: &str) {
        if let Some(top) = self.overlays.last_mut() {
            top.paste(text);
            if matches!(top, Overlay::Picker(p) if p.kind == crate::overlay::PickerKind::Dynamic) {
                self.quick_open_query_changed();
            }
            return;
        }
        match self.focus {
            Focus::Editor => self.insert_text(text),
            Focus::Find => self.find_paste(text),
            Focus::Explorer if self.sidebar_view == SidebarView::Search => self.search_paste(text),
            Focus::Terminal => self.terminal_paste_text(text),
            Focus::TerminalFind => self.terminal_find_paste(text),
            Focus::Explorer => {}
        }
    }

    /// Run a command by id.
    /// Run a command and report a failure as a notification.
    pub fn execute(&mut self, id: &str, args: Option<&serde_json::Value>) {
        if let Err(e) = self.run_command(id, args) {
            self.report_command_error(id, &e, None);
        }
    }

    /// Run a command and return its result (spec §19.1).
    pub fn run_command(&mut self, id: &str, args: Option<&serde_json::Value>) -> crate::commands::CmdResult {
        use crate::commands::CommandError;
        self.needs_render = true;
        if let Some(kind) = self.commands.plugin_command(id).cloned() {
            self.run_plugin_command(id, kind, args.cloned());
            return Ok(());
        }
        let Some(cmd) = self.commands.get(id) else { return Err(CommandError::Unknown(id.to_string())) };
        let handler = cmd.handler;
        // Editor commands need an editor (a few work without one).
        const WITHOUT_EDITOR: &[&str] = &[
            "editor.action.marker.nextInFiles",
            "editor.action.marker.prevInFiles",
            "editor.action.hideHover",
            "editor.action.toggleWordWrap",
            "editor.action.insertSnippet",
        ];
        if id.starts_with("editor.") && !WITHOUT_EDITOR.contains(&id) && self.active().is_none() {
            return Err(CommandError::Unavailable("no editor is open".into()));
        }
        handler(self, args)
    }

    /// Show why a command didn't run; `source` names where its arguments
    /// came from (a keybinding file).
    pub(crate) fn report_command_error(&mut self, id: &str, e: &crate::commands::CommandError, source: Option<&str>) {
        use crate::commands::CommandError;
        let title = self.commands.get(id).map(|c| c.title).unwrap_or(id);
        let (severity, mut msg) = match e {
            CommandError::Unknown(_) => (Severity::Error, e.to_string()),
            CommandError::Unavailable(m) => (Severity::Info, format!("{title}: {m}")),
            CommandError::InvalidArgs(m) => (Severity::Error, format!("{title} ({id}): {m}")),
            CommandError::Failed(m) => (Severity::Error, format!("{title}: {m}")),
        };
        if let Some(src) = source
            && matches!(e, CommandError::InvalidArgs(_) | CommandError::Unknown(_))
        {
            msg.push_str(&format!(" — check this key binding's `args` in {src}"));
        }
        self.notify(severity, msg);
    }

    // ------------------------------------------------------------------
    // Overlays

    pub fn open_overlay(&mut self, o: Overlay) {
        self.resolver.cancel();
        self.overlays.push(o);
        self.needs_render = true;
    }

    fn on_overlay_outcome(&mut self, outcome: OverlayOutcome) {
        match outcome {
            OverlayOutcome::Consumed => {}
            OverlayOutcome::Cancel => {
                let closed = self.overlays.pop();
                if let Some(Overlay::Picker(p)) = &closed
                    && p.action == PickerAction::SelectTheme
                {
                    self.end_theme_preview(None);
                }
                if let Some(Overlay::Prompt(p)) = closed
                    && let PromptAction::SaveAs { doc } = p.action
                    && self.close_flow.as_ref().is_some_and(|f| f.saving == Some(doc))
                {
                    self.close_flow = None;
                }
            }
            OverlayOutcome::SubmitPrompt(text) => {
                let Some(Overlay::Prompt(p)) = self.overlays.last().cloned() else { return };
                match self.on_prompt_submit(p.action.clone(), text) {
                    Ok(()) => {
                        // The handler may have pushed a follow-up overlay;
                        // remove the prompt itself.
                        if let Some(i) =
                            self.overlays.iter().rposition(|o| matches!(o, Overlay::Prompt(q) if q.action == p.action))
                        {
                            self.overlays.remove(i);
                        }
                    }
                    Err(msg) => {
                        if let Some(Overlay::Prompt(q)) = self.overlays.last_mut() {
                            q.error = Some(msg);
                        }
                    }
                }
            }
            OverlayOutcome::Confirm(action) => {
                self.overlays.pop();
                self.on_confirm(action);
            }
            OverlayOutcome::Pick(data) => {
                let Some(Overlay::Picker(p)) = self.overlays.pop() else { return };
                self.on_pick(p.action, data);
            }
            OverlayOutcome::PickBeside(data) => {
                let Some(Overlay::Picker(p)) = self.overlays.pop() else { return };
                if p.action == PickerAction::QuickOpen && self.quick_open_is_file(data) {
                    let g = self.layout.next_group_or_split(true);
                    self.layout.active = g;
                }
                self.on_pick(p.action, data);
            }
            OverlayOutcome::QueryChanged => self.quick_open_query_changed(),
            OverlayOutcome::ToggleIgnored => self.quick_open_toggle_ignored(),
            OverlayOutcome::KeybindingEdit(edit) => self.on_keybinding_edit(edit),
        }
    }

    fn on_prompt_submit(&mut self, action: PromptAction, text: String) -> Result<(), String> {
        let text = text.trim().to_string();
        match action {
            PromptAction::RenameSymbol { doc, pos } => self.lsp_rename(doc, pos, &text),
            PromptAction::InstallPlugin => self.plugins_install_from(&text),
            PromptAction::GitCommit { toplevel, all } => self.git_commit_submit(toplevel, all, &text),
            PromptAction::GitCreateBranch { toplevel } => self.git_create_branch_submit(toplevel, &text),
            PromptAction::ImportThemeFile => {
                let path = yace_workspace::paths::expand_tilde(&text);
                if !path.is_file() {
                    return Err(format!("{} is not a file", path.display()));
                }
                self.import_theme_from(&path, "", "");
                Ok(())
            }
            PromptAction::NewFile { dir } => self.create_entry(&dir, &text, false),
            PromptAction::NewFolder { dir } => self.create_entry(&dir, &text, true),
            PromptAction::Rename { path } => self.rename_entry(&path, &text),
            PromptAction::Move { path } => self.move_entry(&path, &text),
            PromptAction::AddRoot => self.add_root_from_input(&text),
            PromptAction::LocateRoot(id) => self.locate_root(id, &text),
            PromptAction::RenameRoot(id) => {
                self.workspace.rename_root(id, Some(text));
                self.after_roots_changed();
                Ok(())
            }
            PromptAction::SaveAs { doc } => self.save_as(doc, &text),
            PromptAction::SaveWorkspaceAs => self.save_workspace_as(&text),
            PromptAction::OpenFolder => self.open_folder_from_input(&text),
            PromptAction::OpenWorkspaceFile => self.open_workspace_from_input(&text),
            PromptAction::OpenFile => self.open_file_from_input(&text),
            PromptAction::GotoLine => self.goto_line(&text),
            PromptAction::RenameTerminal(id) => {
                self.rename_terminal(id, &text);
                Ok(())
            }
        }
    }

    fn on_confirm(&mut self, action: ConfirmAction) {
        match action {
            ConfirmAction::Nothing => {}
            ConfirmAction::RestoreDefaultKeybindings => self.restore_default_keybindings(),
            ConfirmAction::InstallPlugin => self.plugins_install_confirmed(),
            ConfirmAction::GitCommitAll(top) => self.git_commit_prompt(top, true),
            ConfirmAction::UninstallPlugin(id) => self.uninstall_plugin(&id),
            ConfirmAction::TrustPlugin(dir) => self.trust_plugin(dir),
            ConfirmAction::Close(choice) => self.on_close_choice(choice),
            ConfirmAction::Trash(p) => self.trash_entry(&p),
            ConfirmAction::DeletePermanently(p) => self.delete_permanently(&p),
            ConfirmAction::OverwriteSave(doc) => {
                self.save_doc(doc, true);
            }
            ConfirmAction::ReloadFromDisk(doc) => {
                self.conflict_prompted.remove(&doc);
                self.reload_doc(doc);
            }
            ConfirmAction::KeepMine(doc) => {
                self.conflict_prompted.remove(&doc);
                if let Some(d) = self.docs.doc_mut(doc) {
                    d.disk_state = yace_buffer::DiskState::Conflicted;
                }
            }
            ConfirmAction::RemoveRoot(id) => self.remove_root(id),
            ConfirmAction::OpenLarge { path, preview } => self.open_file_confirmed(&path, preview, true),
            ConfirmAction::OverwriteExisting { doc, path } => self.save_as_confirmed(doc, &path),
            ConfirmAction::ReplaceInFiles => self.search_replace_all(),
            ConfirmAction::KillTerminal(id) => self.kill_terminal(id),
            ConfirmAction::KillTerminalsThen(after) => {
                self.kill_all_terminals();
                let docs = self.docs.ids();
                self.start_close(docs, *after);
            }
            ConfirmAction::CreateParentAndSave { doc, path } => {
                if let Some(parent) = path.parent()
                    && let Err(e) = std::fs::create_dir_all(parent)
                {
                    self.notify(Severity::Error, format!("Could not create {}: {e}", parent.display()));
                    return;
                }
                self.save_as_confirmed(doc, &path);
            }
        }
    }

    fn on_pick(&mut self, action: PickerAction, data: usize) {
        match action {
            PickerAction::RunCommand => {
                if let Some(id) = self.commands.palette_ids().get(data).copied() {
                    self.quick_open.note_command(id);
                    self.execute(id, None);
                }
            }
            PickerAction::QuickOpen => self.quick_open_pick(data),
            PickerAction::SelectTheme => self.end_theme_preview(Some(data)),
            PickerAction::ImportVsCodeTheme => self.import_vscode_theme_pick(data),
            PickerAction::EditUserTheme => self.edit_user_theme_pick(data),
            PickerAction::PluginOp(id) => self.plugin_op_pick(id, data),
            PickerAction::PluginView => self.plugin_view_pick(data),
            PickerAction::PluginBrowse => self.plugin_browse_pick(data),
            PickerAction::GitRepo(cmd) => self.git_run_repo_cmd(data, cmd),
            PickerAction::GitBranch => self.git_branch_pick(data),
            a @ (PickerAction::PluginList
            | PickerAction::PluginEnable
            | PickerAction::PluginDisable
            | PickerAction::PluginUninstall) => self.plugins_pick(a, data),
            PickerAction::NewTerminalInRoot => self.new_terminal_in_root(RootId(data as u32)),
            PickerAction::NewTerminalProfile => self.new_terminal_with_profile(data),
            PickerAction::OpenRecent => self.open_recent(data),
            PickerAction::Root(a) => self.root_action(a, RootId(data as u32)),
            PickerAction::Navigate => {
                if let Some(t) = self.nav_targets.get(data).cloned() {
                    self.jump_to(&t);
                }
            }
            PickerAction::CodeAction => self.pick_code_action(data),
            PickerAction::CopyNotification => {
                if let Some((_, text)) = self.notification_log.get(data).cloned() {
                    if let Some(bytes) = self.clipboard.set_text(text) {
                        self.emit(Effect::Write(bytes));
                    }
                    self.notify(Severity::Info, "Copied to the clipboard");
                }
            }
            PickerAction::ChangeEol => {
                self.change_eol(if data == 0 { yace_buffer::LineEnding::Lf } else { yace_buffer::LineEnding::CrLf })
            }
        }
    }

    // ------------------------------------------------------------------
    // Focus

    /// The icon to draw before a file name (colour adapted to the terminal).
    pub fn file_icon(&self, name: &str) -> Option<crate::icons::Icon> {
        let w = &self.settings.workbench;
        crate::icons::for_file(name, w.glyphs, w.file_icons)
            .map(|i| crate::icons::Icon { color: crate::theme::downgrade(i.color, self.theme.depth), ..i })
    }

    /// A Nerd Font file icon (tabs, special folders); `None` otherwise.
    pub fn nerd_icon(&self, name: &str, folder: bool) -> Option<crate::icons::Icon> {
        let w = &self.settings.workbench;
        if !w.file_icons || w.glyphs != yace_config::Glyphs::NerdFont {
            return None;
        }
        let i = if folder { crate::icons::folder_type(name) } else { crate::icons::file_type(name) }?;
        Some(crate::icons::Icon { color: crate::theme::downgrade(i.color, self.theme.depth), ..i })
    }

    pub fn focus_explorer(&mut self) {
        if !self.sidebar_visible {
            self.sidebar_visible = true;
        }
        self.focus_changed(Focus::Explorer);
    }

    pub fn focus_editor(&mut self) {
        self.focus_changed(Focus::Editor);
    }

    pub(crate) fn focus_changed(&mut self, to: Focus) {
        if self.focus != to && self.settings.files.auto_save == yace_config::AutoSave::OnFocusChange {
            self.autosave_all();
        }
        if self.focus != to {
            self.close_popups();
        }
        self.focus = to;
        self.resolver.cancel();
    }

    // ------------------------------------------------------------------
    // Watching (bounded, non-recursive; TODO.md D8)

    /// Watch every directory that matters, once per real directory: two
    /// paths to one folder (a symlink) share a watch, and events are
    /// reported under each path shown (spec §8.4).
    pub(crate) fn sync_watches(&mut self) {
        let mut wanted: BTreeSet<PathBuf> = self.explorer.expanded_dirs().into_iter().collect();
        wanted.extend(self.config_watch_dirs());
        wanted.extend(self.git_watch_dirs());
        for (_, e) in self.docs.iter() {
            if let Some(parent) = e.doc.path().and_then(Path::parent) {
                wanted.insert(parent.to_path_buf());
            }
        }
        if self.canon_cache.len() > 4096 {
            self.canon_cache.clear();
        }
        let mut want: BTreeSet<PathBuf> = BTreeSet::new();
        let mut alias: HashMap<PathBuf, BTreeSet<PathBuf>> = HashMap::new();
        for p in wanted {
            let c = self
                .canon_cache
                .entry(p.clone())
                .or_insert_with(|| std::fs::canonicalize(&p).unwrap_or_else(|_| p.clone()))
                .clone();
            alias.entry(c.clone()).or_default().insert(p);
            want.insert(c);
        }
        // Index folders under a symlinked root are watched under its real
        // path; report their events under the root's path too.
        for (canon, shown) in self.index_watch.root_aliases() {
            alias.entry(canon.clone()).or_default().insert(shown.clone());
        }
        self.watch_alias = alias;
        let remove: Vec<PathBuf> = self.ui_watched.difference(&want).cloned().collect();
        let add: Vec<PathBuf> = want.difference(&self.ui_watched).cloned().collect();
        self.ui_watched = want;
        for p in remove {
            if !self.index_watch.contains(&p) {
                self.effects.push(Effect::Unwatch(p));
            }
        }
        for p in add {
            if !self.index_watch.contains(&p) {
                self.effects.push(Effect::Watch(p));
            }
        }
    }

    /// The index started needing `p` (already counted in `index_watch`).
    pub(crate) fn index_watched(&mut self, p: PathBuf) {
        if !self.ui_watched.contains(&p) {
            self.effects.push(Effect::Watch(p));
        }
    }

    /// The index no longer needs `p`.
    pub(crate) fn index_unwatched(&mut self, p: PathBuf) {
        if !self.ui_watched.contains(&p) {
            self.effects.push(Effect::Unwatch(p));
        }
    }

    pub(crate) fn ui_watched_len(&self) -> usize {
        self.ui_watched.len()
    }

    /// Every watched directory (diagnostics and tests).
    pub fn watched_dirs(&self) -> BTreeSet<PathBuf> {
        self.ui_watched.iter().cloned().chain(self.index_watch.paths()).collect()
    }

    fn clamp_explorer_scroll(&mut self) {
        let height = crate::ui::explorer_list_height(self) as usize;
        let ex = &mut self.explorer;
        if height == 0 {
            return;
        }
        if ex.selected < ex.scroll {
            ex.scroll = ex.selected;
        } else if ex.selected >= ex.scroll + height {
            ex.scroll = ex.selected + 1 - height;
        }
        ex.scroll = ex.scroll.min(ex.rows.len().saturating_sub(height));
    }

    /// Schedule auto-save after an edit (setting `files.autoSave`).
    /// Feed document edits to incremental consumers (syntax; later LSP).
    pub fn sync_documents(&mut self) {
        let out = self.syntax.sync(&mut self.docs, &self.settings.files.associations);
        for job in out.jobs {
            self.effects.push(Effect::Parse(Box::new(job)));
        }
        if !out.edits.is_empty() {
            self.find_follow_edits(&out.edits);
        }
        if !(out.attached.is_empty() && out.removed.is_empty() && out.edits.is_empty()) {
            self.lsp_sync(&out.attached, &out.removed, &out.edits);
        }
    }

    pub(crate) fn on_document_edited(&mut self) {
        if self.settings.files.auto_save == yace_config::AutoSave::AfterDelay {
            self.autosave_deadline =
                Some(Instant::now() + Duration::from_millis(self.settings.files.auto_save_delay.max(100)));
        }
    }
}

/// Convert a crossterm key to a normalised YACE key plus the typed char.
pub fn convert_key(k: &KeyEvent) -> (Option<KeyPress>, Option<char>) {
    let mut mods = Mods::NONE;
    if k.modifiers.contains(KeyModifiers::CONTROL) {
        mods = mods | Mods::CTRL;
    }
    if k.modifiers.contains(KeyModifiers::SHIFT) {
        mods = mods | Mods::SHIFT;
    }
    if k.modifiers.contains(KeyModifiers::ALT) {
        mods = mods | Mods::ALT;
    }
    if k.modifiers.contains(KeyModifiers::SUPER) || k.modifiers.contains(KeyModifiers::META) {
        mods = mods | Mods::META;
    }
    let (code, ch) = match k.code {
        CtKey::Char(c) => (KeyCode::Char(c), Some(c)),
        CtKey::F(n) => (KeyCode::F(n), None),
        CtKey::Enter => (KeyCode::Enter, None),
        CtKey::Tab => (KeyCode::Tab, None),
        CtKey::BackTab => {
            mods = mods | Mods::SHIFT;
            (KeyCode::Tab, None)
        }
        CtKey::Backspace => (KeyCode::Backspace, None),
        CtKey::Esc => (KeyCode::Escape, None),
        CtKey::Up => (KeyCode::Up, None),
        CtKey::Down => (KeyCode::Down, None),
        CtKey::Left => (KeyCode::Left, None),
        CtKey::Right => (KeyCode::Right, None),
        CtKey::Home => (KeyCode::Home, None),
        CtKey::End => (KeyCode::End, None),
        CtKey::PageUp => (KeyCode::PageUp, None),
        CtKey::PageDown => (KeyCode::PageDown, None),
        CtKey::Insert => (KeyCode::Insert, None),
        CtKey::Delete => (KeyCode::Delete, None),
        _ => return (None, None),
    };
    (Some(KeyPress::normalized(code, mods)), ch)
}
