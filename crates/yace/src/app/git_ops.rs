//! Git views and operations (spec §15.1, §15.2): the Source Control view,
//! changes documents, stage/unstage, commit, branches. Every operation runs
//! only from an explicit command or key (spec §15.4) and is scoped to one
//! repository (spec §4.7).

use std::{
    collections::HashSet,
    path::{Path, PathBuf},
    time::Duration,
};

use yace_git::{Branch, GitError, Kind};

use super::{App, Focus, Severity, SidebarView};
use crate::{
    actions::{ConfirmAction, PickerAction, PromptAction},
    git::{GitJob, GitOp},
    overlay::{Confirm, Overlay, Picker, PickerItem},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ScmGroup {
    Conflicts,
    Staged,
    Changes,
    Untracked,
}

impl ScmGroup {
    pub fn title(self) -> &'static str {
        match self {
            ScmGroup::Conflicts => "Merge Conflicts",
            ScmGroup::Staged => "Staged Changes",
            ScmGroup::Changes => "Changes",
            ScmGroup::Untracked => "Untracked",
        }
    }
}

/// A row of the Source Control view.
#[derive(Debug, Clone, PartialEq)]
pub enum ScmRow {
    Repo(usize),
    Group { repo: usize, group: ScmGroup, count: usize },
    File { repo: usize, group: ScmGroup, path: String, kind: Kind },
}

/// Selection and collapsed sections of the Source Control view.
#[derive(Debug, Clone, Default)]
pub struct ScmUi {
    pub selected: usize,
    /// `toplevel` or `toplevel#Group` keys.
    pub collapsed: HashSet<String>,
}

/// A repository command that needs to know which repository.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RepoCmd {
    Commit { all: bool },
    Checkout,
    CreateBranch,
    StageAll,
    UnstageAll,
}

fn group_key(top: &Path, g: Option<ScmGroup>) -> String {
    match g {
        Some(g) => format!("{}#{g:?}", top.display()),
        None => top.display().to_string(),
    }
}

impl App {
    // ------------------------------------------------------------------
    // Source Control view

    pub fn scm_rows(&self) -> Vec<ScmRow> {
        let mut rows = Vec::new();
        for (i, r) in self.git.repos.iter().enumerate() {
            rows.push(ScmRow::Repo(i));
            if self.scm.collapsed.contains(&group_key(&r.info.toplevel, None)) {
                continue;
            }
            let Some(st) = &r.status else { continue };
            for g in [ScmGroup::Conflicts, ScmGroup::Staged, ScmGroup::Changes, ScmGroup::Untracked] {
                let files: Vec<(String, Kind)> = st
                    .files
                    .iter()
                    .filter_map(|f| match g {
                        ScmGroup::Conflicts if f.conflict => Some((f.path.clone(), Kind::Conflicted)),
                        ScmGroup::Staged if f.staged() => Some((f.path.clone(), f.staged_kind())),
                        ScmGroup::Changes if f.unstaged() => Some((f.path.clone(), f.unstaged_kind())),
                        ScmGroup::Untracked if f.untracked => Some((f.path.clone(), Kind::Untracked)),
                        _ => None,
                    })
                    .collect();
                if files.is_empty() {
                    continue;
                }
                rows.push(ScmRow::Group { repo: i, group: g, count: files.len() });
                if self.scm.collapsed.contains(&group_key(&r.info.toplevel, Some(g))) {
                    continue;
                }
                for (path, kind) in files {
                    rows.push(ScmRow::File { repo: i, group: g, path, kind });
                }
            }
        }
        rows
    }

    /// Ctrl+Shift+G: show the Source Control view.
    pub(crate) fn show_scm(&mut self) {
        self.sidebar_view = SidebarView::Git;
        self.sidebar_visible = true;
        self.focus_changed(Focus::Explorer);
        self.git.last_poll = None; // refresh now
        if !self.git.detected && !self.git.detecting {
            self.git_workspace_changed();
        }
        self.needs_render = true;
    }

    fn scm_selected(&self) -> Option<ScmRow> {
        let rows = self.scm_rows();
        rows.get(self.scm.selected.min(rows.len().saturating_sub(1))).cloned()
    }

