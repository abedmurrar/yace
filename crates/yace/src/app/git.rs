//! Git in the app (spec §15): detection per root, independently scheduled
//! status per repository, Explorer decorations, gutter hunks, refresh
//! triggers. Inspection is automatic and read-only; operations live in
//! `app/git_ops.rs` and run only from commands.

use std::{
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant},
};

use yace_git::{Kind, Status};
use yace_workspace::RootId;

use super::{App, Effect, Severity};
use crate::{
    editor::DocId,
    git::{DocGit, GitJob, GitResult, RepoState},
};

/// Delay after a trigger before a status refresh (coalesces bursts).
pub const STATUS_DEBOUNCE: Duration = Duration::from_millis(300);
/// Delay after typing before the gutter diff is recomputed.
pub const DIFF_DEBOUNCE: Duration = Duration::from_millis(150);
/// Periodic refresh while the Source Control view is visible.
pub const POLL: Duration = Duration::from_secs(10);

/// `a/b/c` from a path relative to a root.
fn slash(p: &Path) -> String {
    p.components().map(|c| c.as_os_str().to_string_lossy()).collect::<Vec<_>>().join("/")
}

impl App {
    fn git_on(&self) -> bool {
        self.settings.git.enabled && !self.git.missing
    }

    pub(crate) fn git_job(&mut self, job: GitJob) {
        tracing::debug!(?job, "git job");
        self.effects.push(Effect::Git(self.git.git.clone(), job));
    }

    /// The workspace's roots changed (or a workspace opened): find their
    /// repositories again.
    pub(crate) fn git_workspace_changed(&mut self) {
        self.git.git = yace_git::Git::new(self.settings.git.path.clone());
        self.git.epoch += 1;
        self.git.detected = false;
        if !self.git_on() {
            self.git.repos.clear();
            self.git.root_repo.clear();
            self.git.docs.clear();
            return;
        }
        let roots: Vec<(RootId, PathBuf)> =
            self.workspace.roots().iter().filter(|r| r.is_available()).map(|r| (r.id, r.path.clone())).collect();
        if roots.is_empty() {
            self.git.repos.clear();
            self.git.root_repo.clear();
            self.git.docs.clear();
            self.git.detected = true;
            return;
        }
        self.git.detecting = true;
        self.git_job(GitJob::Detect { roots });
    }

    pub fn on_git(&mut self, r: GitResult) {
        tracing::debug!(result = ?r, "git result");
        match r {
            GitResult::Detected(res) => self.on_git_detected(res),
            GitResult::Status { toplevel, generation, result } => self.on_git_status(&toplevel, generation, result),
            GitResult::Base { doc, generation, text } => {
                if let Some(d) = self.git.docs.get_mut(&doc)
                    && d.base_gen == generation
                {
                    d.base_pending = false;
                    d.base_loaded = true;
                    d.base = text;
                    if d.base.is_none() {
                        d.hunks.clear();
                    }
                    d.hunks_version = u64::MAX; // recompute
                    d.diff_due = Some(Instant::now());
                }
            }
            GitResult::Hunks { doc, version, hunks } => {
                if let Some(d) = self.git.docs.get_mut(&doc) {
                    d.diff_pending = false;
                    d.hunks = hunks;
                    d.hunks_version = version;
                }
            }
            GitResult::DiffText { title, file, result } => self.on_git_diff_text(title, file, result),
            GitResult::Branches { toplevel, result } => self.on_git_branches(&toplevel, result),
            GitResult::Op { toplevel, op, result } => self.on_git_op(&toplevel, op, result),
        }
        self.needs_render = true;
    }

