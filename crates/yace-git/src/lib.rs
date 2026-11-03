//! Git support for YACE (spec §15): runs the user's `git` program on worker
//! threads. Inspection (detect, status, show, diff) is read-only and never
//! takes the index lock; operations (stage, commit, switch) are separate
//! functions the editor calls only on explicit user action. Each call is
//! scoped to one repository (spec §4.7).

pub mod diff;
pub mod exec;
pub mod ops;
pub mod repo;
pub mod status;

pub use diff::{Hunk, HunkKind, line_hunks};
pub use exec::{Git, GitError, LogEntry};
pub use ops::Branch;
pub use repo::{RepoInfo, detect};
pub use status::{FileStatus, Kind, Status, status};
