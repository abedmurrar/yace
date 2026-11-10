//! The YACE native plugin ABI (spec §18.1, §18.8), version 1.
//!
//! This crate defines only `#[repr(C)]` types and constants; it is the
//! contract between the editor (`yace-plugin-host`) and a plugin (usually
//! written with the `yace-plugin` SDK, but any language with a C ABI works:
//! see `include/yace_plugin.h`). Rules:
//!
//! - **Entry point.** A plugin library exports
//!   `yace_plugin_init(abi, host, out) -> YaceStatus` ([`InitFn`]). The host
//!   calls it once, after the manifest was validated, with
//!   [`ABI_VERSION`]. A plugin built for another major ABI returns
//!   [`YACE_E_ABI`] without touching `out`.
//! - **Struct sizes.** [`YaceHost`] and [`YacePlugin`] start with
//!   `size` and `abi_version`; fields are only ever appended within a major
//!   version. The host passes `out` zeroed with `out.size` set to the size
//!   it knows; the plugin writes at most that many bytes and sets `size` to
//!   what it wrote. Each side reads only the fields both know. A null
//!   callback means the plugin does not implement it.
//! - **Strings** are [`YaceStr`]: UTF-8, not NUL-terminated, borrowed for
//!   the duration of the call only. Structured data (settings, events,
//!   items) crosses the boundary as UTF-8 JSON; document text as raw UTF-8.
//! - **Ownership.** A [`YaceBuf`] filled by the host belongs to the plugin,
//!   which returns it with `host.free`. The plugin never frees host memory
//!   any other way, and vice versa.
//! - **Handles** (documents, roots, terminals, jobs) are opaque `u64`s; no
//!   pointer into editor memory is ever exposed (spec §18.2).
//! - **Threading.** Every plugin callback runs on the editor's UI thread and
//!   must return quickly (calls are timed; slow plugins are reported).
//!   Host functions may be called only from inside a callback, on that
//!   thread — except `post_message` and `job_cancelled`, which any thread
//!   may call. Long work goes to `spawn_job` (host worker threads).
//! - **Cancellation.** A job polls `job_cancelled(job)`; jobs are cancelled
//!   when the plugin deactivates.
//! - **Errors.** Functions return a [`YaceStatus`]; on failure
//!   `host.last_error` describes the last host-side error.
//! - **Panics/exceptions** must never unwind across the boundary (the SDK
//!   catches them and returns [`YACE_E_PANIC`]).

#![no_std]

use core::ffi::c_void;

/// The ABI major version this crate describes.
pub const ABI_VERSION: u32 = 1;

/// Name of the entry point a plugin library exports.
pub const INIT_SYMBOL: &str = "yace_plugin_init";

/// Status codes. Plain integers rather than an enum, so an unknown value
/// from foreign code is never undefined behaviour.
pub type YaceStatus = i32;
pub const YACE_OK: YaceStatus = 0;
/// Generic failure; see `last_error`.
pub const YACE_E_FAILED: YaceStatus = 1;
/// Unsupported ABI version.
pub const YACE_E_ABI: YaceStatus = 2;
/// A null pointer, invalid UTF-8, malformed JSON or an oversized value.
pub const YACE_E_INVALID: YaceStatus = 3;
/// Unknown handle, document, view or command.
pub const YACE_E_NOT_FOUND: YaceStatus = 4;
/// The document changed since the version given.
pub const YACE_E_STALE: YaceStatus = 5;
/// The plugin did not declare the capability this call needs.
pub const YACE_E_DENIED: YaceStatus = 6;
/// Not available in this context (e.g. called from the wrong thread).
pub const YACE_E_UNSUPPORTED: YaceStatus = 7;
/// The operation was cancelled.
pub const YACE_E_CANCELLED: YaceStatus = 8;
/// The plugin panicked (set by the SDK).
pub const YACE_E_PANIC: YaceStatus = 9;
/// `on_key`: the plugin did not handle the key; YACE handles it as usual.
pub const YACE_PASS: YaceStatus = 10;

/// Log / notification levels.
pub const YACE_LEVEL_DEBUG: u32 = 0;
pub const YACE_LEVEL_INFO: u32 = 1;
pub const YACE_LEVEL_WARNING: u32 = 2;
pub const YACE_LEVEL_ERROR: u32 = 3;

/// An opaque handle (document, root, terminal, job).
pub type YaceHandle = u64;

