//! Rust SDK for YACE native plugins (spec §18.15).
//!
//! Implement [`Plugin`], export it with [`export_plugin!`], build the crate
//! as a `cdylib`, and describe it in `yace-plugin.toml`:
//!
//! ```ignore
//! use yace_plugin::{Host, Plugin, Result, export_plugin};
//!
//! struct Hello { host: Host }
//!
//! impl Plugin for Hello {
//!     fn new(host: Host) -> Self { Hello { host } }
//!     fn command(&mut self, id: &str, _args: Option<serde_json::Value>) -> Result<()> {
//!         if id == "hello.greet" {
//!             self.host.info("Hello from a plugin!");
//!         }
//!         Ok(())
//!     }
//! }
//!
//! export_plugin!(Hello);
//! ```
//!
//! Callbacks run on the editor's UI thread and must return quickly; do slow
//! work in [`Host::spawn_job`] and send results back with [`Job::post`]
//! (they arrive in [`Plugin::message`]). Panics are caught at the
//! boundary and reported as errors; they never unwind into the editor.
//! See `docs/plugins.md` for the manifest and the ABI.

#![allow(unsafe_code)] // the FFI boundary lives here, so plugins need none

use std::{ffi::c_void, panic::AssertUnwindSafe};

use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::Value;
pub use yace_plugin_api as abi;
use yace_plugin_api::{
    ABI_VERSION, HOST_V1_SIZE, YACE_E_ABI, YACE_E_FAILED, YACE_E_INVALID, YACE_E_PANIC, YACE_E_UNSUPPORTED, YACE_OK,
    YaceBuf, YaceHandle, YaceHost, YacePlugin, YaceStatus, YaceStr,
};

pub mod testing;

/// An error returned to the editor: a status code and a message (logged).
#[derive(Debug, Clone, PartialEq)]
pub struct Error {
    pub status: YaceStatus,
    pub message: String,
}

impl Error {
    pub fn new(message: impl Into<String>) -> Self {
        Self { status: YACE_E_FAILED, message: message.into() }
    }
    pub fn unsupported() -> Self {
        Self { status: YACE_E_UNSUPPORTED, message: "not supported".into() }
    }
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} (status {})", self.message, self.status)
    }
}

impl std::error::Error for Error {}

pub type Result<T> = std::result::Result<T, Error>;

/// Levels for [`Host::log`] and [`Host::notify`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Level {
    Debug,
    Info,
    Warning,
    Error,
}

impl Level {
    fn raw(self) -> u32 {
        match self {
            Level::Debug => abi::YACE_LEVEL_DEBUG,
            Level::Info => abi::YACE_LEVEL_INFO,
            Level::Warning => abi::YACE_LEVEL_WARNING,
            Level::Error => abi::YACE_LEVEL_ERROR,
        }
    }
}

/// A workspace folder.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct Root {
    pub id: u64,
    pub name: String,
    pub path: String,
    #[serde(default)]
    pub available: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Deserialize, Serialize)]
pub struct Selection {
    pub anchor: u64,
    pub head: u64,
}

/// The active editor's document.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct ActiveDocument {
    pub doc: u64,
    pub version: u64,
    pub path: Option<String>,
    pub language: String,
    #[serde(default)]
    pub selections: Vec<Selection>,
}

/// Replace chars `start..end` with `text`.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct Edit {
    pub start: u64,
    pub end: u64,
    pub text: String,
}

/// A status bar item's content.
#[derive(Debug, Clone, Default, PartialEq, Deserialize, Serialize)]
pub struct StatusItem {
    pub text: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub tooltip: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub command: String,
    /// A theme colour token: `error`, `warning`, `info`, `muted`, …
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub color: String,
}

/// A row of a plugin view (a tree).
#[derive(Debug, Clone, Default, PartialEq, Deserialize, Serialize)]
pub struct ViewItem {
    pub id: String,
    pub label: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
    /// A short glyph shown before the label.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub icon: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub color: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub children: Vec<ViewItem>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub expanded: bool,
}

/// An Explorer badge on a file.
#[derive(Debug, Clone, Default, PartialEq, Deserialize, Serialize)]
pub struct Decoration {
    /// Path relative to the root (or absolute).
    pub path: String,
    pub badge: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub color: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub tooltip: String,
}