    /// Keys while the Source Control view has focus.
    pub(crate) fn scm_key(&mut self, key: yace_keymap::KeyPress, ch: Option<char>) -> bool {
        use yace_keymap::KeyCode as K;
        if !(key.mods.is_empty() || key.mods == yace_keymap::Mods::SHIFT) {
            return false;
        }
        let n = self.scm_rows().len();
        let sel = self.scm.selected.min(n.saturating_sub(1));
        match key.code {
            K::Up => self.scm.selected = sel.saturating_sub(1),
            K::Down => self.scm.selected = (sel + 1).min(n.saturating_sub(1)),
            K::PageUp => self.scm.selected = sel.saturating_sub(10),
            K::PageDown => self.scm.selected = (sel + 10).min(n.saturating_sub(1)),
            K::Home => self.scm.selected = 0,
            K::End => self.scm.selected = n.saturating_sub(1),
            K::Left | K::Right => self.scm_toggle(key.code == K::Right),
            K::Enter | K::Space => self.scm_open(sel, false),
            K::Char(_) => match ch.unwrap_or(' ') {
                'o' => self.scm_open(sel, true),
                's' => self.scm_stage_selected(true),
                'u' => self.scm_stage_selected(false),
                'c' => self.scm_repo_cmd(RepoCmd::Commit { all: false }),
                'b' => self.scm_repo_cmd(RepoCmd::Checkout),
                'r' => self.git_refresh(),
                _ => return false,
            },
            _ => return false,
        }
        self.needs_render = true;
        true
    }

    fn scm_toggle(&mut self, open: bool) {
        let key = match self.scm_selected() {
            Some(ScmRow::Repo(i)) => group_key(&self.git.repos[i].info.toplevel, None),
            Some(ScmRow::Group { repo, group, .. }) => group_key(&self.git.repos[repo].info.toplevel, Some(group)),
            Some(ScmRow::File { .. }) if !open => {
                // Left on a file: go to its group.
                let rows = self.scm_rows();
                if let Some(p) = (0..self.scm.selected).rev().find(|&k| matches!(rows[k], ScmRow::Group { .. })) {
                    self.scm.selected = p;
                }
                return;
            }
            _ => return,
        };
        if open {
            self.scm.collapsed.remove(&key);
        } else {
            self.scm.collapsed.insert(key);
        }
    }

    /// Enter on a row: toggle a section, or open a file's changes (or the
    /// file itself with `file`).
    pub(crate) fn scm_open(&mut self, row: usize, file: bool) {
        let rows = self.scm_rows();
        self.scm.selected = row.min(rows.len().saturating_sub(1));
        match rows.get(row).cloned() {
            Some(ScmRow::File { repo, group, path, .. }) => {
                let full = self.git.repos[repo].info.toplevel.join(path.trim_end_matches('/'));
                if file || group == ScmGroup::Conflicts {
                    if full.is_file() {
                        self.open_file(&full, false);
                        self.focus_editor();
                    } else {
                        self.notify(Severity::Info, format!("{} is not a file in the working tree", full.display()));
                    }
                } else {
                    self.git_open_changes(repo, &path, group);
                }
            }
            Some(ScmRow::Repo(_)) | Some(ScmRow::Group { .. }) => {
                let key = match &rows[row] {
                    ScmRow::Repo(i) => group_key(&self.git.repos[*i].info.toplevel, None),
                    ScmRow::Group { repo, group, .. } => group_key(&self.git.repos[*repo].info.toplevel, Some(*group)),
                    _ => return,
                };
                if !self.scm.collapsed.remove(&key) {
                    self.scm.collapsed.insert(key);
                }
            }
            None => {}
        }
    }

    fn scm_stage_selected(&mut self, stage: bool) {
        let rows = self.scm_rows();
        let Some(row) = rows.get(self.scm.selected).cloned() else { return };
        let (repo, paths) = match row {
            ScmRow::File { repo, group, path, .. } => {
                if stage == (group == ScmGroup::Staged) {
                    return; // already there
                }
                (repo, vec![path])
            }
            ScmRow::Group { repo, group, .. } => {
                let paths: Vec<String> = rows
                    .iter()
                    .filter_map(|r| match r {
                        ScmRow::File { repo: rr, group: g, path, .. } if *rr == repo && *g == group => {
                            Some(path.clone())
                        }
                        _ => None,
                    })
                    .collect();
                if paths.is_empty() || stage == (group == ScmGroup::Staged) {
                    return;
                }
                (repo, paths)
            }
            ScmRow::Repo(repo) => {
                self.git_stage_all(repo, stage);
                return;
            }
        };
        self.git_stage(repo, paths, stage);
    }

