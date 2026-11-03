//! # yace-search
//!
//! Search for YACE, independent of the UI:
//! - [`fuzzy`]: VS Code-style fuzzy matching of paths and labels
//! - [`walk`]: parallel, ignore-aware, bounded, cancellable file discovery
//! - [`index`]: the Quick Open file-name index (top-K queries, warm cache)
//! - [`content`]: streamed workspace content search and replace on disk
//! - [`find`]: find and replace within a single document

pub mod content;
pub mod find;
pub mod fuzzy;
pub mod index;
pub mod walk;

pub use regex;

pub use content::{ContentQuery, FileMatches, LineMatch, SearchRoot, SearchSummary};
pub use find::{FindOptions, FindResult};
pub use index::{FileIndex, Hit, QueryContext, RootKey};
pub use walk::{WalkOptions, WalkSummary};
