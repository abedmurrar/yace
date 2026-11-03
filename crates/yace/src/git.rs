//! Git state of the workspace and the jobs that update it (spec §15, §4.7).
//! Every job runs `git` on a worker thread (the test harness runs them
//! inline); results come back as `AppEvent::Git`. Each repository is
//! refreshed independently, at most one status at a time, coalesced.

use std::{collections::HashMap, path::PathBuf, sync::Arc, time::Instant};

use crossbeam_channel::{Receiver, Sender, unbounded};
use yace_buffer::Rope;
use yace_git::{Branch, Git, GitError, Hunk, Kind, RepoInfo, Status};
use yace_workspace::RootId;

use crate::{app::AppEvent, editor::DocId};

/// A unit of git work.
#[derive(Debug, Clone)]
pub enum GitJob {
    /// Which repository each root belongs to.
    Detect {
        roots: Vec<(RootId, PathBuf)>,
    },
    Status {
        generation: u64,
        toplevel: PathBuf,
    },
    /// A document's base text (index, falling back to HEAD).
    Base {
        doc: DocId,
        rel: String,
        toplevel: PathBuf,
        generation: u64,
    },
    /// Line hunks of a document's current text against its base.
    Diff {
        doc: DocId,
        version: u64,
        base: Arc<str>,
        text: Rope,
    },
    /// Text of a changes document (`git diff`).
    DiffText {
        toplevel: PathBuf,
        rel: String,
        staged: bool,
        untracked: bool,
        title: String,
        file: PathBuf,
    },
    Branches {
        toplevel: PathBuf,
    },
    /// An operation the user asked for.
    Op {
        toplevel: PathBuf,
        op: GitOp,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub enum GitOp {
    Stage(Vec<String>),
    Unstage { paths: Vec<String>, has_head: bool },
    Commit { message: String, all: bool },
    Switch(Branch),
    CreateBranch(String),
}

impl GitOp {
    pub fn describe(&self) -> String {
        match self {
            GitOp::Stage(p) => format!("Staged {}", count(p.len())),
            GitOp::Unstage { paths, .. } => format!("Unstaged {}", count(paths.len())),
            GitOp::Commit { .. } => "Committed".into(),
            GitOp::Switch(b) => format!("Switched to {}", b.name),
            GitOp::CreateBranch(n) => format!("Created and switched to {n}"),
        }
    }
}

fn count(n: usize) -> String {
    if n == 1 { "1 file".into() } else { format!("{n} files") }
}

/// A finished job.
#[derive(Debug, Clone)]
pub enum GitResult {
    Detected(Result<Vec<(RootId, Option<RepoInfo>)>, GitError>),
    /// With the Explorer decorations, built on the worker too.
    Status {
        toplevel: PathBuf,
        generation: u64,
        result: Result<(Status, HashMap<String, Kind>), GitError>,
    },
    Base {
        doc: DocId,
        generation: u64,
        text: Option<Arc<str>>,
    },
    Hunks {
        doc: DocId,
        version: u64,
        hunks: Vec<Hunk>,
    },
    DiffText {
        title: String,
        file: PathBuf,
        result: Result<String, GitError>,
    },
    Branches {
        toplevel: PathBuf,
        result: Result<Vec<Branch>, GitError>,
    },
    Op {
        toplevel: PathBuf,
        op: GitOp,
        result: Result<String, GitError>,
    },
}

/// Run one job (on a worker, or inline in tests).
pub fn run_job(git: &Git, job: GitJob) -> GitResult {
    match job {
        GitJob::Detect { roots } => GitResult::Detected(
            roots
                .into_iter()
                .map(|(id, path)| yace_git::detect(git, &path).map(|r| (id, r)))
                .collect::<Result<Vec<_>, _>>(),
        ),
        GitJob::Status { generation, toplevel } => {
            let result = yace_git::status(git, &toplevel).map(|st| {
                let d = decorations(&st);
                (st, d)
            });
            GitResult::Status { toplevel, generation, result }
        }
        GitJob::Base { doc, rel, toplevel, generation } => {
            let text = yace_git::ops::base_text(git, &toplevel, &rel, true)
                .ok()
                .flatten()
                .or_else(|| yace_git::ops::base_text(git, &toplevel, &rel, false).ok().flatten());
            GitResult::Base { doc, generation, text: text.map(Arc::from) }
        }
        GitJob::Diff { doc, version, base, text } => {
            let current = text.to_string();
            GitResult::Hunks { doc, version, hunks: yace_git::line_hunks(&base, &current) }
        }
        GitJob::DiffText { toplevel, rel, staged, untracked, title, file } => {
            let result = if untracked {
                yace_git::ops::untracked_diff(git, &toplevel, &rel)
            } else {
                yace_git::ops::diff_text(git, &toplevel, &rel, staged)
            };
            GitResult::DiffText { title, file, result }
        }
        GitJob::Branches { toplevel } => {
            let result = yace_git::ops::branches(git, &toplevel);
            GitResult::Branches { toplevel, result }
        }
        GitJob::Op { toplevel, op } => {
            let t = &toplevel;
            let result = match &op {
                GitOp::Stage(p) => {
                    let p: Vec<&str> = p.iter().map(String::as_str).collect();
                    yace_git::ops::stage(git, t, &p).map(|_| String::new())
                }
                GitOp::Unstage { paths, has_head } => {
                    let p: Vec<&str> = paths.iter().map(String::as_str).collect();
                    yace_git::ops::unstage(git, t, &p, *has_head).map(|_| String::new())
                }
                GitOp::Commit { message, all } => yace_git::ops::commit(git, t, message, *all),
                GitOp::Switch(b) => yace_git::ops::switch(git, t, b).map(|_| String::new()),
                GitOp::CreateBranch(n) => yace_git::ops::create_branch(git, t, n).map(|_| String::new()),
            };
            GitResult::Op { toplevel, op, result }
        }
    }
}

/// Worker threads that run git jobs (created on first use).
/// A job and the git program to run it with.
pub type GitTask = (Git, GitJob);

pub fn spawn_git_workers(events: Sender<AppEvent>) -> Sender<GitTask> {
    let (tx, rx): (Sender<GitTask>, Receiver<GitTask>) = unbounded();
    for i in 0..2 {
        let rx = rx.clone();
        let events = events.clone();
        let _ = std::thread::Builder::new().name(format!("yace-git-{i}")).spawn(move || {
            for (git, job) in rx {
                if events.send(AppEvent::Git(Box::new(run_job(&git, job)))).is_err() {
                    return;
                }
            }
        });
    }
    tx
}

/// One repository and what we know about it.
#[derive(Debug, Clone)]
pub struct RepoState {
    pub info: RepoInfo,
    pub name: String,
    /// Workspace roots in this repository, with the root's path relative to
    /// the repository top level (`""` or `"sub/dir/"`).
    pub roots: Vec<(RootId, String)>,
    pub status: Option<Status>,
    pub error: Option<String>,
    pub in_flight: bool,
    /// A refresh was requested while one was running.
    pub again: bool,
    /// When the next status should run.
    pub due: Option<Instant>,
    /// Bumped when the index or HEAD may have changed (bases are stale).
    pub generation: u64,
    /// Repository-relative path → state shown in the Explorer (folders
    /// containing changes included; ignored folders without `/`).
    pub decor: HashMap<String, Kind>,
}

/// Repository-relative path → state shown in the Explorer, with folders
/// taking the most important state of their contents.
pub fn decorations(st: &Status) -> HashMap<String, Kind> {
    let mut decor = HashMap::with_capacity(st.files.len() * 2);
    for f in &st.files {
        let path = f.path.trim_end_matches('/');
        let kind = f.kind();
        decor.insert(path.to_string(), kind);
        if kind == Kind::Ignored {
            continue;
        }
        let mut p = path;
        while let Some(i) = p.rfind('/') {
            p = &p[..i];
            let e = decor.entry(p.to_string()).or_insert(kind);
            if rank(kind) < rank(*e) {
                *e = kind;
            }
        }
    }
    decor
}

/// Importance of a state for folder colours (lower wins).
fn rank(k: Kind) -> u8 {
    match k {
        Kind::Conflicted => 0,
        Kind::Modified | Kind::Deleted | Kind::Renamed => 1,
        Kind::Added | Kind::Untracked => 2,
        Kind::Ignored => 3,
    }
}

/// Git state of one open document.
#[derive(Debug, Clone, Default)]
pub struct DocGit {
    pub repo: usize,
    /// Repository-relative path.
    pub rel: String,
    /// Base text (index or HEAD); `None` for files git doesn't track.
    pub base: Option<Arc<str>>,
    /// Repository generation of the last base requested.
    pub base_gen: u64,
    pub base_pending: bool,
    /// A base answer arrived (`base` may still be `None`: untracked).
    pub base_loaded: bool,
    pub hunks: Vec<Hunk>,
    /// Document version the hunks were computed for.
    pub hunks_version: u64,
    pub diff_pending: bool,
    pub diff_due: Option<Instant>,
}

#[derive(Debug, Default)]
pub struct GitState {
    pub git: Git,
    pub repos: Vec<RepoState>,
    pub root_repo: HashMap<RootId, usize>,
    pub docs: HashMap<DocId, DocGit>,
    /// Detection ran for the current roots.
    pub detected: bool,
    pub detecting: bool,
    /// The git program is missing (hint shown once).
    pub missing: bool,
    /// Bumped on every detection, to drop results for old repo indices.
    pub epoch: u64,
    /// Last periodic refresh while the Source Control view is visible.
    pub last_poll: Option<Instant>,
    /// Branches listed by the branch picker: (repo, branches).
    pub branch_pick: Option<(usize, Vec<Branch>)>,
}