    // ------------------------------------------------------------------
    // Which repository?

    /// The repository a command applies to: the Source Control selection,
    /// the active file's repository, or the only one. Otherwise ask.
    fn git_repo_for_command(&mut self, cmd: RepoCmd) -> Option<usize> {
        if self.git.repos.is_empty() {
            self.notify(Severity::Info, "No Git repository in this workspace");
            return None;
        }
        if self.focus == Focus::Explorer && self.sidebar_view == SidebarView::Git {
            return self.scm_selected().map(|r| match r {
                ScmRow::Repo(i) | ScmRow::Group { repo: i, .. } | ScmRow::File { repo: i, .. } => i,
            });
        }
        let from_doc = self
            .active()
            .and_then(|(_, _, d)| self.docs.doc(d).and_then(|x| x.path().map(Path::to_path_buf)))
            .and_then(|p| self.git_repo_of(&p).map(|x| x.0));
        if let Some(i) = from_doc {
            return Some(i);
        }
        if self.git.repos.len() == 1 {
            return Some(0);
        }
        let items = self
            .git
            .repos
            .iter()
            .enumerate()
            .map(|(i, r)| {
                let branch = r.status.as_ref().map(|s| s.label()).unwrap_or_default();
                PickerItem::new(r.name.clone(), format!("{branch} · {}", r.info.toplevel.display()), "", i)
            })
            .collect();
        self.open_overlay(Overlay::Picker(Picker::new("Choose a Repository", items, PickerAction::GitRepo(cmd))));
        None
    }

    /// Switch the active file's repository (or the only one) to `branch`
    /// (`git.checkout` with arguments). Never asks which repository.
    pub(crate) fn git_switch_to(&mut self, branch: &str) -> Result<(), String> {
        let from_doc = self
            .active()
            .and_then(|(_, _, d)| self.docs.doc(d).and_then(|x| x.path().map(Path::to_path_buf)))
            .and_then(|p| self.git_repo_of(&p).map(|x| x.0));
        let repo = match (from_doc, self.git.repos.len()) {
            (Some(i), _) => i,
            (None, 1) => 0,
            (None, 0) => return Err("no Git repository in this workspace".into()),
            _ => return Err("several repositories: open a file of the one to switch".into()),
        };
        let b = yace_git::Branch {
            name: branch.to_string(),
            remote: false,
            current: false,
            upstream: None,
            commit: String::new(),
        };
        self.git_op(repo, GitOp::Switch(b));
        Ok(())
    }

    pub(crate) fn scm_repo_cmd(&mut self, cmd: RepoCmd) {
        if let Some(i) = self.git_repo_for_command(cmd) {
            self.git_run_repo_cmd(i, cmd);
        }
    }

    pub(crate) fn git_run_repo_cmd(&mut self, i: usize, cmd: RepoCmd) {
        match cmd {
            RepoCmd::Commit { all } => self.git_commit_start(i, all),
            RepoCmd::Checkout => {
                let toplevel = self.git.repos[i].info.toplevel.clone();
                self.git_job(GitJob::Branches { toplevel });
            }
            RepoCmd::CreateBranch => self.git_create_branch_prompt(i),
            RepoCmd::StageAll => self.git_stage_all(i, true),
            RepoCmd::UnstageAll => self.git_stage_all(i, false),
        }
    }

    // ------------------------------------------------------------------
    // Changes documents

    /// Open the changes of a file: working tree vs index, or staged.
    pub(crate) fn git_open_changes(&mut self, repo: usize, rel: &str, group: ScmGroup) {
        let r = &self.git.repos[repo];
        let name = rel.trim_end_matches('/').rsplit('/').next().unwrap_or(rel).to_string();
        let (title, staged, untracked) = match group {
            ScmGroup::Staged => (format!("{name} (Index)"), true, false),
            ScmGroup::Untracked => (format!("{name} (Untracked)"), false, true),
            _ => (format!("{name} (Working Tree)"), false, false),
        };
        let job = GitJob::DiffText {
            toplevel: r.info.toplevel.clone(),
            rel: rel.to_string(),
            staged,
            untracked,
            title,
            file: r.info.toplevel.join(rel),
        };
        self.git_job(job);
    }

