//! Native plugins in the editor (spec §18): per-plugin state and the host
//! function table plugins call (`yace_plugin_api::YaceHost`).
//!
//! All of the editor's `unsafe` plugin code is in this module. The rules:
//! - Every pointer, length and string a plugin passes is checked (null,
//!   size limits, UTF-8, JSON) before use; bad input returns an error code.
//! - Host functions reach the `App` through a pointer that is set only
//!   while the editor is calling into that plugin (on the UI thread), and
//!   the plugin's own `Loaded` table is moved out of the `App` for the
//!   duration of the call, so host functions never alias it.
//! - `post_message` and `job_cancelled` touch only thread-safe state.
//! - No host function unwinds into plugin code (panics are caught).

#![allow(unsafe_code)]

use std::{
    cell::{Cell, RefCell},
    collections::{BTreeMap, HashMap, HashSet, VecDeque},
    ffi::c_void,
    panic::AssertUnwindSafe,
    path::PathBuf,
    sync::Arc,
    time::Duration,
};

use crossbeam_channel::{Receiver, Sender, unbounded};
use serde::Deserialize;
use serde_json::Value;
use yace_plugin_api::{
    ABI_VERSION, YACE_E_DENIED, YACE_E_FAILED, YACE_E_INVALID, YACE_E_NOT_FOUND, YACE_E_UNSUPPORTED, YACE_OK, YaceBuf,
    YaceHandle, YaceHost, YaceHostCtx, YaceJobFn, YaceStatus, YaceStr,
};
use yace_plugin_host::{Jobs, Loaded, Manifest};

use crate::app::App;

/// Longest string accepted from a plugin (16 MiB).
pub const MAX_STR: usize = 16 << 20;
/// Lines kept per plugin log.
pub const LOG_LINES: usize = 500;

/// A message from a plugin's worker thread to its `on_message`.
#[derive(Debug, Clone, PartialEq)]
pub struct PluginMsg {
    pub plugin: usize,
    pub data: String,
}

/// Thread-safe part of a plugin's host context.
pub struct SharedCtx {
    pub index: usize,
    pub tx: Sender<PluginMsg>,
    pub jobs: Jobs,
}