/// A Quick Open result. Picking it runs `command` with `args`.
#[derive(Debug, Clone, Default, PartialEq, Deserialize, Serialize)]
pub struct QuickOpenItem {
    pub label: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
    pub command: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub args: Option<Value>,
}

/// An integrated terminal to open.
#[derive(Debug, Clone, Default, PartialEq, Deserialize, Serialize)]
pub struct TerminalSpec {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub name: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub cwd: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub root: Option<u64>,
    /// A program to run instead of the user's shell.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub command: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub args: Vec<String>,
}

fn s(v: &str) -> YaceStr {
    YaceStr::from_str(v)
}

/// Read a borrowed string from the editor.
///
/// # Safety
/// `v` must point to `len` readable bytes for the duration of the call
/// (guaranteed by the ABI for every string the editor passes).
unsafe fn str_of<'a>(v: YaceStr) -> &'a str {
    if v.ptr.is_null() || v.len == 0 {
        return "";
    }
    // SAFETY: the caller guarantees the bytes; the editor sends UTF-8, but
    // stay defensive.
    let bytes = unsafe { std::slice::from_raw_parts(v.ptr, v.len) };
    std::str::from_utf8(bytes).unwrap_or("")
}

/// The editor's API, for use inside callbacks (on the UI thread).
#[derive(Clone, Copy)]
pub struct Host {
    raw: *const YaceHost,
}

impl std::fmt::Debug for Host {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Host")
    }
}

impl Host {
    /// # Safety
    /// `raw` must point to a valid host table for the plugin's lifetime.
    pub unsafe fn from_raw(raw: *const YaceHost) -> Self {
        Self { raw }
    }

    fn h(&self) -> &YaceHost {
        // SAFETY: the table outlives the plugin (ABI rule).
        unsafe { &*self.raw }
    }

    fn check(&self, status: YaceStatus) -> Result<()> {
        if status == YACE_OK {
            return Ok(());
        }
        let h = self.h();
        // SAFETY: valid until the next host call (ABI rule); copied now.
        let message = unsafe { str_of((h.last_error)(h.ctx)) }.to_string();
        Err(Error { status, message })
    }

    fn take_buf(&self, buf: YaceBuf) -> String {
        let out = if buf.ptr.is_null() {
            String::new()
        } else {
            // SAFETY: the host filled `len` bytes of UTF-8.
            let bytes = unsafe { std::slice::from_raw_parts(buf.ptr, buf.len) };
            String::from_utf8_lossy(bytes).into_owned()
        };
        let h = self.h();
        (h.free)(h.ctx, buf);
        out
    }

    fn json_out(&self, f: impl FnOnce(&YaceHost, *mut YaceBuf) -> YaceStatus) -> Result<String> {
        let mut buf = YaceBuf::EMPTY;
        let st = f(self.h(), &mut buf);
        let text = self.take_buf(buf);
        self.check(st)?;
        Ok(text)
    }

    pub fn log(&self, level: Level, message: &str) {
        let h = self.h();
        (h.log)(h.ctx, level.raw(), s(message));
    }

    pub fn notify(&self, level: Level, message: &str) {
        let h = self.h();
        (h.notify)(h.ctx, level.raw(), s(message));
    }

    pub fn info(&self, message: &str) {
        self.notify(Level::Info, message);
    }

    /// The plugin's setting (declared under `[settings]` in its manifest).
    pub fn setting<T: DeserializeOwned>(&self, key: &str) -> Result<T> {
        let text = self.json_out(|h, out| (h.get_setting)(h.ctx, s(key), out))?;
        serde_json::from_str(&text).map_err(|e| Error::new(format!("setting `{key}`: {e}")))
    }

    pub fn roots(&self) -> Result<Vec<Root>> {
        let text = self.json_out(|h, out| (h.workspace_roots)(h.ctx, out))?;
        serde_json::from_str(&text).map_err(|e| Error::new(e.to_string()))
    }

    pub fn active_document(&self) -> Result<Option<ActiveDocument>> {
        let text = self.json_out(|h, out| (h.active_document)(h.ctx, out))?;
        serde_json::from_str(&text).map_err(|e| Error::new(e.to_string()))
    }

