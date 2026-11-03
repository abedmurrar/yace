//! Loading plugin libraries (spec §18.5, §18.8). The only `unsafe` code of
//! the host lives here: opening a shared library, resolving its entry
//! point, and calling through the function table it returns. Every call is
//! timed (spec §18.10).
//!
//! Loading native code cannot be made safe: a plugin can crash the editor
//! or corrupt its memory (spec §18.9). What this module guarantees is that
//! a library built for another ABI, or one that returns a malformed table,
//! is rejected before any of its callbacks runs.

use std::{
    ffi::c_void,
    path::Path,
    time::{Duration, Instant},
};

use yace_plugin_api::{
    ABI_VERSION, InitFn, PLUGIN_V1_SIZE, YACE_E_ABI, YACE_OK, YaceHost, YacePlugin, YaceStatus, YaceStr,
};

/// Human-readable status code.
pub fn status_name(s: YaceStatus) -> &'static str {
    match s {
        0 => "ok",
        1 => "failed",
        2 => "unsupported ABI",
        3 => "invalid argument",
        4 => "not found",
        5 => "stale document version",
        6 => "capability not declared",
        7 => "unsupported",
        8 => "cancelled",
        9 => "the plugin panicked",
        _ => "unknown error",
    }
}

/// Which callback a timing belongs to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Callback {
    Init,
    Activate,
    Deactivate,
    Command,
    Event,
    Message,
    ViewAction,
    QuickOpen,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct CallStat {
    pub calls: u64,
    pub total: Duration,
    pub max: Duration,
    /// Calls over the slow threshold.
    pub slow: u64,
}

/// A callback slower than this is counted as slow (one frame at 60 Hz).
pub const SLOW: Duration = Duration::from_millis(16);

fn s(v: &str) -> YaceStr {
    YaceStr::from_str(v)
}

/// A loaded plugin library and its callback table.
pub struct Loaded {
    table: YacePlugin,
    /// Must outlive every call into the plugin (it keeps a pointer).
    _host: Box<YaceHost>,
    lib: Option<libloading::Library>,
    unloadable: bool,
    pub stats: std::collections::BTreeMap<Callback, CallStat>,
    shut_down: bool,
}

impl std::fmt::Debug for Loaded {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Loaded(size {})", self.table.size)
    }
}

impl Loaded {
    /// Open `path`, resolve `yace_plugin_init`, call it and validate the
    /// returned table. `host` stays alive as long as the plugin.
    #[allow(unsafe_code)]
    pub fn open(path: &Path, host: Box<YaceHost>, unloadable: bool) -> Result<(Loaded, Duration), String> {
        let start = Instant::now();
        // SAFETY: loading a library runs its initialisers. This is the
        // trust decision the user made when installing the plugin; the
        // caller has verified the file's hash against that approval.
        let lib = unsafe { open_now(path) }.map_err(|e| format!("could not load {}: {e}", path.display()))?;
        // SAFETY: the symbol's type is fixed by the plugin ABI.
        let init: InitFn = unsafe {
            *lib.get::<InitFn>(yace_plugin_api::INIT_SYMBOL.as_bytes())
                .map_err(|_| format!("{} does not export {}", path.display(), yace_plugin_api::INIT_SYMBOL))?
        };
        let mut table = YacePlugin {
            size: std::mem::size_of::<YacePlugin>() as u32,
            abi_version: 0,
            instance: std::ptr::null_mut(),
            activate: None,
            deactivate: None,
            on_command: None,
            on_event: None,
            on_message: None,
            on_view_action: None,
            on_quick_open: None,
            destroy: None,
        };
        // SAFETY: `host` and `table` are valid for the call; the plugin may
        // write at most `table.size` bytes (ABI rule).
        let status = unsafe { init(ABI_VERSION, &*host, &mut table) };
        if status == YACE_E_ABI {
            return Err(format!("the plugin library does not support plugin ABI {ABI_VERSION}"));
        }
        if status != YACE_OK {
            return Err(format!("the plugin failed to initialise ({})", status_name(status)));
        }
        if table.abi_version != ABI_VERSION {
            return Err(format!("the plugin answered with ABI {} (expected {ABI_VERSION})", table.abi_version));
        }
        if (table.size as usize) < PLUGIN_V1_SIZE {
            return Err(format!("the plugin's callback table is too small ({} < {PLUGIN_V1_SIZE} bytes)", table.size));
        }
        let mut loaded =
            Loaded { table, _host: host, lib: Some(lib), unloadable, stats: Default::default(), shut_down: false };
        let took = start.elapsed();
        loaded.record(Callback::Init, took);
        Ok((loaded, took))
    }