/// What `YaceHost::ctx` points to (one per plugin, boxed so it never moves).
pub struct HostCtx {
    pub shared: Arc<SharedCtx>,
    /// The app, only while the editor is calling into this plugin.
    pub app: Cell<*mut App>,
    pub last_error: RefCell<String>,
    /// Backing store for the string `last_error` returns.
    pub error_out: RefCell<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum PluginState {
    /// The manifest could not be read or is invalid (see `problems`).
    Invalid,
    /// Built for another OS/architecture/YACE/ABI.
    Incompatible(String),
    /// Disabled by the user (globally or in this workspace).
    Disabled,
    /// Its code is not approved (not installed through YACE, or changed).
    Untrusted(String),
    /// Contributions registered; code not loaded yet.
    Inactive,
    Active,
    /// Loading or activation failed, or it was stopped after errors.
    Failed(String),
}

impl PluginState {
    pub fn label(&self) -> String {
        match self {
            PluginState::Invalid => "invalid".into(),
            PluginState::Incompatible(why) => format!("incompatible: {why}"),
            PluginState::Disabled => "disabled".into(),
            PluginState::Untrusted(why) => format!("not loaded: {why}"),
            PluginState::Inactive => "inactive (loads on first use)".into(),
            PluginState::Active => "active".into(),
            PluginState::Failed(why) => format!("failed: {why}"),
        }
    }
    /// Contributions (commands, themes, …) are offered in these states.
    pub fn contributes(&self) -> bool {
        matches!(self, PluginState::Inactive | PluginState::Active | PluginState::Untrusted(_) | PluginState::Failed(_))
    }
}

/// A status bar item set by a plugin.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
pub struct StatusContent {
    pub text: String,
    #[serde(default)]
    pub tooltip: String,
    #[serde(default)]
    pub command: String,
    #[serde(default)]
    pub color: String,
}

/// A node of a plugin view.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
pub struct ViewNode {
    pub id: String,
    pub label: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub icon: String,
    #[serde(default)]
    pub color: String,
    #[serde(default)]
    pub command: String,
    #[serde(default)]
    pub args: Option<Value>,
    #[serde(default)]
    pub children: Vec<ViewNode>,
    #[serde(default)]
    pub expanded: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
pub struct DecorationIn {
    pub path: String,
    pub badge: String,
    #[serde(default)]
    pub color: String,
    #[serde(default)]
    pub tooltip: String,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
pub struct QuickOpenIn {
    pub label: String,
    #[serde(default)]
    pub description: String,
    pub command: String,
    #[serde(default)]
    pub args: Option<Value>,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
pub struct TerminalIn {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub cwd: String,
    #[serde(default)]
    pub root: Option<u64>,
    #[serde(default)]
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct EditIn {
    pub start: u64,
    pub end: u64,
    pub text: String,
}

/// One installed (or found) plugin.
pub struct PluginEntry {
    pub folder: String,
    pub dir: PathBuf,
    pub manifest: Option<Manifest>,
    pub problems: Vec<String>,
    pub state: PluginState,
    /// Taken out while the editor calls into the plugin.
    pub loaded: Option<Loaded>,
    pub ctx: Box<HostCtx>,
    pub activation_time: Option<Duration>,
    pub status: BTreeMap<String, StatusContent>,
    pub views: BTreeMap<String, Vec<ViewNode>>,
    /// Root id → decorations.
    pub decorations: BTreeMap<u64, Vec<DecorationIn>>,
    pub log: VecDeque<String>,
    /// Failed callbacks (errors and panics) this session.
    pub failures: u32,
    /// Already warned about a slow callback.
    pub slow_warned: bool,
}

impl PluginEntry {
    pub fn id(&self) -> &str {
        self.manifest.as_ref().map(|m| m.id.as_str()).unwrap_or(&self.folder)
    }
    pub fn name(&self) -> &str {
        self.manifest.as_ref().map(|m| m.name.as_str()).unwrap_or(&self.folder)
    }
    pub fn push_log(&mut self, line: String) {
        if self.log.len() >= LOG_LINES {
            self.log.pop_front();
        }
        self.log.push_back(line);
    }
}

/// UI state of a plugin view (selection, expansion, scroll).
#[derive(Debug, Clone, Default)]
pub struct ViewUi {
    pub selected: usize,
    pub scroll: usize,
    pub collapsed: HashSet<String>,
    pub expanded: HashSet<String>,
}

/// A view the user can open: (plugin index, view id, title, location).
#[derive(Debug, Clone, PartialEq)]
pub struct ViewRef {
    pub plugin: usize,
    pub id: String,
    pub title: String,
}

pub struct PluginHost {
    pub entries: Vec<PluginEntry>,
    pub jobs: Jobs,
    tx: Sender<PluginMsg>,
    rx: Receiver<PluginMsg>,
    /// `--safe-mode`: no plugin code or grammar is loaded.
    pub safe_mode: bool,
    /// Discovery finished (contributions registered).
    pub discovered: bool,
    /// Commands plugins asked to run, after the current callback.
    pub deferred: Vec<(String, Option<Value>)>,
    /// Events waiting for delivery: (event type, JSON).
    pub queue: Vec<(String, Value)>,
    /// Coalesced high-frequency events, delivered after a short delay.
    pub pending: BTreeMap<String, Value>,
    pub pending_deadline: Option<std::time::Instant>,
    /// The open Quick Open request: (request id, plugin, provider prefix).
    pub quick_open: Option<(u64, usize, String)>,
    pub next_request: u64,
    pub view_ui: HashMap<(usize, String), ViewUi>,
    /// The view shown in the sidebar.
    pub active_view: Option<(usize, String)>,
    /// Plugins whose activation was triggered, in order (crash marker).
    pub activated: Vec<String>,
    /// Snapshot for detecting changes worth an event.
    pub observed: Observed,
}

/// State compared after every input to produce events (spec §18.4).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Observed {
    pub active: Option<(u32, u64)>,
    pub selection: Option<(u32, usize, usize)>,
    pub roots: Vec<u32>,
    pub theme: String,
    pub terminals: Vec<u32>,
    pub languages: Vec<String>,
    pub docs: Vec<u32>,
}

impl Default for PluginHost {
    fn default() -> Self {
        let (tx, rx) = unbounded();
        Self {
            entries: Vec::new(),
            jobs: Jobs::default(),
            tx,
            rx,
            safe_mode: false,
            discovered: false,
            deferred: Vec::new(),
            queue: Vec::new(),
            pending: BTreeMap::new(),
            pending_deadline: None,
            quick_open: None,
            next_request: 0,
            view_ui: HashMap::new(),
            active_view: None,
            activated: Vec::new(),
            observed: Observed::default(),
        }
    }
}

impl PluginHost {
    /// A context for plugin `index`.
    pub fn new_ctx(&self, index: usize) -> Box<HostCtx> {
        Box::new(HostCtx {
            shared: Arc::new(SharedCtx { index, tx: self.tx.clone(), jobs: self.jobs.clone() }),
            app: Cell::new(std::ptr::null_mut()),
            last_error: RefCell::new(String::new()),
            error_out: RefCell::new(String::new()),
        })
    }