    /// Text of `doc` between char offsets (`end = u64::MAX`: to the end).
    pub fn document_text(&self, doc: u64, start: u64, end: u64) -> Result<String> {
        self.json_out(|h, out| (h.document_text)(h.ctx, doc, start, end, out))
    }

    pub fn apply_edits(&self, doc: u64, version: u64, edits: &[Edit]) -> Result<()> {
        let json = serde_json::to_string(edits).map_err(|e| Error::new(e.to_string()))?;
        let h = self.h();
        self.check((h.apply_edits)(h.ctx, doc, version, s(&json)))
    }

    pub fn open_file(&self, path: &str, line: u32, column: u32) -> Result<()> {
        let h = self.h();
        self.check((h.open_file)(h.ctx, s(path), line, column))
    }

    /// Run an editor or plugin command after the current callback.
    pub fn execute_command(&self, id: &str, args: Option<&Value>) -> Result<()> {
        let args = args.map(Value::to_string).unwrap_or_default();
        let h = self.h();
        self.check((h.execute_command)(h.ctx, s(id), s(&args)))
    }

    /// Show (`Some`) or hide (`None`) a status item declared in the manifest.
    pub fn set_status_item(&self, id: &str, item: Option<&StatusItem>) -> Result<()> {
        let json = match item {
            Some(i) => serde_json::to_string(i).map_err(|e| Error::new(e.to_string()))?,
            None => "null".into(),
        };
        let h = self.h();
        self.check((h.set_status_item)(h.ctx, s(id), s(&json)))
    }

    pub fn set_view_items(&self, view: &str, items: &[ViewItem]) -> Result<()> {
        let json = serde_json::to_string(items).map_err(|e| Error::new(e.to_string()))?;
        let h = self.h();
        self.check((h.set_view_items)(h.ctx, s(view), s(&json)))
    }

    pub fn set_decorations(&self, root: u64, items: &[Decoration]) -> Result<()> {
        let json = serde_json::to_string(items).map_err(|e| Error::new(e.to_string()))?;
        let h = self.h();
        self.check((h.set_decorations)(h.ctx, root, s(&json)))
    }

    pub fn quick_open_results(&self, request: u64, items: &[QuickOpenItem]) -> Result<()> {
        let json = serde_json::to_string(items).map_err(|e| Error::new(e.to_string()))?;
        let h = self.h();
        self.check((h.quick_open_results)(h.ctx, request, s(&json)))
    }

    pub fn create_terminal(&self, spec: &TerminalSpec) -> Result<u64> {
        let json = serde_json::to_string(spec).map_err(|e| Error::new(e.to_string()))?;
        let mut id: YaceHandle = 0;
        let h = self.h();
        self.check((h.create_terminal)(h.ctx, s(&json), &mut id))?;
        Ok(id)
    }

    /// A handle usable from any thread to post messages back.
    pub fn messenger(&self) -> Messenger {
        Messenger { raw: self.raw }
    }

    /// Run `f` on an editor worker thread. Check [`Job::cancelled`] often;
    /// send results with [`Job::post`].
    pub fn spawn_job<F: FnOnce(Job) + Send + 'static>(&self, f: F) -> Result<u64> {
        struct Payload {
            f: Box<dyn FnOnce(Job) + Send>,
            messenger: Messenger,
        }
        extern "C" fn trampoline(data: *mut c_void, job: YaceHandle) {
            // SAFETY: `data` is the Box leaked below, passed back exactly once.
            let payload = unsafe { Box::from_raw(data as *mut Payload) };
            let Payload { f, messenger } = *payload;
            let _ = std::panic::catch_unwind(AssertUnwindSafe(|| f(Job { id: job, messenger })));
        }
        let data = Box::into_raw(Box::new(Payload { f: Box::new(f), messenger: self.messenger() })) as *mut c_void;
        let mut id: YaceHandle = 0;
        let h = self.h();
        let st = (h.spawn_job)(h.ctx, trampoline, data, &mut id);
        if st != YACE_OK {
            // SAFETY: the host did not take the job; reclaim it.
            drop(unsafe { Box::from_raw(data as *mut Payload) });
        }
        self.check(st)?;
        Ok(id)
    }
}

/// The thread-safe part of the host: post messages, check cancellation.
#[derive(Clone, Copy)]
pub struct Messenger {
    raw: *const YaceHost,
}