    /// Git: Open Changes — for the active file (or the selection).
    pub(crate) fn git_open_changes_cmd(&mut self) {
        if self.focus == Focus::Explorer && self.sidebar_view == SidebarView::Git {
            self.scm_open(self.scm.selected, false);
            return;
        }
        let Some(path) =
            self.active().and_then(|(_, _, d)| self.docs.doc(d).and_then(|x| x.path().map(Path::to_path_buf)))
        else {
            return;
        };
        let Some((repo, rel)) = self.git_repo_of(&path) else {
            self.notify(Severity::Info, "This file is not in a Git repository");
            return;
        };
        let untracked = self.git.repos[repo]
            .status
            .as_ref()
            .is_some_and(|s| s.files.iter().any(|f| f.untracked && rel.starts_with(f.path.trim_end_matches('/'))));
        self.git_open_changes(repo, &rel, if untracked { ScmGroup::Untracked } else { ScmGroup::Changes });
    }

    pub(crate) fn on_git_diff_text(&mut self, title: String, file: PathBuf, result: Result<String, GitError>) {
        match result {
            Ok(text) if text.trim().is_empty() => self.notify(Severity::Info, format!("No changes in {title}")),
            Ok(text) => self.open_virtual_doc(&title, &text, Some(file)),
            Err(e) => self.git_failed("diff", &e),
        }
    }

    /// Enter in a changes document: open the file at the line under the
    /// cursor (from the hunk header and the lines before it).
    pub(crate) fn git_diff_jump(&mut self) -> bool {
        let Some((_, v, d)) = self.active() else { return false };
        let Some(file) = self.docs.get(d).and_then(|e| e.virtual_doc.as_ref()).and_then(|v| v.file.clone()) else {
            return false;
        };
        let Some(rope) = self.docs.doc(d).map(|x| x.rope().clone()) else { return false };
        let head = self.layout.active_group().views[v].sel.primary().head;
        let cur = rope.char_to_line(head);
        // Find the hunk header above and count new-file lines to here.
        let mut line = None;
        for l in (0..=cur).rev() {
            let text = rope.line(l).to_string();
            if let Some(rest) = text.strip_prefix("@@ ") {
                let start = rest
                    .split_whitespace()
                    .find_map(|p| p.strip_prefix('+'))
                    .and_then(|p| p.split(',').next())
                    .and_then(|n| n.parse::<usize>().ok())
                    .unwrap_or(1);
                let mut n = start;
                for k in l + 1..cur {
                    if !rope.line(k).to_string().starts_with('-') {
                        n += 1;
                    }
                }
                line = Some(n);
                break;
            }
        }
        if !file.is_file() {
            self.notify(Severity::Info, format!("{} is not in the working tree", file.display()));
            return true;
        }
        self.open_file(&file, false);
        if let Some(n) = line {
            self.set_cursor_line_col(n.saturating_sub(1), 0);
            self.center_active_cursor();
        }
        true
    }

    // ------------------------------------------------------------------
    // Stage, unstage, commit

    fn git_op(&mut self, repo: usize, op: GitOp) {
        let toplevel = self.git.repos[repo].info.toplevel.clone();
        self.git_job(GitJob::Op { toplevel, op });
    }

    pub(crate) fn git_stage(&mut self, repo: usize, paths: Vec<String>, stage: bool) {
        let paths: Vec<String> = paths.into_iter().map(|p| p.trim_end_matches('/').to_string()).collect();
        if stage {
            self.git_op(repo, GitOp::Stage(paths));
        } else {
            let has_head = self.git.repos[repo].status.as_ref().is_some_and(|s| s.head.is_some());
            self.git_op(repo, GitOp::Unstage { paths, has_head });
        }
    }

    pub(crate) fn git_stage_all(&mut self, repo: usize, stage: bool) {
        let Some(st) = self.git.repos[repo].status.clone() else { return };
        let paths: Vec<String> = st
            .files
            .iter()
            .filter(|f| if stage { f.unstaged() || f.untracked || f.conflict } else { f.staged() })
            .map(|f| f.path.clone())
            .collect();
        if paths.is_empty() {
            self.notify(Severity::Info, if stage { "Nothing to stage" } else { "Nothing staged" });
            return;
        }
        self.git_stage(repo, paths, stage);
    }