    fn record(&mut self, cb: Callback, d: Duration) {
        let st = self.stats.entry(cb).or_default();
        st.calls += 1;
        st.total += d;
        st.max = st.max.max(d);
        if d > SLOW {
            st.slow += 1;
        }
    }

    fn timed<R>(&mut self, cb: Callback, f: impl FnOnce(&YacePlugin) -> R) -> (R, Duration) {
        let start = Instant::now();
        let r = f(&self.table);
        let d = start.elapsed();
        self.record(cb, d);
        (r, d)
    }

    pub fn instance(&self) -> *mut c_void {
        self.table.instance
    }

    pub fn activate(&mut self) -> (YaceStatus, Duration) {
        self.timed(Callback::Activate, |t| match t.activate {
            Some(f) => f(t.instance),
            None => YACE_OK,
        })
    }

    pub fn has_command_handler(&self) -> bool {
        self.table.on_command.is_some()
    }

    pub fn command(&mut self, id: &str, args: &str) -> (YaceStatus, Duration) {
        self.timed(Callback::Command, |t| match t.on_command {
            Some(f) => f(t.instance, s(id), s(args)),
            None => yace_plugin_api::YACE_E_UNSUPPORTED,
        })
    }

    pub fn event(&mut self, json: &str) -> Duration {
        self.timed(Callback::Event, |t| {
            if let Some(f) = t.on_event {
                f(t.instance, s(json));
            }
        })
        .1
    }

    pub fn message(&mut self, data: &str) -> Duration {
        self.timed(Callback::Message, |t| {
            if let Some(f) = t.on_message {
                f(t.instance, s(data));
            }
        })
        .1
    }

    pub fn view_action(&mut self, view: &str, item: &str, action: &str) -> (YaceStatus, Duration) {
        self.timed(Callback::ViewAction, |t| match t.on_view_action {
            Some(f) => f(t.instance, s(view), s(item), s(action)),
            None => yace_plugin_api::YACE_E_UNSUPPORTED,
        })
    }

    pub fn quick_open(&mut self, provider: &str, query: &str, request: u64) -> (YaceStatus, Duration) {
        self.timed(Callback::QuickOpen, |t| match t.on_quick_open {
            Some(f) => f(t.instance, s(provider), s(query), request),
            None => yace_plugin_api::YACE_E_UNSUPPORTED,
        })
    }

    /// Deactivate and destroy the instance. The library stays mapped unless
    /// the plugin declared `unloadable = true`: unloading Rust code that
    /// left thread-locals or threads behind would crash the editor later.
    pub fn shutdown(&mut self) -> Duration {
        if self.shut_down {
            return Duration::ZERO;
        }
        self.shut_down = true;
        let (_, d) = self.timed(Callback::Deactivate, |t| {
            if let Some(f) = t.deactivate {
                f(t.instance);
            }
            if let Some(f) = t.destroy {
                f(t.instance);
            }
        });
        if let Some(lib) = self.lib.take()
            && !self.unloadable
        {
            std::mem::forget(lib);
        }
        d
    }
}

impl Drop for Loaded {
    fn drop(&mut self) {
        self.shutdown();
    }
}

#[cfg(unix)]
#[allow(unsafe_code)]
unsafe fn open_now(path: &Path) -> Result<libloading::Library, libloading::Error> {
    use libloading::os::unix::{Library, RTLD_LOCAL, RTLD_NOW};
    // RTLD_NOW: missing symbols fail here, not later in the middle of a call.
    // SAFETY: see `Loaded::open`.
    unsafe { Library::open(Some(path), RTLD_NOW | RTLD_LOCAL).map(Into::into) }
}

#[cfg(not(unix))]
#[allow(unsafe_code)]
unsafe fn open_now(path: &Path) -> Result<libloading::Library, libloading::Error> {
    // SAFETY: see `Loaded::open`.
    unsafe { libloading::Library::new(path) }
}

/// A tree-sitter grammar function from a shared library (Helix/Neovim
/// layout: `tree_sitter_<lang>`). The library stays loaded for the rest of
/// the session.
pub type GrammarFn = unsafe extern "C" fn() -> *const ();

#[allow(unsafe_code)]
pub fn open_grammar(path: &Path, symbol: &str) -> Result<GrammarFn, String> {
    // SAFETY: as for plugins; the caller verified the file's hash.
    let lib = unsafe { open_now(path) }.map_err(|e| format!("could not load {}: {e}", path.display()))?;
    // SAFETY: tree-sitter grammars export `const TSLanguage *tree_sitter_x(void)`.
    let f: GrammarFn = unsafe {
        *lib.get::<GrammarFn>(symbol.as_bytes()).map_err(|_| format!("{} does not export {symbol}", path.display()))?
    };
    std::mem::forget(lib);
    Ok(f)
}