    /// The receiving end for the runtime's forwarder thread.
    pub fn messages(&self) -> Receiver<PluginMsg> {
        self.rx.clone()
    }

    pub fn try_messages(&self) -> Vec<PluginMsg> {
        self.rx.try_iter().collect()
    }

    pub fn find(&self, id: &str) -> Option<usize> {
        self.entries.iter().position(|e| e.id() == id)
    }

    /// Every view of plugins that contribute.
    pub fn views(&self) -> Vec<ViewRef> {
        let mut out = Vec::new();
        for (i, e) in self.entries.iter().enumerate() {
            if !e.state.contributes() {
                continue;
            }
            if let Some(m) = &e.manifest {
                for v in &m.views {
                    out.push(ViewRef { plugin: i, id: v.id.clone(), title: v.title.clone() });
                }
            }
        }
        out
    }

    /// Does any active plugin want this event?
    pub fn wants(&self, event: &str) -> bool {
        self.entries.iter().any(|e| {
            e.manifest.as_ref().is_some_and(|m| {
                (e.state == PluginState::Active && m.events.iter().any(|x| x == event))
                    || (e.state == PluginState::Inactive
                        && m.activation.iter().any(|a| a.strip_prefix("onEvent:") == Some(event)))
            })
        })
    }
}

/// The host function table for a plugin context.
pub fn host_table(ctx: &HostCtx) -> Box<YaceHost> {
    Box::new(YaceHost {
        size: std::mem::size_of::<YaceHost>() as u32,
        abi_version: ABI_VERSION,
        ctx: ctx as *const HostCtx as *mut YaceHostCtx,
        log: h_log,
        notify: h_notify,
        last_error: h_last_error,
        free: h_free,
        get_setting: h_get_setting,
        workspace_roots: h_workspace_roots,
        active_document: h_active_document,
        document_text: h_document_text,
        apply_edits: h_apply_edits,
        open_file: h_open_file,
        execute_command: h_execute_command,
        set_status_item: h_set_status_item,
        set_view_items: h_set_view_items,
        set_decorations: h_set_decorations,
        quick_open_results: h_quick_open_results,
        create_terminal: h_create_terminal,
        spawn_job: h_spawn_job,
        job_cancelled: h_job_cancelled,
        post_message: h_post_message,
    })
}

/// Call into plugin `index` with the app reachable from host functions.
/// The plugin's `Loaded` is moved out for the call (so host functions can
/// use the whole `App`), then put back. Returns `None` if the plugin is not
/// loaded or already in a call (re-entrancy).
pub fn call<R>(app: &mut App, index: usize, f: impl FnOnce(&mut Loaded) -> R) -> Option<R> {
    let mut loaded = app.plugins.entries.get_mut(index)?.loaded.take()?;
    let app_ptr: *mut App = app;
    // The context is boxed and owned by the entry; the pointer stays valid
    // while the entry exists, and entries are never removed during a call.
    let ctx: *const HostCtx = &*app.plugins.entries[index].ctx;
    // SAFETY: `ctx` points into a live Box (see above).
    unsafe { (*ctx).app.set(app_ptr) };
    // From here until the end of the call, `app` is only reached through
    // `app_ptr` (by host functions); `loaded` is a local.
    let r = f(&mut loaded);
    // SAFETY: as above.
    unsafe { (*ctx).app.set(std::ptr::null_mut()) };
    if let Some(e) = app.plugins.entries.get_mut(index) {
        e.loaded = Some(loaded);
    }
    Some(r)
}

/// A tree-sitter language from a grammar library's exported function.
pub fn grammar_language(f: yace_plugin_host::GrammarFn) -> yace_syntax::Language {
    // SAFETY: `f` is a grammar's `tree_sitter_<lang>` function from a
    // library whose hash the user approved; the language's ABI version is
    // checked by `yace_syntax::register_grammar` before use.
    let lf = unsafe { yace_syntax::LanguageFn::from_raw(f) };
    yace_syntax::Language::new(lf)
}

// ---------------------------------------------------------------------------
// Host functions. Each returns an error code for bad input, never unwinds.

fn ctx_of<'a>(c: *mut YaceHostCtx) -> Option<&'a HostCtx> {
    // SAFETY: `c` is the pointer we put in the table (or null).
    (!c.is_null()).then(|| unsafe { &*(c as *const HostCtx) })
}

fn str_of(v: YaceStr) -> Result<String, &'static str> {
    if v.len == 0 {
        return Ok(String::new());
    }
    if v.ptr.is_null() {
        return Err("null string");
    }
    if v.len > MAX_STR {
        return Err("string too long");
    }
    // SAFETY: the plugin promises `len` readable bytes for the call (ABI);
    // the length was bounded above.
    let bytes = unsafe { std::slice::from_raw_parts(v.ptr, v.len) };
    std::str::from_utf8(bytes).map(str::to_string).map_err(|_| "invalid UTF-8")
}

fn set_error(c: &HostCtx, msg: impl Into<String>) {
    *c.last_error.borrow_mut() = msg.into();
}

fn fail(c: &HostCtx, status: YaceStatus, msg: impl Into<String>) -> YaceStatus {
    set_error(c, msg);
    status
}

fn fill(c: &HostCtx, out: *mut YaceBuf, data: String) -> YaceStatus {
    if out.is_null() {
        return fail(c, YACE_E_INVALID, "null output buffer");
    }
    let mut bytes = data.into_bytes().into_boxed_slice();
    let (ptr, len) = (bytes.as_mut_ptr(), bytes.len());
    std::mem::forget(bytes);
    // SAFETY: checked non-null; the plugin owns `*out` and frees it with
    // `free`, which rebuilds this boxed slice.
    unsafe { *out = YaceBuf { ptr, len, cap: len } };
    YACE_OK
}

/// Run a host function body with the app, catching panics.
fn with_app(c: *mut YaceHostCtx, f: impl FnOnce(&HostCtx, &mut App) -> YaceStatus) -> YaceStatus {
    let Some(ctx) = ctx_of(c) else { return YACE_E_INVALID };
    let app = ctx.app.get();
    if app.is_null() {
        return fail(ctx, YACE_E_UNSUPPORTED, "host functions can only be called from a callback, on the UI thread");
    }
    // SAFETY: set by `call` for the duration of the plugin call; the UI
    // thread is inside that call, so no other reference to the app is used.
    let app = unsafe { &mut *app };
    let index = ctx.shared.index;
    match std::panic::catch_unwind(AssertUnwindSafe(|| f(ctx, app))) {
        Ok(s) => s,
        Err(_) => {
            if let Some(e) = app.plugins.entries.get_mut(index) {
                e.push_log("internal error in a host function (reported as failure)".into());
            }
            fail(ctx, YACE_E_FAILED, "internal error")
        }
    }
}

fn needs(app: &App, ctx: &HostCtx, cap: &str) -> Result<(), YaceStatus> {
    let ok = app
        .plugins
        .entries
        .get(ctx.shared.index)
        .and_then(|e| e.manifest.as_ref())
        .is_some_and(|m| m.has_capability(cap));
    if ok {
        Ok(())
    } else {
        Err(fail(ctx, YACE_E_DENIED, format!("the plugin did not declare the `{cap}` capability")))
    }
}

macro_rules! arg {
    ($ctx:expr, $v:expr, $what:literal) => {
        match str_of($v) {
            Ok(s) => s,
            Err(e) => return fail($ctx, YACE_E_INVALID, format!("{}: {e}", $what)),
        }
    };
}

extern "C" fn h_log(c: *mut YaceHostCtx, level: u32, msg: YaceStr) {
    with_app(c, |ctx, app| {
        let m = arg!(ctx, msg, "message");
        app.plugin_log(ctx.shared.index, level, m);
        YACE_OK
    });
}

extern "C" fn h_notify(c: *mut YaceHostCtx, level: u32, msg: YaceStr) {
    with_app(c, |ctx, app| {
        let m = arg!(ctx, msg, "message");
        app.plugin_notify(ctx.shared.index, level, m);
        YACE_OK
    });
}

extern "C" fn h_last_error(c: *mut YaceHostCtx) -> YaceStr {
    let Some(ctx) = ctx_of(c) else { return YaceStr::EMPTY };
    let msg = ctx.last_error.borrow().clone();
    let mut out = ctx.error_out.borrow_mut();
    *out = msg;
    YaceStr { ptr: out.as_ptr(), len: out.len() }
}

extern "C" fn h_free(_c: *mut YaceHostCtx, buf: YaceBuf) {
    if buf.ptr.is_null() || buf.cap == 0 {
        return;
    }
    // SAFETY: only buffers made by `fill` (boxed slices of `cap` bytes)
    // are handed out; a plugin passing anything else breaks the ABI.
    let _ = std::panic::catch_unwind(|| {
        drop(unsafe { Box::from_raw(std::ptr::slice_from_raw_parts_mut(buf.ptr, buf.cap)) })
    });
}

extern "C" fn h_get_setting(c: *mut YaceHostCtx, key: YaceStr, out: *mut YaceBuf) -> YaceStatus {
    with_app(c, |ctx, app| {
        let key = arg!(ctx, key, "key");
        if out.is_null() {
            return fail(ctx, YACE_E_INVALID, "null output buffer");
        }
        match app.plugin_setting(ctx.shared.index, &key) {
            Ok(v) => fill(ctx, out, v.to_string()),
            Err(e) => fail(ctx, YACE_E_NOT_FOUND, e),
        }
    })
}

extern "C" fn h_workspace_roots(c: *mut YaceHostCtx, out: *mut YaceBuf) -> YaceStatus {
    with_app(c, |ctx, app| {
        if let Err(s) = needs(app, ctx, "workspace.read") {
            return s;
        }
        fill(ctx, out, app.plugin_roots_json().to_string())
    })
}

extern "C" fn h_active_document(c: *mut YaceHostCtx, out: *mut YaceBuf) -> YaceStatus {
    with_app(c, |ctx, app| {
        if let Err(s) = needs(app, ctx, "documents.read") {
            return s;
        }
        fill(ctx, out, app.plugin_active_document_json().to_string())
    })
}

extern "C" fn h_document_text(
    c: *mut YaceHostCtx,
    doc: YaceHandle,
    start: u64,
    end: u64,
    out: *mut YaceBuf,
) -> YaceStatus {
    with_app(c, |ctx, app| {
        if let Err(s) = needs(app, ctx, "documents.read") {
            return s;
        }
        match app.plugin_document_text(doc, start, end) {
            Ok(t) => fill(ctx, out, t),
            Err(e) => fail(ctx, YACE_E_NOT_FOUND, e),
        }
    })
}

extern "C" fn h_apply_edits(c: *mut YaceHostCtx, doc: YaceHandle, version: u64, edits: YaceStr) -> YaceStatus {
    with_app(c, |ctx, app| {
        if let Err(s) = needs(app, ctx, "documents.write") {
            return s;
        }
        let json = arg!(ctx, edits, "edits");
        let edits: Vec<EditIn> = match serde_json::from_str(&json) {
            Ok(e) => e,
            Err(e) => return fail(ctx, YACE_E_INVALID, format!("edits: {e}")),
        };
        match app.plugin_apply_edits(doc, version, &edits) {
            Ok(()) => YACE_OK,
            Err((s, e)) => fail(ctx, s, e),
        }
    })
}

extern "C" fn h_open_file(c: *mut YaceHostCtx, path: YaceStr, line: u32, column: u32) -> YaceStatus {
    with_app(c, |ctx, app| {
        let path = arg!(ctx, path, "path");
        match app.plugin_open_file(&path, line, column) {
            Ok(()) => YACE_OK,
            Err(e) => fail(ctx, YACE_E_NOT_FOUND, e),
        }
    })
}

extern "C" fn h_execute_command(c: *mut YaceHostCtx, id: YaceStr, args: YaceStr) -> YaceStatus {
    with_app(c, |ctx, app| {
        let id = arg!(ctx, id, "command id");
        let args = arg!(ctx, args, "args");
        let args = if args.trim().is_empty() {
            None
        } else {
            match serde_json::from_str(&args) {
                Ok(v) => Some(v),
                Err(e) => return fail(ctx, YACE_E_INVALID, format!("args: {e}")),
            }
        };
        if app.commands.get(&id).is_none() {
            return fail(ctx, YACE_E_NOT_FOUND, format!("unknown command `{id}`"));
        }
        app.plugins.deferred.push((id, args));
        YACE_OK
    })
}

extern "C" fn h_set_status_item(c: *mut YaceHostCtx, id: YaceStr, item: YaceStr) -> YaceStatus {
    with_app(c, |ctx, app| {
        let id = arg!(ctx, id, "status item id");
        let json = arg!(ctx, item, "item");
        let item: Option<StatusContent> = match serde_json::from_str(&json) {
            Ok(i) => i,
            Err(e) => return fail(ctx, YACE_E_INVALID, format!("status item: {e}")),
        };
        match app.plugin_set_status(ctx.shared.index, &id, item) {
            Ok(()) => YACE_OK,
            Err(e) => fail(ctx, YACE_E_NOT_FOUND, e),
        }
    })
}

extern "C" fn h_set_view_items(c: *mut YaceHostCtx, view: YaceStr, items: YaceStr) -> YaceStatus {
    with_app(c, |ctx, app| {
        let view = arg!(ctx, view, "view id");
        let json = arg!(ctx, items, "items");
        let items: Vec<ViewNode> = match serde_json::from_str(&json) {
            Ok(i) => i,
            Err(e) => return fail(ctx, YACE_E_INVALID, format!("view items: {e}")),
        };
        match app.plugin_set_view(ctx.shared.index, &view, items) {
            Ok(()) => YACE_OK,
            Err(e) => fail(ctx, YACE_E_NOT_FOUND, e),
        }
    })
}

extern "C" fn h_set_decorations(c: *mut YaceHostCtx, root: YaceHandle, items: YaceStr) -> YaceStatus {
    with_app(c, |ctx, app| {
        let json = arg!(ctx, items, "items");
        let items: Vec<DecorationIn> = match serde_json::from_str(&json) {
            Ok(i) => i,
            Err(e) => return fail(ctx, YACE_E_INVALID, format!("decorations: {e}")),
        };
        match app.plugin_set_decorations(ctx.shared.index, root, items) {
            Ok(()) => YACE_OK,
            Err(e) => fail(ctx, YACE_E_NOT_FOUND, e),
        }
    })
}

extern "C" fn h_quick_open_results(c: *mut YaceHostCtx, request: u64, items: YaceStr) -> YaceStatus {
    with_app(c, |ctx, app| {
        let json = arg!(ctx, items, "items");
        let items: Vec<QuickOpenIn> = match serde_json::from_str(&json) {
            Ok(i) => i,
            Err(e) => return fail(ctx, YACE_E_INVALID, format!("quick open items: {e}")),
        };
        app.plugin_quick_open_results(ctx.shared.index, request, items);
        YACE_OK
    })
}

extern "C" fn h_create_terminal(c: *mut YaceHostCtx, spec: YaceStr, out: *mut YaceHandle) -> YaceStatus {
    with_app(c, |ctx, app| {
        if let Err(s) = needs(app, ctx, "terminal") {
            return s;
        }
        let json = arg!(ctx, spec, "terminal spec");
        let spec: TerminalIn = match serde_json::from_str(&json) {
            Ok(s) => s,
            Err(e) => return fail(ctx, YACE_E_INVALID, format!("terminal spec: {e}")),
        };
        match app.plugin_create_terminal(ctx.shared.index, spec) {
            Ok(id) => {
                if !out.is_null() {
                    // SAFETY: checked non-null; a plugin-provided output slot.
                    unsafe { *out = id };
                }
                YACE_OK
            }
            Err(e) => fail(ctx, YACE_E_FAILED, e),
        }
    })
}

/// A plugin job: its function and data pointer, moved to a worker thread.
struct RawJob {
    f: YaceJobFn,
    data: *mut c_void,
}
// SAFETY: the ABI makes the plugin responsible for `data` being usable on
// the worker thread it asked for.
unsafe impl Send for RawJob {}

extern "C" fn h_spawn_job(c: *mut YaceHostCtx, job: YaceJobFn, data: *mut c_void, out: *mut YaceHandle) -> YaceStatus {
    with_app(c, |ctx, _app| {
        let raw = RawJob { f: job, data };
        let id = ctx.shared.jobs.spawn(ctx.shared.index, move |id| {
            let raw = raw;
            (raw.f)(raw.data, id);
        });
        if !out.is_null() {
            // SAFETY: checked non-null.
            unsafe { *out = id };
        }
        YACE_OK
    })
}

/// The thread-safe part of a context, without referencing the rest (which
/// the UI thread may be using).
fn shared_of<'a>(c: *mut YaceHostCtx) -> Option<&'a Arc<SharedCtx>> {
    if c.is_null() {
        return None;
    }
    // SAFETY: `c` is our `HostCtx`; only its `shared` field (an `Arc` of a
    // `Sync` type) is borrowed.
    Some(unsafe { &*std::ptr::addr_of!((*(c as *const HostCtx)).shared) })
}

extern "C" fn h_job_cancelled(c: *mut YaceHostCtx, job: YaceHandle) -> bool {
    let Some(shared) = shared_of(c) else { return true };
    shared.jobs.cancelled(job)
}

extern "C" fn h_post_message(c: *mut YaceHostCtx, data: YaceStr) -> YaceStatus {
    let Some(shared) = shared_of(c) else { return YACE_E_INVALID };
    let Ok(data) = str_of(data) else { return YACE_E_INVALID };
    match shared.tx.send(PluginMsg { plugin: shared.index, data }) {
        Ok(()) => YACE_OK,
        Err(_) => YACE_E_FAILED,
    }
}
