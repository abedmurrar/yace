//! Blobs, diffs and operations. Everything that changes a repository is
//! here and runs only on explicit user action (spec §15.4).

use std::path::Path;

use crate::exec::{Git, GitError};

/// A file's text in the index (`stage = true`) or in HEAD. `None` when it
/// isn't there (new file) or isn't text.
pub fn base_text(git: &Git, toplevel: &Path, rel: &str, index: bool) -> Result<Option<String>, GitError> {
    let spec = if index { format!(":{rel}") } else { format!("HEAD:{rel}") };
    match git.read(toplevel, &["show", &spec]) {
        Ok(bytes) => Ok(String::from_utf8(bytes).ok()),
        Err(GitError::Failed { .. }) => Ok(None),
        Err(e) => Err(e),
    }
}

/// Unified diff of a file: working tree vs index, or staged (index vs HEAD).
pub fn diff_text(git: &Git, toplevel: &Path, rel: &str, staged: bool) -> Result<String, GitError> {
    let mut args = vec!["diff", "--no-ext-diff", "--no-color", "-U3"];
    if staged {
        args.push("--cached");
    }
    args.extend(["--", rel]);
    Ok(String::from_utf8_lossy(&git.read(toplevel, &args)?).into_owned())
}

/// Diff of an untracked file (shown as all-added).
pub fn untracked_diff(git: &Git, toplevel: &Path, rel: &str) -> Result<String, GitError> {
    let args = ["diff", "--no-ext-diff", "--no-color", "--no-index", "--", "/dev/null", rel];
    Ok(String::from_utf8_lossy(&git.read_ok(toplevel, &args, &[1])?).into_owned())
}

pub fn stage(git: &Git, toplevel: &Path, paths: &[&str]) -> Result<(), GitError> {
    let mut args = vec!["add", "--all", "--"];
    args.extend(paths);
    git.write(toplevel, &args, None).map(drop)
}

pub fn unstage(git: &Git, toplevel: &Path, paths: &[&str], has_head: bool) -> Result<(), GitError> {
    let mut args = if has_head { vec!["restore", "--staged", "--"] } else { vec!["rm", "--cached", "-r", "-q", "--"] };
    args.extend(paths);
    git.write(toplevel, &args, None).map(drop)
}

/// Commit what is staged (or everything tracked with `all`). The message
/// goes through stdin, so no editor and no shell quoting are involved.
pub fn commit(git: &Git, toplevel: &Path, message: &str, all: bool) -> Result<String, GitError> {
    let mut args = vec!["commit", "-F", "-"];
    if all {
        args.push("--all");
    }
    let out = git.write(toplevel, &args, Some(message.as_bytes()))?;
    Ok(String::from_utf8_lossy(&out).lines().next().unwrap_or("").to_string())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Branch {
    /// `main`, or `origin/feature` for remote branches.
    pub name: String,
    pub remote: bool,
    pub current: bool,
    pub upstream: Option<String>,
    pub commit: String,
}

pub fn branches(git: &Git, toplevel: &Path) -> Result<Vec<Branch>, GitError> {
    let out = git.read(
        toplevel,
        &[
            "for-each-ref",
            "--sort=-committerdate",
            "--format=%(refname)%00%(refname:short)%00%(objectname:short)%00%(upstream:short)%00%(HEAD)",
            "refs/heads",
            "refs/remotes",
        ],
    )?;
    let text = String::from_utf8_lossy(&out);
    let mut v = Vec::new();
    for line in text.lines() {
        let f: Vec<&str> = line.split('\0').collect();
        if f.len() < 5 || f[0].ends_with("/HEAD") {
            continue;
        }
        v.push(Branch {
            name: f[1].to_string(),
            remote: f[0].starts_with("refs/remotes/"),
            current: f[4] == "*",
            upstream: (!f[3].is_empty()).then(|| f[3].to_string()),
            commit: f[2].to_string(),
        });
    }
    Ok(v)
}

/// Switch to a branch; a remote branch (`origin/x`) creates a local
/// tracking branch `x`. git refuses when local changes would be lost.
pub fn switch(git: &Git, toplevel: &Path, branch: &Branch) -> Result<(), GitError> {
    if branch.remote {
        let local = branch.name.split_once('/').map(|(_, b)| b).unwrap_or(&branch.name);
        git.write(toplevel, &["switch", "--track", "-c", local, &branch.name], None).map(drop)
    } else {
        git.write(toplevel, &["switch", &branch.name], None).map(drop)
    }
}

pub fn create_branch(git: &Git, toplevel: &Path, name: &str) -> Result<(), GitError> {
    git.write(toplevel, &["switch", "-c", name], None).map(drop)
}

/// A branch name git accepts (checked by git itself).
pub fn valid_branch_name(git: &Git, toplevel: &Path, name: &str) -> bool {
    !name.trim().is_empty() && git.read(toplevel, &["check-ref-format", "--branch", name]).is_ok()
}
