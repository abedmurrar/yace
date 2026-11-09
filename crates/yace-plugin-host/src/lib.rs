//! The YACE native plugin host (spec §18): plugin manifests, discovery of
//! installed plugins, trust records, install/uninstall/packaging, loading
//! libraries with ABI checks, timed calls and background jobs. The editor
//! (`crates/yace/src/plugins.rs`) supplies the host function table and
//! wires plugins into the UI.
//!
//! In-process native plugins are **not sandboxed** (spec §18.9): they run
//! with the editor's permissions and can crash it. The host refuses
//! incompatible or modified code, measures every call and lets the user
//! disable plugins or start without them (`--safe-mode`).

pub mod jobs;
pub mod loader;
pub mod manifest;
pub mod registry;
pub mod store;

pub use jobs::Jobs;
pub use loader::{CallStat, Callback, GrammarFn, Loaded, SLOW, open_grammar, status_name};
pub use manifest::{MANIFEST_FILE, Manifest};
pub use store::{Discovered, InstallPlan, TrustRecord};