/// A borrowed UTF-8 string.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct YaceStr {
    pub ptr: *const u8,
    pub len: usize,
}

impl YaceStr {
    pub const EMPTY: YaceStr = YaceStr { ptr: core::ptr::null(), len: 0 };

    pub const fn from_str(s: &str) -> YaceStr {
        YaceStr { ptr: s.as_ptr(), len: s.len() }
    }
}

/// A buffer filled by the host and owned by the plugin until it calls
/// `host.free` on it.
#[repr(C)]
#[derive(Debug)]
pub struct YaceBuf {
    pub ptr: *mut u8,
    pub len: usize,
    pub cap: usize,
}

impl YaceBuf {
    pub const EMPTY: YaceBuf = YaceBuf { ptr: core::ptr::null_mut(), len: 0, cap: 0 };
}

/// Identifies the calling plugin to the host. Opaque to plugins.
#[repr(C)]
pub struct YaceHostCtx {
    _private: [u8; 0],
}

/// A background job: runs on a host worker thread with the plugin's data
/// and the job's handle (for `job_cancelled` / `post_message`).
pub type YaceJobFn = extern "C" fn(data: *mut c_void, job: YaceHandle);

/// Functions the editor provides to a plugin. Version 1.
#[repr(C)]
pub struct YaceHost {
    pub size: u32,
    pub abi_version: u32,
    /// Pass back as the first argument of every call.
    pub ctx: *mut YaceHostCtx,

    // --- Diagnostics ---------------------------------------------------
    /// Write to the plugin's log ("Extensions: Show Plugin Log").
    pub log: extern "C" fn(ctx: *mut YaceHostCtx, level: u32, message: YaceStr),
    /// Show a notification.
    pub notify: extern "C" fn(ctx: *mut YaceHostCtx, level: u32, message: YaceStr),
    /// Description of the last failed host call (valid until the next call).
    pub last_error: extern "C" fn(ctx: *mut YaceHostCtx) -> YaceStr,
    /// Release a buffer the host filled.
    pub free: extern "C" fn(ctx: *mut YaceHostCtx, buf: YaceBuf),

    // --- Settings and workspace (JSON out) -------------------------------
    /// The plugin's setting `key` (declared in its manifest) as JSON.
    pub get_setting: extern "C" fn(ctx: *mut YaceHostCtx, key: YaceStr, out: *mut YaceBuf) -> YaceStatus,
    /// `[{"id": u64, "name": str, "path": str, "available": bool}, …]`.
    pub workspace_roots: extern "C" fn(ctx: *mut YaceHostCtx, out: *mut YaceBuf) -> YaceStatus,
    /// The active editor: `{"doc": u64, "version": u64, "path": str|null,
    /// "language": str, "selections": [{"anchor": n, "head": n}]}` (char
    /// offsets), or `null`.
    pub active_document: extern "C" fn(ctx: *mut YaceHostCtx, out: *mut YaceBuf) -> YaceStatus,

    // --- Documents ------------------------------------------------------
    /// Text of a document (raw UTF-8) between char offsets `start..end`
    /// (`end = u64::MAX` for the end). Needs `documents.read`.
    pub document_text:
        extern "C" fn(ctx: *mut YaceHostCtx, doc: YaceHandle, start: u64, end: u64, out: *mut YaceBuf) -> YaceStatus,
    /// Apply `[{"start": n, "end": n, "text": str}, …]` (char offsets) as one
    /// undo step, if the document is still at `version`. Needs
    /// `documents.write`.
    pub apply_edits: extern "C" fn(ctx: *mut YaceHostCtx, doc: YaceHandle, version: u64, edits: YaceStr) -> YaceStatus,
    /// Open a file at a 1-based line and column (0 = unspecified).
    pub open_file: extern "C" fn(ctx: *mut YaceHostCtx, path: YaceStr, line: u32, column: u32) -> YaceStatus,
    /// Run a command (deferred until the current callback returns).
    pub execute_command: extern "C" fn(ctx: *mut YaceHostCtx, id: YaceStr, args: YaceStr) -> YaceStatus,

