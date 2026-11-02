//! # yace-workspace
//!
//! Multi-root workspaces (spec §4): a workspace is a named collection of
//! independent roots that need not share a parent directory. Includes
//! read/write compatibility with VS Code `.code-workspace` files (JSONC,
//! relative paths resolved against the file, non-destructive edits) and the
//! persistent session state used to restore a workspace.

pub mod paths;
pub mod session;
pub mod workspace;

pub use workspace::{Root, RootId, RootStatus, Workspace, WorkspaceError, WorkspaceKind};
