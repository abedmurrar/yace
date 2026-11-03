//! Repository detection (spec §4.7): each workspace root belongs to at
//! most one repository; nested repositories are not scanned.

use std::path::{Path, PathBuf};

use crate::exec::{Git, GitError};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepoInfo {
    /// The working tree's top level.
    pub toplevel: PathBuf,
    /// Absolute path of the `.git` directory (or worktree git dir).
    pub git_dir: PathBuf,
}

/// The repository containing `dir`, if any. Not being in a repository is
/// `Ok(None)`; a missing git program is an error.
pub fn detect(git: &Git, dir: &Path) -> Result<Option<RepoInfo>, GitError> {
    match git.read(dir, &["rev-parse", "--show-toplevel", "--absolute-git-dir"]) {
        Ok(out) => {
            let text = String::from_utf8_lossy(&out);
            let mut lines = text.lines();
            match (lines.next(), lines.next()) {
                (Some(top), Some(gd)) if !top.is_empty() => {
                    Ok(Some(RepoInfo { toplevel: PathBuf::from(top), git_dir: PathBuf::from(gd) }))
                }
                _ => Ok(None),
            }
        }
        // Outside a repository, or inside `.git` / a bare repository.
        Err(GitError::Failed { .. }) => Ok(None),
        Err(e) => Err(e),
    }
}
