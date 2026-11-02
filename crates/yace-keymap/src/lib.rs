//! # yace-keymap
//!
//! Keyboard handling independent of the terminal backend:
//! [`KeyPress`]/[`KeySeq`] parsing (VS Code syntax), [`When`] clauses,
//! layered [`Keymap`]s with removal and conflict detection, and a chord-aware
//! [`Resolver`]. The default table mirrors VS Code's Linux/Windows defaults.

pub mod defaults;
pub mod key;
pub mod keymap;
pub mod when;

pub use defaults::{ctx, default_keymap};
pub use key::{KeyCode, KeyPress, KeySeq, Mods, ParseError};
pub use keymap::{Binding, BindingError, BindingSpec, Conflict, Keymap, Resolution, Resolver, Source};
pub use when::{Context, ContextSet, When};