    /// Git: Stage / Unstage — the active file.
    pub(crate) fn git_stage_active(&mut self, stage: bool) {
        if self.focus == Focus::Explorer && self.sidebar_view == SidebarView::Git {
            self.scm_stage_selected(stage);
            return;
        }
        let Some(path) =
            self.active().and_then(|(_, _, d)| self.docs.doc(d).and_then(|x| x.path().map(Path::to_path_buf)))
        else {
            return;
        };
        match self.git_repo_of(&path) {
            Some((repo, rel)) => self.git_stage(repo, vec![rel], stage),
            None => self.notify(Severity::Info, "This file is not in a Git repository"),
        }
    }

    /// Commit: with nothing staged, offer to commit every change (VS
    /// Code's smart commit), then ask for the message.
    fn git_commit_start(&mut self, repo: usize, all: bool) {
        let Some(st) = self.git.repos[repo].status.clone() else {
            self.notify(Severity::Info, "Still reading the repository status; try again");
            return;
        };
        let name = self.git.repos[repo].name.clone();
        let toplevel = self.git.repos[repo].info.toplevel.clone();
        if st.conflicts() > 0 {
            self.notify(Severity::Warning, format!("{name} has merge conflicts; resolve and stage them first"));
            return;
        }
        let staged = st.files.iter().any(|f| f.staged());
        let changed = st.files.iter().any(|f| f.unstaged());
        if !all && !staged {
            if !changed {
                self.notify(Severity::Info, format!("Nothing to commit in {name}"));
                return;
            }
            self.open_overlay(Overlay::Confirm(Confirm {
                title: "Commit".into(),
                message: vec![
                    format!("There are no staged changes in {name}."),
                    "Stage all changes to tracked files and commit them?".into(),
                ],
                buttons: vec![
                    ("Yes, commit all".into(), ConfirmAction::GitCommitAll(toplevel)),
                    ("Cancel".into(), ConfirmAction::Nothing),
                ],
                selected: 0,
                cancel: ConfirmAction::Nothing,
            }));
            return;
        }
        if all && !changed && !staged {
            self.notify(Severity::Info, format!("Nothing to commit in {name}"));
            return;
        }
        self.git_commit_prompt(toplevel, all);
    }

    pub(crate) fn git_commit_prompt(&mut self, toplevel: PathBuf, all: bool) {
        let name =
            self.git.repos.iter().find(|r| r.info.toplevel == toplevel).map(|r| r.name.clone()).unwrap_or_default();
        let branch = self
            .git
            .repos
            .iter()
            .find(|r| r.info.toplevel == toplevel)
            .and_then(|r| r.status.as_ref().and_then(|s| s.branch.clone()))
            .unwrap_or_default();
        let what = if all { "all changes" } else { "staged changes" };
        self.prompt(
            &format!("Commit {what} to {name} ({branch})"),
            "Message (Enter commits, Esc cancels)",
            String::new(),
            PromptAction::GitCommit { toplevel, all },
            false,
        );
    }

    pub(crate) fn git_commit_submit(&mut self, toplevel: PathBuf, all: bool, message: &str) -> Result<(), String> {
        if message.trim().is_empty() {
            return Err("Enter a commit message".into());
        }
        let Some(repo) = self.git.repos.iter().position(|r| r.info.toplevel == toplevel) else {
            return Err("The repository is no longer in the workspace".into());
        };
        self.git_op(repo, GitOp::Commit { message: message.trim().to_string(), all });
        Ok(())
    }

    // ------------------------------------------------------------------
    // Branches

    pub(crate) fn on_git_branches(&mut self, toplevel: &Path, result: Result<Vec<Branch>, GitError>) {
        let Some(repo) = self.git.repos.iter().position(|r| r.info.toplevel == toplevel) else { return };
        let branches = match result {
            Ok(b) => b,
            Err(e) => return self.git_failed("branches", &e),
        };
        let mut items = vec![PickerItem::new("+ Create new branch…", "from the current commit", "", 0)];
        for (k, b) in branches.iter().enumerate() {
            let detail = match (&b.upstream, b.current) {
                (_, true) => format!("current · {}", b.commit),
                (Some(u), _) => format!("{} · tracks {u}", b.commit),
                _ => b.commit.clone(),
            };
            let hint = if b.remote { "remote" } else { "" };
            items.push(PickerItem::new(b.name.clone(), detail, hint, k + 1));
        }
        let name = self.git.repos[repo].name.clone();
        self.git.branch_pick = Some((repo, branches));
        self.open_overlay(Overlay::Picker(Picker::new(
            format!("Switch Branch in {name}"),
            items,
            PickerAction::GitBranch,
        )));
    }