    fn on_git_detected(&mut self, res: Result<Vec<(RootId, Option<yace_git::RepoInfo>)>, yace_git::GitError>) {
        self.git.detecting = false;
        let found = match res {
            Ok(f) => f,
            Err(yace_git::GitError::NotFound(p)) => {
                self.git.missing = true;
                self.notify_sticky(
                    Severity::Info,
                    format!("Git features are off: the git program was not found ({p}). Install git or set git.path."),
                );
                return;
            }
            Err(e) => {
                self.notify(Severity::Warning, format!("Git: {e}"));
                return;
            }
        };
        let old = std::mem::take(&mut self.git.repos);
        self.git.root_repo.clear();
        for (root, info) in found {
            let Some(info) = info else { continue };
            let root_path = self.workspace.root(root).map(|r| r.canonical.clone().unwrap_or_else(|| r.path.clone()));
            let prefix = root_path
                .and_then(|p| p.strip_prefix(&info.toplevel).ok().map(slash))
                .map(|p| if p.is_empty() { p } else { format!("{p}/") })
                .unwrap_or_default();
            let idx = match self.git.repos.iter().position(|r| r.info.toplevel == info.toplevel) {
                Some(i) => i,
                None => {
                    // Keep what we knew about a repository already shown.
                    let prev = old.iter().find(|r| r.info.toplevel == info.toplevel);
                    let name = info.toplevel.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                    self.git.repos.push(RepoState {
                        name,
                        roots: Vec::new(),
                        status: prev.and_then(|p| p.status.clone()),
                        error: None,
                        in_flight: false,
                        again: false,
                        due: Some(Instant::now()),
                        generation: prev.map(|p| p.generation).unwrap_or(0),
                        decor: prev.map(|p| p.decor.clone()).unwrap_or_default(),
                        info,
                    });
                    self.git.repos.len() - 1
                }
            };
            self.git.repos[idx].roots.push((root, prefix));
            self.git.root_repo.insert(root, idx);
        }
        // Prefer the root's own name for a repository with one root.
        for r in &mut self.git.repos {
            if let [(root, _)] = r.roots.as_slice()
                && let Some(w) = self.workspace.root(*root)
            {
                r.name = w.display_name();
            }
        }
        self.git.docs.clear();
        self.git.detected = true;
        self.sync_watches();
    }

    fn repo_by_toplevel(&self, toplevel: &Path) -> Option<usize> {
        self.git.repos.iter().position(|r| r.info.toplevel == toplevel)
    }

    fn on_git_status(
        &mut self,
        toplevel: &Path,
        _generation: u64,
        result: Result<(Status, std::collections::HashMap<String, Kind>), yace_git::GitError>,
    ) {
        let Some(i) = self.repo_by_toplevel(toplevel) else { return };
        let r = &mut self.git.repos[i];
        r.in_flight = false;
        if r.again {
            r.again = false;
            r.due = Some(Instant::now() + STATUS_DEBOUNCE);
        }
        match result {
            Ok((st, decor)) => {
                let changed = r.status.as_ref() != Some(&st);
                // The first status after detection doesn't invalidate bases.
                let head_moved = r.status.as_ref().is_some_and(|s| (&s.branch, &s.head) != (&st.branch, &st.head));
                r.status = Some(st);
                r.error = None;
                if changed {
                    r.decor = decor;
                    if head_moved {
                        r.generation += 1; // bases may differ now
                    }
                    let (branch, roots) = (
                        r.status.as_ref().and_then(|s| s.branch.clone()),
                        r.roots.iter().map(|x| x.0.0).collect::<Vec<_>>(),
                    );
                    self.plugin_event(
                        "gitStatusChanged",
                        serde_json::json!({ "repo": toplevel.display().to_string(), "roots": roots, "branch": branch }),
                    );
                }
            }
            Err(e) => r.error = Some(e.to_string()),
        }
    }

    /// Ask for a status refresh of repository `i` after `delay`.
    pub(crate) fn git_schedule(&mut self, i: usize, delay: Duration) {
        let Some(r) = self.git.repos.get_mut(i) else { return };
        if r.in_flight {
            r.again = true;
            return;
        }
        let at = Instant::now() + delay;
        r.due = Some(r.due.map_or(at, |d| d.min(at)));
    }

