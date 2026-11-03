//! # yace-lsp
//!
//! The language server client for YACE (spec §13):
//! - [`jsonrpc`]: `Content-Length` framing
//! - [`client`]: one server process with reader/writer/stderr threads;
//!   responses are decoded into typed [`decode::Reply`]s off the UI thread
//! - [`types`] / [`decode`]: the protocol subset YACE uses, leniently parsed
//! - [`position`]: rope ↔ LSP positions in UTF-8/16/32
//! - [`uri`]: `file://` URIs
//! - [`registry`]: built-in server table, `PATH` lookup, install hints
//!
//! Servers are external programs (some need Node.js); YACE starts them on
//! demand, one per workspace root, and never installs anything.

pub mod client;
pub mod decode;
pub mod jsonrpc;
pub mod position;
pub mod registry;
pub mod types;
pub mod uri;

pub use client::{Client, Launch};
pub use decode::{Incoming, Reply};
pub use position::{Encoding, from_lsp, to_lsp};
pub use registry::ServerDef;
pub use types::*;