    pub(crate) fn git_branch_pick(&mut self, data: usize) {
        let Some((repo, branches)) = self.git.branch_pick.take() else { return };
        if repo >= self.git.repos.len() {
            return;
        }
        if data == 0 {
            self.git_create_branch_prompt(repo);
            return;
        }
        let Some(b) = branches.get(data - 1).cloned() else { return };
        if b.current {
            return;
        }
        self.git_op(repo, GitOp::Switch(b));
    }

    fn git_create_branch_prompt(&mut self, repo: usize) {
        let toplevel = self.git.repos[repo].info.toplevel.clone();
        let name = self.git.repos[repo].name.clone();
        self.prompt(
            &format!("Create Branch in {name}"),
            "New branch name (created from the current commit and checked out)",
            String::new(),
            PromptAction::GitCreateBranch { toplevel },
            false,
        );
    }

    pub(crate) fn git_create_branch_submit(&mut self, toplevel: PathBuf, name: &str) -> Result<(), String> {
        let name = name.trim();
        if name.is_empty() || name.contains(char::is_whitespace) {
            return Err("Branch names can't be empty or contain spaces".into());
        }
        let Some(repo) = self.git.repos.iter().position(|r| r.info.toplevel == toplevel) else {
            return Err("The repository is no longer in the workspace".into());
        };
        self.git_op(repo, GitOp::CreateBranch(name.to_string()));
        Ok(())
    }

    // ------------------------------------------------------------------
    // Results

    pub(crate) fn on_git_op(&mut self, toplevel: &Path, op: GitOp, result: Result<String, GitError>) {
        let repo = self.git.repos.iter().position(|r| r.info.toplevel == toplevel);
        match &result {
            Ok(out) => {
                let name = repo.map(|i| self.git.repos[i].name.clone()).unwrap_or_default();
                let mut msg = format!("{name}: {}", op.describe());
                if matches!(op, GitOp::Commit { .. }) && !out.is_empty() {
                    msg = format!("{name}: {out}");
                }
                self.notify(Severity::Info, msg);
            }
            Err(e) => self.git_failed(&op.describe().to_lowercase(), e),
        }
        if let Some(i) = repo {
            self.git.repos[i].generation += 1; // index/HEAD changed: bases are stale
            self.git_schedule(i, Duration::ZERO);
        }
    }

    fn git_failed(&mut self, what: &str, e: &GitError) {
        let first = e.to_string().lines().find(|l| !l.trim().is_empty()).unwrap_or("").to_string();
        self.notify_sticky(Severity::Error, format!("Git {what} failed: {first} (Git: Show Output has the details)"));
    }

    /// Git: Show Output — every git command YACE ran, newest last.
    pub(crate) fn git_show_output(&mut self) {
        let mut text = String::from("# git commands run by YACE (read-only ones use --no-optional-locks)\n");
        for e in self.git.git.log() {
            text.push_str(&e.line());
            text.push('\n');
            if let Some(err) = &e.error {
                for l in err.lines() {
                    text.push_str("    ");
                    text.push_str(l);
                    text.push('\n');
                }
            }
        }
        self.open_virtual_doc("Git Output", &text, None);
    }

    /// A read-only generated document; one with the same title is reused.
    pub(crate) fn open_virtual_doc(&mut self, title: &str, text: &str, file: Option<PathBuf>) {
        let existing = self
            .docs
            .ids()
            .into_iter()
            .find(|d| self.docs.get(*d).and_then(|e| e.virtual_doc.as_ref()).is_some_and(|v| v.title == title));
        if let Some(d) = existing {
            self.close_doc_views(d);
        }
        self.new_untitled_with(text);
        if let Some((_, _, d)) = self.active()
            && let Some(e) = self.docs.get_mut(d)
        {
            e.virtual_doc = Some(crate::editor::VirtualDoc { title: title.to_string(), file });
        }
        self.focus_editor();
    }
}