// SAFETY: only `post_message` and `job_cancelled` are called through it,
// which the ABI declares callable from any thread.
unsafe impl Send for Messenger {}
unsafe impl Sync for Messenger {}

impl Messenger {
    /// Deliver `data` to [`Plugin::message`] on the UI thread.
    pub fn post(&self, data: &str) -> bool {
        // SAFETY: the table outlives the plugin.
        let h = unsafe { &*self.raw };
        (h.post_message)(h.ctx, s(data)) == YACE_OK
    }
}

/// A running background job.
pub struct Job {
    pub id: u64,
    messenger: Messenger,
}

impl Job {
    pub fn cancelled(&self) -> bool {
        // SAFETY: the table outlives the plugin; thread-safe by the ABI.
        let h = unsafe { &*self.messenger.raw };
        (h.job_cancelled)(h.ctx, self.id)
    }
    pub fn post(&self, data: &str) -> bool {
        self.messenger.post(data)
    }
}

/// A plugin. One instance is created when the library loads (on the first
/// activation event); every method runs on the UI thread.
pub trait Plugin: Sized + 'static {
    fn new(host: Host) -> Self;
    fn activate(&mut self) -> Result<()> {
        Ok(())
    }
    fn deactivate(&mut self) {}
    /// A command from the manifest ran; `args` as given by the caller.
    fn command(&mut self, _id: &str, _args: Option<Value>) -> Result<()> {
        Err(Error::unsupported())
    }
    /// A subscribed event: `{"type": "documentSaved", …}`.
    fn event(&mut self, _event: Value) {}
    /// Data posted by a job.
    fn message(&mut self, _data: &str) {}
    /// A view item was opened (`action` = "open").
    fn view_action(&mut self, _view: &str, _item: &str, _action: &str) -> Result<()> {
        Err(Error::unsupported())
    }
    /// Quick Open with the plugin's prefix; answer with
    /// [`Host::quick_open_results`] (now or later, using `request`).
    fn quick_open(&mut self, _provider: &str, _query: &str, _request: u64) -> Result<()> {
        Err(Error::unsupported())
    }
}

struct Instance<T> {
    plugin: T,
    host: Host,
}

fn guard<T: Plugin>(inst: *mut c_void, f: impl FnOnce(&mut T) -> Result<()>) -> YaceStatus {
    if inst.is_null() {
        return YACE_E_INVALID;
    }
    // SAFETY: `inst` is the Box created in `__init`, alive until `destroy`;
    // the editor calls back on one thread at a time.
    let me = unsafe { &mut *(inst as *mut Instance<T>) };
    let host = me.host;
    match std::panic::catch_unwind(AssertUnwindSafe(|| f(&mut me.plugin))) {
        Ok(Ok(())) => YACE_OK,
        Ok(Err(e)) => {
            if e.status != YACE_E_UNSUPPORTED {
                host.log(Level::Error, &e.message);
            }
            e.status
        }
        Err(p) => {
            let what = p
                .downcast_ref::<&str>()
                .map(|s| s.to_string())
                .or_else(|| p.downcast_ref::<String>().cloned())
                .unwrap_or_else(|| "panic".into());
            host.log(Level::Error, &format!("panicked: {what}"));
            YACE_E_PANIC
        }
    }
}

