//! # yace-buffer
//!
//! The YACE text engine. Pure (no terminal or UI dependencies) so that every
//! editing behaviour can be unit-tested in isolation.
//!
//! - [`Document`]: rope + history + file metadata
//! - [`Selection`] / [`Range`]: multi-cursor selections in char indices
//! - [`ChangeSet`]: batched replacements; apply / invert / map positions
//! - [`movement`] and [`ops`]: VS Code-style cursor movement and edits
//! - [`io`]: UTF-8 (+BOM) loading and atomic, permission-preserving saving

pub mod changes;
pub mod document;
pub mod history;
pub mod io;
pub mod line_ending;
pub mod movement;
pub mod ops;
pub mod selection;
pub mod text;

pub use changes::{Assoc, Change, ChangeSet};
pub use document::{Applied, DiskState, Document};
pub use history::EditKind;
pub use line_ending::{LineEnding, LineEndingInfo};
pub use ropey::{Rope, RopeSlice};
pub use selection::{Range, Selection};