    pub(crate) fn git_schedule_all(&mut self, delay: Duration) {
        for i in 0..self.git.repos.len() {
            self.git_schedule(i, delay);
        }
    }

    /// The repository a path belongs to and its repository-relative path,
    /// computed from the workspace root (no filesystem calls).
    pub fn git_repo_of(&self, path: &Path) -> Option<(usize, String)> {
        let root = self.workspace.root_for_path(path)?;
        let i = *self.git.root_repo.get(&root.id)?;
        let prefix = &self.git.repos[i].roots.iter().find(|(r, _)| *r == root.id)?.1;
        let rel = path
            .strip_prefix(&root.path)
            .or_else(|_| root.canonical.as_deref().map_or(Err(()), |c| path.strip_prefix(c).map_err(|_| ())))
            .ok()?;
        Some((i, format!("{prefix}{}", slash(rel))))
    }

    /// Explorer state of a path (files, folders with changes, ignored).
    pub fn git_decoration(&self, path: &Path) -> Option<Kind> {
        if !self.settings.git.decorations_enabled || self.git.repos.is_empty() {
            return None;
        }
        let (i, rel) = self.git_repo_of(path)?;
        let decor = &self.git.repos[i].decor;
        if let Some(k) = decor.get(&rel) {
            return Some(*k);
        }
        // Inside an ignored folder?
        let mut p = rel.as_str();
        while let Some(at) = p.rfind('/') {
            p = &p[..at];
            if decor.get(p) == Some(&Kind::Ignored) {
                return Some(Kind::Ignored);
            }
        }
        None
    }

    /// The repository of the active file, else of the selected Explorer
    /// row's root, else the first one.
    pub fn git_active_repo(&self) -> Option<usize> {
        if self.git.repos.is_empty() {
            return None;
        }
        let from_doc = self
            .active()
            .and_then(|(_, _, d)| self.docs.doc(d).and_then(|x| x.path().map(Path::to_path_buf)))
            .and_then(|p| self.git_repo_of(&p).map(|x| x.0));
        let from_explorer =
            || self.explorer.rows.get(self.explorer.selected).and_then(|r| self.git.root_repo.get(&r.root).copied());
        from_doc.or_else(from_explorer).or(Some(0))
    }

    /// Status bar text for the active repository.
    pub fn git_status_label(&self) -> Option<String> {
        if !self.git_on() {
            return None;
        }
        let r = &self.git.repos[self.git_active_repo()?];
        if r.error.is_some() && r.status.is_none() {
            return Some("git ⚠".into());
        }
        let st = r.status.as_ref()?;
        let mut s = format!("{} {}", self.glyphs_branch(), st.label());
        if st.conflicts() > 0 {
            s.push_str(&format!(" {} {}", self.glyphs.warning, st.conflicts()));
        }
        Some(s)
    }