    // --- UI contributions (JSON in) ---------------------------------------
    /// Set a status bar item declared in the manifest:
    /// `{"text": str, "tooltip": str?, "command": str?, "color": token?}`;
    /// `null` hides it.
    pub set_status_item: extern "C" fn(ctx: *mut YaceHostCtx, id: YaceStr, item: YaceStr) -> YaceStatus,
    /// Replace the items of a view declared in the manifest: a tree of
    /// `{"id": str, "label": str, "description": str?, "icon": str?,
    /// "color": token?, "command": str?, "args": any?, "children": [...]?,
    /// "expanded": bool?}`.
    pub set_view_items: extern "C" fn(ctx: *mut YaceHostCtx, view: YaceStr, items: YaceStr) -> YaceStatus,
    /// Explorer decorations for files under a root:
    /// `[{"path": str, "badge": str, "color": token?, "tooltip": str?}]`.
    pub set_decorations: extern "C" fn(ctx: *mut YaceHostCtx, root: YaceHandle, items: YaceStr) -> YaceStatus,
    /// Answer an `on_quick_open` request: `[{"label", "description"?,
    /// "detail"?, "command", "args"?}]`. Late answers are ignored.
    pub quick_open_results: extern "C" fn(ctx: *mut YaceHostCtx, request: u64, items: YaceStr) -> YaceStatus,
    /// Open an integrated terminal: `{"name": str?, "cwd": str?, "root":
    /// u64?, "command": str?, "args": [str]?}`. Needs `terminal`.
    pub create_terminal: extern "C" fn(ctx: *mut YaceHostCtx, spec: YaceStr, out: *mut YaceHandle) -> YaceStatus,

    // --- Background work ---------------------------------------------------
    /// Run `job(data, handle)` on a host worker thread.
    pub spawn_job:
        extern "C" fn(ctx: *mut YaceHostCtx, job: YaceJobFn, data: *mut c_void, out: *mut YaceHandle) -> YaceStatus,
    /// True once the job should stop. Any thread.
    pub job_cancelled: extern "C" fn(ctx: *mut YaceHostCtx, job: YaceHandle) -> bool,
    /// Deliver `data` to the plugin's `on_message` on the UI thread. Any
    /// thread.
    pub post_message: extern "C" fn(ctx: *mut YaceHostCtx, data: YaceStr) -> YaceStatus,

    // --- Added in YACE 0.15 (check `size` before calling) ------------------
    /// Set the selections of the editor showing `doc` (the active one if it
    /// shows it): `[{"anchor": n, "head": n}, …]` in char offsets, the first
    /// is the primary; the cursor is at `head`. Fails if the document is no
    /// longer at `version`. Needs `editor.selections`.
    pub set_selections:
        extern "C" fn(ctx: *mut YaceHostCtx, doc: YaceHandle, version: u64, sels: YaceStr) -> YaceStatus,
    /// The terminal cursor's shape: `"block"`, `"bar"`, `"underline"` or
    /// `"default"` (the user's).
    pub set_cursor_style: extern "C" fn(ctx: *mut YaceHostCtx, style: YaceStr) -> YaceStatus,
    /// While on, every key typed in an editor goes to `on_key` first (modal
    /// editing). Needs `keyboard`.
    pub capture_keys: extern "C" fn(ctx: *mut YaceHostCtx, on: bool) -> YaceStatus,
}

/// Callbacks a plugin provides. Version 1. Every pointer may be null.
#[repr(C)]
pub struct YacePlugin {
    pub size: u32,
    pub abi_version: u32,
    /// The plugin's state, passed back to every callback.
    pub instance: *mut c_void,
    /// Called on the first activation event.
    pub activate: Option<extern "C" fn(instance: *mut c_void) -> YaceStatus>,
    /// Called before the plugin is disabled or the editor exits.
    pub deactivate: Option<extern "C" fn(instance: *mut c_void)>,
    /// A command declared in the manifest ran; `args` is JSON or empty.
    pub on_command: Option<extern "C" fn(instance: *mut c_void, id: YaceStr, args: YaceStr) -> YaceStatus>,
    /// An event the plugin subscribed to (`events` in the manifest), as JSON
    /// `{"type": str, …}`.
    pub on_event: Option<extern "C" fn(instance: *mut c_void, event: YaceStr)>,
    /// Data sent with `post_message` (usually from a job).
    pub on_message: Option<extern "C" fn(instance: *mut c_void, data: YaceStr)>,
    /// An item of one of the plugin's views was activated (`action` =
    /// "open") or a context action was chosen.
    pub on_view_action:
        Option<extern "C" fn(instance: *mut c_void, view: YaceStr, item: YaceStr, action: YaceStr) -> YaceStatus>,
    /// Quick Open with the plugin's prefix; answer with `quick_open_results`.
    pub on_quick_open:
        Option<extern "C" fn(instance: *mut c_void, provider: YaceStr, query: YaceStr, request: u64) -> YaceStatus>,
    /// Free the plugin's state (after `deactivate`, before unloading).
    pub destroy: Option<extern "C" fn(instance: *mut c_void)>,
    /// Added in YACE 0.15. A key typed in an editor while `capture_keys` is
    /// on: `{"key": "ctrl+r", "text": "r"|null, "popup": bool}` (`key` in
    /// keybinding syntax, `text` what it would type, `popup` whether a
    /// completion, hover or parameter-hint popup is open). Return
    /// [`YACE_OK`] if handled, [`YACE_PASS`] to let YACE handle it.
    pub on_key: Option<extern "C" fn(instance: *mut c_void, key: YaceStr) -> YaceStatus>,
}