extern "C" fn cb_activate<T: Plugin>(inst: *mut c_void) -> YaceStatus {
    guard::<T>(inst, |p| p.activate())
}
extern "C" fn cb_deactivate<T: Plugin>(inst: *mut c_void) {
    guard::<T>(inst, |p| {
        p.deactivate();
        Ok(())
    });
}
extern "C" fn cb_command<T: Plugin>(inst: *mut c_void, id: YaceStr, args: YaceStr) -> YaceStatus {
    // SAFETY: borrowed for the call (ABI rule).
    let (id, args) = unsafe { (str_of(id), str_of(args)) };
    let args = if args.is_empty() { None } else { serde_json::from_str(args).ok() };
    guard::<T>(inst, |p| p.command(id, args))
}
extern "C" fn cb_event<T: Plugin>(inst: *mut c_void, ev: YaceStr) {
    // SAFETY: borrowed for the call.
    let ev = unsafe { str_of(ev) };
    let v = serde_json::from_str(ev).unwrap_or(Value::Null);
    guard::<T>(inst, |p| {
        p.event(v);
        Ok(())
    });
}
extern "C" fn cb_message<T: Plugin>(inst: *mut c_void, data: YaceStr) {
    // SAFETY: borrowed for the call.
    let data = unsafe { str_of(data) };
    guard::<T>(inst, |p| {
        p.message(data);
        Ok(())
    });
}
extern "C" fn cb_view_action<T: Plugin>(
    inst: *mut c_void,
    view: YaceStr,
    item: YaceStr,
    action: YaceStr,
) -> YaceStatus {
    // SAFETY: borrowed for the call.
    let (view, item, action) = unsafe { (str_of(view), str_of(item), str_of(action)) };
    guard::<T>(inst, |p| p.view_action(view, item, action))
}
extern "C" fn cb_quick_open<T: Plugin>(
    inst: *mut c_void,
    provider: YaceStr,
    query: YaceStr,
    request: u64,
) -> YaceStatus {
    // SAFETY: borrowed for the call.
    let (provider, query) = unsafe { (str_of(provider), str_of(query)) };
    guard::<T>(inst, |p| p.quick_open(provider, query, request))
}
extern "C" fn cb_destroy<T: Plugin>(inst: *mut c_void) {
    if !inst.is_null() {
        // SAFETY: created in `__init`, destroyed once.
        let _ = std::panic::catch_unwind(AssertUnwindSafe(|| drop(unsafe { Box::from_raw(inst as *mut Instance<T>) })));
    }
}

/// The body of `yace_plugin_init` (use [`export_plugin!`]).
///
/// # Safety
/// Called by the editor with valid `host` and `out` pointers.
#[doc(hidden)]
pub unsafe fn __init<T: Plugin>(abi: u32, host: *const YaceHost, out: *mut YacePlugin) -> YaceStatus {
    if abi != ABI_VERSION {
        return YACE_E_ABI;
    }
    if host.is_null() || out.is_null() {
        return YACE_E_INVALID;
    }
    // SAFETY: checked non-null; valid per the ABI.
    let h = unsafe { &*host };
    if h.abi_version != ABI_VERSION || (h.size as usize) < HOST_V1_SIZE {
        return YACE_E_ABI;
    }
    let result = std::panic::catch_unwind(AssertUnwindSafe(|| {
        // SAFETY: valid for the plugin's lifetime (ABI rule).
        let host = unsafe { Host::from_raw(host) };
        Box::into_raw(Box::new(Instance { plugin: T::new(host), host })) as *mut c_void
    }));
    let Ok(instance) = result else { return YACE_E_PANIC };
    let table = YacePlugin {
        size: std::mem::size_of::<YacePlugin>() as u32,
        abi_version: ABI_VERSION,
        instance,
        activate: Some(cb_activate::<T>),
        deactivate: Some(cb_deactivate::<T>),
        on_command: Some(cb_command::<T>),
        on_event: Some(cb_event::<T>),
        on_message: Some(cb_message::<T>),
        on_view_action: Some(cb_view_action::<T>),
        on_quick_open: Some(cb_quick_open::<T>),
        destroy: Some(cb_destroy::<T>),
    };
    // Write at most the size the editor knows (ABI rule).
    // SAFETY: `out` points to at least `out.size` writable bytes.
    unsafe {
        let n = ((*out).size as usize).min(std::mem::size_of::<YacePlugin>());
        std::ptr::copy_nonoverlapping(&table as *const YacePlugin as *const u8, out as *mut u8, n);
        (*out).size = n as u32;
    }
    YACE_OK
}

/// Export a [`Plugin`] type as the library's `yace_plugin_init`.
#[macro_export]
macro_rules! export_plugin {
    ($t:ty) => {
        /// Entry point called by YACE (see `yace_plugin_api`).
        ///
        /// # Safety
        /// Called by the editor with valid pointers.
        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn yace_plugin_init(
            abi: u32,
            host: *const $crate::abi::YaceHost,
            out: *mut $crate::abi::YacePlugin,
        ) -> $crate::abi::YaceStatus {
            // SAFETY: forwarded from the editor.
            unsafe { $crate::__init::<$t>(abi, host, out) }
        }
    };
}