    fn glyphs_branch(&self) -> &'static str {
        if self.glyphs.vbar == "|" { "git:" } else { "\u{2387}" }
    }

    /// When git work is next due (status debounces, gutter diffs, the
    /// Source Control view's periodic refresh), for the event loop.
    pub(crate) fn git_deadline(&self) -> Option<Instant> {
        if !self.git_on() || !self.git.detected {
            return None;
        }
        let repos = self.git.repos.iter().filter(|r| !r.in_flight).filter_map(|r| r.due);
        let docs = self.git.docs.values().filter(|d| !d.diff_pending).filter_map(|d| d.diff_due);
        let poll =
            (self.sidebar_visible && self.sidebar_view == super::SidebarView::Git && self.settings.git.auto_refresh)
                .then(|| self.git.last_poll.map_or_else(Instant::now, |t| t + POLL));
        repos.chain(docs).chain(poll).min()
    }

    /// Start whatever git work is due (end of every event and on ticks).
    pub(crate) fn git_pump(&mut self, now: Instant) {
        if !self.git_on() || !self.git.detected {
            return;
        }
        // Periodic refresh while the Source Control view is shown.
        if self.sidebar_visible
            && self.sidebar_view == super::SidebarView::Git
            && self.settings.git.auto_refresh
            && self.git.last_poll.is_none_or(|t| now.duration_since(t) >= POLL)
        {
            self.git.last_poll = Some(now);
            self.git_schedule_all(Duration::ZERO);
        }
        let mut jobs = Vec::new();
        for r in &mut self.git.repos {
            if !r.in_flight && r.due.is_some_and(|d| d <= now) {
                r.due = None;
                r.in_flight = true;
                jobs.push(GitJob::Status { generation: r.generation, toplevel: r.info.toplevel.clone() });
            }
        }
        for j in jobs {
            self.git_job(j);
        }
        self.git_track_docs(now);
    }

    /// Bases and gutter diffs of open documents in repositories.
    fn git_track_docs(&mut self, now: Instant) {
        if self.settings.git.diff_decorations == yace_config::DiffDecorations::None {
            return;
        }
        let ids = self.docs.ids();
        self.git.docs.retain(|d, _| ids.contains(d));
        let mut jobs = Vec::new();
        for d in ids {
            if self.docs.is_loading(d) {
                continue;
            }
            let Some(doc) = self.docs.doc(d) else { continue };
            let Some(path) = doc.path().map(Path::to_path_buf) else { continue };
            let version = doc.version();
            let Some((repo, rel)) = self.git_repo_of(&path) else { continue };
            let generation = self.git.repos[repo].generation;
            let toplevel = self.git.repos[repo].info.toplevel.clone();
            let fresh = || DocGit { repo, rel: rel.clone(), hunks_version: u64::MAX, ..Default::default() };
            let entry = self.git.docs.entry(d).or_insert_with(fresh);
            if entry.rel != rel || entry.repo != repo {
                *entry = fresh();
            }
            if !entry.base_pending && (!entry.base_loaded || entry.base_gen < generation) {
                entry.base_pending = true;
                entry.base_gen = generation;
                jobs.push(GitJob::Base { doc: d, rel: rel.clone(), toplevel, generation });
                continue;
            }
            let Some(base) = entry.base.clone() else { continue };
            if entry.hunks_version == version || entry.diff_pending {
                continue;
            }
            let due = *entry.diff_due.get_or_insert(now + DIFF_DEBOUNCE);
            if due <= now {
                entry.diff_due = None;
                entry.diff_pending = true;
                let text = doc.rope().clone();
                jobs.push(GitJob::Diff { doc: d, version, base, text });
            }
        }
        for j in jobs {
            self.git_job(j);
        }
    }

    /// Gutter hunks of a document (for drawing), if current enough.
    pub fn git_hunks(&self, d: DocId) -> &[yace_git::Hunk] {
        if self.settings.git.diff_decorations == yace_config::DiffDecorations::None || !self.git_on() {
            return &[];
        }
        self.git.docs.get(&d).map(|g| g.hunks.as_slice()).unwrap_or(&[])
    }

    /// Filesystem events: repositories whose files or `.git` changed are
    /// refreshed; a changed index or HEAD makes document bases stale.
    pub(crate) fn git_paths_changed(&mut self, paths: &[PathBuf]) {
        if !self.git_on() || !self.settings.git.auto_refresh || self.git.repos.is_empty() {
            return;
        }
        let mut touched = Vec::new();
        for p in paths {
            for (i, r) in self.git.repos.iter().enumerate() {
                if p.starts_with(&r.info.git_dir) {
                    let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
                    // Lock files and temporary objects come and go during
                    // our own reads; only real state changes matter.
                    if name.ends_with(".lock") {
                        continue;
                    }
                    touched.push((i, matches!(name, "index" | "HEAD" | "ORIG_HEAD" | "MERGE_HEAD" | "packed-refs")));
                } else if p.starts_with(&r.info.toplevel) {
                    touched.push((i, false));
                }
            }
        }
        for (i, base_stale) in touched {
            if base_stale {
                self.git.repos[i].generation += 1;
            }
            self.git_schedule(i, STATUS_DEBOUNCE);
        }
    }

    /// The `.git` folders to watch (non-recursively).
    pub(crate) fn git_watch_dirs(&self) -> Vec<PathBuf> {
        if !self.git_on() || !self.settings.git.auto_refresh {
            return Vec::new();
        }
        self.git.repos.iter().map(|r| r.info.git_dir.clone()).filter(|d| d.is_dir()).collect()
    }

    /// Git: Refresh.
    pub(crate) fn git_refresh(&mut self) {
        if self.git.missing {
            self.git.missing = false;
            self.git_workspace_changed();
            return;
        }
        for r in &mut self.git.repos {
            r.generation += 1;
        }
        self.git_schedule_all(Duration::ZERO);
    }

    /// Go to Next/Previous Change (gutter hunks), wrapping around.
    pub(crate) fn git_goto_change(&mut self, forward: bool) {
        let Some((_, v, d)) = self.active() else { return };
        let hunks = self.git_hunks(d).to_vec();
        if hunks.is_empty() {
            self.notify(Severity::Info, "No changes in this file");
            return;
        }
        let head = self.layout.active_group().views[v].sel.primary().head;
        let line = self.docs.doc(d).map(|x| x.rope().char_to_line(head)).unwrap_or(0) as u32;
        let starts: Vec<u32> = hunks.iter().map(|h| h.lines.start).collect();
        let target = if forward {
            starts.iter().copied().find(|s| *s > line).unwrap_or(starts[0])
        } else {
            starts.iter().rev().copied().find(|s| *s < line).unwrap_or(*starts.last().unwrap_or(&0))
        };
        let max_line = self.docs.doc(d).map(|x| x.rope().len_lines().saturating_sub(1)).unwrap_or(0);
        self.set_cursor_line_col((target as usize).min(max_line), 0);
        self.center_active_cursor();
    }

    /// Git: Show Change Under Cursor — the base's lines of the hunk.
    pub(crate) fn git_show_change(&mut self) {
        let Some((_, v, d)) = self.active() else { return };
        let head = self.layout.active_group().views[v].sel.primary().head;
        let line = self.docs.doc(d).map(|x| x.rope().char_to_line(head)).unwrap_or(0) as u32;
        let Some(g) = self.git.docs.get(&d) else {
            self.notify(Severity::Info, "This file is not in a Git repository");
            return;
        };
        let hunk = g.hunks.iter().find(|h| h.lines.contains(&line) || (h.lines.is_empty() && h.lines.start == line));
        let (Some(h), Some(base)) = (hunk, g.base.as_ref()) else {
            self.notify(Severity::Info, "No change at the cursor");
            return;
        };
        let old = yace_git::diff::old_text(base, h);
        let mut lines: Vec<String> = if old.is_empty() {
            vec!["(added lines; nothing was here before)".into()]
        } else {
            old.lines().map(|l| format!("- {l}")).collect()
        };
        lines.truncate(200);
        self.open_overlay(crate::overlay::Overlay::Info {
            title: format!("Before (index) — lines {}", h.old.start + 1),
            lines,
        });
    }

    /// Repository status, for the Source Control view and tests.
    pub fn git_status_of(&self, i: usize) -> Option<&Status> {
        self.git.repos.get(i).and_then(|r| r.status.as_ref())
    }

    /// Bases kept per document (for tests).
    pub fn git_base(&self, d: DocId) -> Option<Arc<str>> {
        self.git.docs.get(&d).and_then(|g| g.base.clone())
    }
}