/// `yace_plugin_init`.
pub type InitFn = unsafe extern "C" fn(abi: u32, host: *const YaceHost, out: *mut YacePlugin) -> YaceStatus;

/// Size of the first version-1 tables (YACE ≤ 0.14), the smallest the host
/// accepts; fields after them were appended later.
pub const PLUGIN_V1_SIZE: usize = core::mem::offset_of!(YacePlugin, on_key);
pub const HOST_V1_SIZE: usize = core::mem::offset_of!(YaceHost, set_selections);

#[cfg(test)]
extern crate std;

#[cfg(test)]
mod tests {
    use std::{format, process::Command, string::String, vec::Vec};

    use super::*;

    /// The C header matches the Rust layout (sizes and every offset).
    #[test]
    fn c_header_matches() {
        if Command::new("cc").arg("--version").output().is_err() {
            std::eprintln!("no C compiler; skipped");
            return;
        }
        macro_rules! off {
            ($t:ty, $($f:ident),*) => {{
                let mut v: Vec<(String, usize)> = Vec::new();
                $( v.push((format!("{}.{}", stringify!($t), stringify!($f)), core::mem::offset_of!($t, $f))); )*
                v.push((format!("sizeof {}", stringify!($t)), core::mem::size_of::<$t>()));
                v
            }};
        }
        let mut want = off!(YaceStr, ptr, len);
        want.extend(off!(YaceBuf, ptr, len, cap));
        want.extend(off!(
            YaceHost,
            size,
            abi_version,
            ctx,
            log,
            notify,
            last_error,
            free,
            get_setting,
            workspace_roots,
            active_document,
            document_text,
            apply_edits,
            open_file,
            execute_command,
            set_status_item,
            set_view_items,
            set_decorations,
            quick_open_results,
            create_terminal,
            spawn_job,
            job_cancelled,
            post_message,
            set_selections,
            set_cursor_style,
            capture_keys
        ));
        want.extend(off!(
            YacePlugin,
            size,
            abi_version,
            instance,
            activate,
            deactivate,
            on_command,
            on_event,
            on_message,
            on_view_action,
            on_quick_open,
            destroy,
            on_key
        ));
        let mut c =
            String::from("#include <stdio.h>\n#include <stddef.h>\n#include \"yace_plugin.h\"\nint main(void) {\n");
        for (name, _) in &want {
            let expr = match name.strip_prefix("sizeof ") {
                Some(t) => format!("sizeof({t})"),
                None => {
                    let (t, f) = name.split_once('.').unwrap();
                    format!("offsetof({t}, {f})")
                }
            };
            c.push_str(&format!("  printf(\"%zu\\n\", (size_t){expr});\n"));
        }
        c.push_str("  return 0;\n}\n");
        let dir = std::env::temp_dir().join(format!("yace-abi-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("t.c"), c).unwrap();
        let include = concat!(env!("CARGO_MANIFEST_DIR"), "/include");
        let st = Command::new("cc")
            .args(["-std=c11", "-Wall", "-Werror", "-I", include, "t.c", "-o", "t"])
            .current_dir(&dir)
            .status()
            .unwrap();
        assert!(st.success(), "the header does not compile");
        let out = Command::new(dir.join("t")).output().unwrap();
        let got: Vec<usize> = String::from_utf8(out.stdout).unwrap().lines().map(|l| l.parse().unwrap()).collect();
        for ((name, w), g) in want.iter().zip(&got) {
            assert_eq!(w, g, "{name}");
        }
        assert_eq!(got.len(), want.len());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
