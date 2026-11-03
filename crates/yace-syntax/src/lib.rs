//! # yace-syntax
//!
//! Syntax support for YACE (spec §12), built on
//! [tree-sitter](https://tree-sitter.github.io) and the grammars published by
//! the tree-sitter organisation and community:
//! - [`language`]: registry, detection and per-language editing rules
//! - [`syntax`]: incremental parsing from a rope, with a time budget
//! - [`highlight`]: viewport-bounded highlighting with injections
//! - [`fold`]: folding ranges from the tree (or indentation)

pub mod fold;
pub mod highlight;
pub mod language;
pub mod syntax;

pub use highlight::{InjectionCache, Scope, Span, highlight};
pub use language::{
    Associations, EditingRules, LANGUAGES, LanguageConfig, LanguageDef, LoadedGrammar, PACKAGED, by_id, detect,
    has_external_grammar, plain_text, register_grammar, register_language,
};
pub use syntax::{ParseOutcome, Syntax, input_edits, parse_full};
pub use tree_sitter::Language;
pub use tree_sitter::{InputEdit, Tree};
pub use tree_sitter_language::LanguageFn;
