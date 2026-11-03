//! # yace-syntax
//!
//! Syntax support for YACE (spec §12), built on
//! [tree-sitter](https://tree-sitter.github.io) and the grammars published by
//! the tree-sitter organisation and community:
//! - [`language`]: registry, detection and per-language editing rules
//! - [`syntax`]: incremental parsing from a rope, with a time budget
//! - [`highlight`]: viewport-bounded highlighting with injections

pub mod highlight;
pub mod language;
pub mod syntax;

pub use highlight::{InjectionCache, Scope, Span, highlight};
pub use language::{Associations, EditingRules, LANGUAGES, LanguageConfig, by_id, detect, plain_text};
pub use syntax::{ParseOutcome, Syntax, input_edits, parse_full};
pub use tree_sitter::{InputEdit, Tree};
