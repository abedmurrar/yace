//! Workspace search view in the sidebar (spec §4.6, §14): query, replace,
//! include/exclude, streamed results grouped by root → file → line, and
//! preview-before-apply replacement that never silently overwrites open
//! buffers (§14.3).

use std::{
    collections::{BTreeSet, HashMap, HashSet},
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant},
};

use yace_buffer::{Change, ChangeSet};
use yace_keymap::{KeyCode, KeyPress, Mods};
use yace_search::{
    ContentQuery, FileMatches, FindOptions, LineMatch, RootKey, SearchRoot, SearchSummary, WalkOptions,
    content::{apply_replacements, plan_replacements, replace_in_file},
    find,
    regex::Regex,
};
use yace_workspace::{RootId, RootStatus};

use super::{App, DocId, Effect, Focus, Severity, SidebarView};
use crate::{
    actions::ConfirmAction,
    overlay::{Confirm, LineInput, Overlay},
    workers::SearchJob,
};

const DEBOUNCE: Duration = Duration::from_millis(150);
/// Delay before re-searching files that changed while results are shown.
const UPDATE_DELAY: Duration = Duration::from_millis(300);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SearchField {
    #[default]
    Query,
    Replace,
    Include,
    Exclude,
    Results,
}

#[derive(Debug, Clone)]
pub struct FileResult {
    pub root: RootKey,
    pub rel: String,
    pub path: PathBuf,
    pub hash: u64,
    pub from_buffer: bool,
    pub matches: Vec<LineMatch>,
    pub expanded: bool,
    /// Line matches excluded from replacement (indices into `matches`).
    pub excluded: HashSet<usize>,
    pub file_excluded: bool,
}

impl FileResult {
    fn new(f: FileMatches) -> Self {
        Self {
            root: f.root,
            rel: f.rel,
            path: f.path,
            hash: f.hash,
            from_buffer: f.from_buffer,
            matches: f.matches,
            expanded: true,
            excluded: HashSet::new(),
            file_excluded: false,
        }
    }
}

/// A visible row of the results tree.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Row {
    Root(RootKey),
    File(usize),
    Match(usize, usize),
}

#[derive(Debug, Default)]
pub struct SearchState {
    pub query: LineInput,
    pub replace: LineInput,
    pub include: LineInput,
    pub exclude: LineInput,
    pub show_replace: bool,
    pub show_details: bool,
    pub field: SearchField,
    pub opts: FindOptions,
    pub seq: u64,
    pub running: bool,
    pub debounce: Option<Instant>,
    pub files: Vec<FileResult>,
    pub summary: Option<SearchSummary>,
    pub error: Option<String>,
    pub selected: usize,
    pub scroll: usize,
    /// Root order for sorting results.
    pub root_order: Vec<RootKey>,
    /// The regex of the search that produced `files` (for replace
    /// previews); `None` for an invalid pattern.
    pub regex: Option<Regex>,
    /// The query that produced `files` (live updates re-run it on
    /// changed files).
    pub last_query: Option<ContentQuery>,
    /// What the last search depended on, and whether it finished without
    /// hitting a limit: a longer literal query can then search only the
    /// files that matched before.
    last_key: Option<(SearchKey, bool)>,
    /// Files changed since the results were produced, re-searched at
    /// `update_due`.
    pub pending_paths: BTreeSet<PathBuf>,
    pub update_due: Option<Instant>,
    /// Files of the last finished search (the candidates when narrowing).
    last_files: Vec<PathBuf>,
    /// Version, path and dirty flag of each open file, to notice edits.
    seen_docs: HashMap<DocId, (u64, PathBuf, bool)>,
}

/// Everything besides the pattern that a search's results depend on.
#[derive(Debug, Clone, PartialEq)]
struct SearchKey {
    pattern: String,
    opts: FindOptions,
    include: Vec<String>,
    exclude: Vec<String>,
    walk_exclude: Vec<String>,
    use_ignore_files: bool,
    max_results: usize,
    roots: Vec<(RootKey, PathBuf)>,
}

impl SearchKey {
    /// True when every match of `self` is also a match of `prev`, so only
    /// files that matched `prev` can match `self`.
    fn narrows(&self, prev: &SearchKey) -> bool {
        let same = SearchKey { pattern: prev.pattern.clone(), ..self.clone() };
        if same != *prev || self.opts.regex || self.opts.whole_word || prev.pattern.is_empty() {
            return false;
        }
        if self.opts.case_sensitive {
            self.pattern.contains(&prev.pattern)
        } else {
            self.pattern.to_lowercase().contains(&prev.pattern.to_lowercase())
        }
    }
}

impl SearchState {
    pub fn input_focused(&self) -> bool {
        self.field != SearchField::Results
    }

    /// Replace previews are shown while the replace field is visible, as
    /// in VS Code (an empty replacement previews deleting the matches).
    pub fn replace_preview_active(&self) -> bool {
        self.show_replace && self.regex.is_some()
    }

    /// `line_text` with each match's replacement, as
    /// `(char_from, char_to, replacement)` within the line, using the same
    /// function Replace All uses.
    pub fn line_replacements(&self, line_text: &str) -> Vec<(usize, usize, String)> {
        match &self.regex {
            Some(re) => find::replacements(line_text, re, &self.replace.text, self.opts.regex),
            None => vec![],
        }
    }

    pub fn rows(&self) -> Vec<Row> {
        let mut out = Vec::new();
        let mut last_root = None;
        for (fi, f) in self.files.iter().enumerate() {
            if last_root != Some(f.root) {
                out.push(Row::Root(f.root));
                last_root = Some(f.root);
            }
            out.push(Row::File(fi));
            if f.expanded {
                out.extend((0..f.matches.len()).map(|mi| Row::Match(fi, mi)));
            }
        }
        out
    }

    pub fn total_matches(&self) -> usize {
        self.files.iter().map(|f| f.matches.iter().map(|m| m.ranges.len().max(1)).sum::<usize>()).sum()
    }

    /// Matches / files that a Replace All would touch.
    pub fn replace_counts(&self) -> (usize, usize) {
        let mut m = 0;
        let mut files = 0;
        for f in self.files.iter().filter(|f| !f.file_excluded) {
            let n: usize = f
                .matches
                .iter()
                .enumerate()
                .filter(|(i, _)| !f.excluded.contains(i))
                .map(|(_, lm)| lm.ranges.len().max(1))
                .sum();
            if n > 0 {
                m += n;
                files += 1;
            }
        }
        (m, files)
    }

    fn field_mut(&mut self) -> Option<&mut LineInput> {
        match self.field {
            SearchField::Query => Some(&mut self.query),
            SearchField::Replace => Some(&mut self.replace),
            SearchField::Include => Some(&mut self.include),
            SearchField::Exclude => Some(&mut self.exclude),
            SearchField::Results => None,
        }
    }

    /// The input fields currently shown, in order.
    pub fn fields(&self) -> Vec<SearchField> {
        let mut v = vec![SearchField::Query];
        if self.show_replace {
            v.push(SearchField::Replace);
        }
        if self.show_details {
            v.push(SearchField::Include);
            v.push(SearchField::Exclude);
        }
        v.push(SearchField::Results);
        v
    }
}

/// `(line, char_col)` of every match the user excluded in `f`.
fn skip_set(f: &FileResult) -> HashSet<(usize, usize)> {
    let mut skip = HashSet::new();
    for mi in &f.excluded {
        let lm = &f.matches[*mi];
        skip.extend(lm.ranges.iter().map(|r| (lm.line, r.0)));
    }
    skip
}

fn split_list(s: &str) -> Vec<String> {
    s.split(',').map(|p| p.trim().to_string()).filter(|p| !p.is_empty()).collect()
}

impl App {
    /// `Ctrl+Shift+F` (and `Ctrl+Shift+H` with replace).
    pub(crate) fn search_open(&mut self, replace: bool) {
        // Seed from a single-line editor selection.
        if self.focus == Focus::Editor
            && let Some((g, v, d)) = self.active()
            && let Some(r) = self.layout.group(g).map(|gr| gr.views[v].sel.primary())
            && !r.is_empty()
            && let Some(doc) = self.docs.doc(d)
        {
            let t: String = doc.rope().slice(r.from()..r.to()).into();
            if !t.contains('\n') {
                self.search.query = LineInput::new(t);
                self.schedule_search();
            }
        }
        self.search.query.all_selected = true;
        if replace {
            self.search.show_replace = true;
        }
        self.search.field = SearchField::Query;
        self.sidebar_view = SidebarView::Search;
        self.sidebar_visible = true;
        self.focus_changed(Focus::Explorer);
    }

    /// Explorer: "Find in Folder…".
    pub(crate) fn search_in_folder(&mut self) {
        let Some((root, dir)) = self.explorer.target_dir() else { return };
        let Some(r) = self.workspace.root(root) else { return };
        let rel = dir.strip_prefix(&r.path).map(|p| p.to_string_lossy().into_owned()).unwrap_or_default();
        let scope =
            if rel.is_empty() { format!("./{}", r.display_name()) } else { format!("./{}/{rel}", r.display_name()) };
        self.search.include = LineInput::new(scope);
        self.search.show_details = true;
        self.search_open(false);
        self.schedule_search();
    }

    pub(crate) fn schedule_search(&mut self) {
        self.search.debounce = Some(Instant::now() + DEBOUNCE);
    }

    pub(crate) fn search_roots(&self) -> Vec<SearchRoot> {
        self.workspace
            .roots()
            .iter()
            .filter(|r| r.status == RootStatus::Available)
            .map(|r| SearchRoot { key: r.id.0, name: r.display_name(), path: r.path.clone() })
            .collect()
    }

    /// Start (or restart) the search for the current inputs.
    pub(crate) fn run_search(&mut self) {
        self.search.seq += 1;
        self.search.files.clear();
        self.search.summary = None;
        self.search.error = None;
        self.search.selected = 0;
        self.search.scroll = 0;
        let pattern = self.search.query.text.clone();
        self.search.regex = if pattern.is_empty() { None } else { find::build_regex(&pattern, self.search.opts).ok() };
        if pattern.is_empty() {
            self.search.running = false;
            self.search.last_query = None;
            self.search.last_key = None;
            self.search.pending_paths.clear();
            self.search.update_due = None;
            self.emit(Effect::SearchCancel);
            return;
        }
        let s = &self.settings;
        let mut exclude = s.files.exclude.clone();
        exclude.extend(s.search.exclude.iter().cloned());
        let query = ContentQuery {
            pattern,
            opts: self.search.opts,
            include: split_list(&self.search.include.text),
            exclude: split_list(&self.search.exclude.text),
            walk: WalkOptions {
                use_ignore_files: s.search.use_ignore_files,
                include_hidden: true,
                exclude,
                max_files: usize::MAX,
                report_dirs: false,
            },
            max_results: s.search.max_results.max(1),
            max_per_file: 1_000,
        };
        // Dirty open documents are searched as the user sees them.
        let overlays = self.dirty_overlays(None);
        self.search.root_order = self.workspace.roots().iter().map(|r| r.id.0).collect();
        self.search.running = true;
        let roots = self.search_roots();
        let key = SearchKey {
            pattern: query.pattern.clone(),
            opts: query.opts,
            include: query.include.clone(),
            exclude: query.exclude.clone(),
            walk_exclude: query.walk.exclude.clone(),
            use_ignore_files: query.walk.use_ignore_files,
            max_results: query.max_results,
            roots: roots.iter().map(|r| (r.key, r.path.clone())).collect(),
        };
        // A longer literal query only needs the files that matched the
        // shorter one, provided those results were complete and kept up to
        // date by live updates.
        let only = match &self.search.last_key {
            Some((prev, true)) if key.narrows(prev) && self.all_roots_live() => {
                let mut paths: BTreeSet<PathBuf> = std::mem::take(&mut self.search.pending_paths);
                paths.extend(self.search.last_files.drain(..));
                Some(paths.into_iter().collect())
            }
            _ => None,
        };
        self.search.last_files.clear();
        self.search.pending_paths.clear();
        self.search.update_due = None;
        self.search.last_key = Some((key, false));
        self.search.last_query = Some(query.clone());
        let job = SearchJob { seq: self.search.seq, roots, query, overlays, only, update: false };
        self.emit(Effect::Search(Box::new(job)));
    }

    /// Text of dirty open documents (all, or just those in `paths`).
    fn dirty_overlays(&self, paths: Option<&BTreeSet<PathBuf>>) -> HashMap<PathBuf, Arc<str>> {
        let mut overlays = HashMap::new();
        for (_, e) in self.docs.iter() {
            if e.doc.is_dirty()
                && let Some(p) = e.doc.path()
                && paths.is_none_or(|set| set.contains(p))
            {
                overlays.insert(p.to_path_buf(), Arc::from(e.doc.text()));
            }
        }
        overlays
    }

    /// Queue files for a live update of the shown results.
    pub(crate) fn search_queue_paths(&mut self, paths: impl IntoIterator<Item = PathBuf>) {
        if self.search.last_query.is_none() {
            return;
        }
        let roots: Vec<PathBuf> = self.workspace.roots().iter().map(|r| r.path.clone()).collect();
        let before = self.search.pending_paths.len();
        self.search.pending_paths.extend(paths.into_iter().filter(|p| roots.iter().any(|r| p.starts_with(r))));
        if self.search.pending_paths.len() != before && self.search.update_due.is_none() {
            self.search.update_due = Some(Instant::now() + UPDATE_DELAY);
        }
    }

    /// Notice edits to open files (and dirty files being closed) while
    /// results are shown.
    pub(crate) fn search_track_edits(&mut self) {
        if self.search.last_query.is_none() {
            self.search.seen_docs.clear();
            return;
        }
        let mut seen = HashMap::with_capacity(self.search.seen_docs.len());
        let mut changed = Vec::new();
        for (id, e) in self.docs.iter() {
            let Some(p) = e.doc.path() else { continue };
            let now = (e.doc.version(), p.to_path_buf(), e.doc.is_dirty());
            if let Some(old) = self.search.seen_docs.get(&id)
                && (old.0 != now.0 || old.1 != now.1)
            {
                changed.push(now.1.clone());
            }
            seen.insert(id, now);
        }
        for (id, (_, p, dirty)) in &self.search.seen_docs {
            if *dirty && !seen.contains_key(id) {
                changed.push(p.clone());
            }
        }
        self.search.seen_docs = seen;
        if !changed.is_empty() {
            self.search_queue_paths(changed);
        }
    }

    /// Re-search the queued files with the query of the shown results.
    pub(crate) fn run_search_update(&mut self) {
        if self.search.running {
            // The running search may already have read them; update after.
            self.search.update_due = Some(Instant::now() + UPDATE_DELAY);
            return;
        }
        let Some(query) = self.search.last_query.clone() else { return };
        if self.search.pending_paths.is_empty() {
            return;
        }
        let paths = std::mem::take(&mut self.search.pending_paths);
        let overlays = self.dirty_overlays(Some(&paths));
        let job = SearchJob {
            seq: self.search.seq,
            roots: self.search_roots(),
            query,
            overlays,
            only: Some(paths.into_iter().collect()),
            update: true,
        };
        self.emit(Effect::SearchUpdate(Box::new(job)));
    }

    fn insert_result_sorted(&mut self, result: FileResult) {
        let order = |r: RootKey| self.search.root_order.iter().position(|x| *x == r).unwrap_or(usize::MAX);
        let key = (order(result.root), result.rel.as_str());
        let at = self.search.files.partition_point(|x| (order(x.root), x.rel.as_str()) < key);
        self.search.files.insert(at, result);
    }

    pub(crate) fn on_search_file_update(&mut self, seq: u64, path: PathBuf, file: Option<FileMatches>) {
        if seq != self.search.seq {
            return;
        }
        let pos = self.search.files.iter().position(|f| f.path == path);
        match (pos, file) {
            (Some(i), None) => {
                self.search.files.remove(i);
            }
            (Some(i), Some(f)) => {
                let old = &self.search.files[i];
                // Keep exclusions of matches that are still there.
                let gone: HashSet<(&str, usize)> = old
                    .excluded
                    .iter()
                    .filter_map(|mi| old.matches.get(*mi))
                    .map(|m| (m.text.as_str(), m.ranges.first().map_or(0, |r| r.0)))
                    .collect();
                let excluded = f
                    .matches
                    .iter()
                    .enumerate()
                    .filter(|(_, m)| gone.contains(&(m.text.as_str(), m.ranges.first().map_or(0, |r| r.0))))
                    .map(|(i, _)| i)
                    .collect();
                let result = FileResult {
                    root: f.root,
                    rel: f.rel,
                    path: f.path,
                    hash: f.hash,
                    from_buffer: f.from_buffer,
                    matches: f.matches,
                    expanded: old.expanded,
                    excluded,
                    file_excluded: old.file_excluded,
                };
                self.search.files[i] = result;
            }
            (None, Some(f)) => self.insert_result_sorted(FileResult::new(f)),
            (None, None) => return,
        }
        let rows = self.search.rows().len();
        self.search.selected = self.search.selected.min(rows.saturating_sub(1));
        self.needs_render = true;
    }

    pub(crate) fn on_search_file(&mut self, seq: u64, f: FileMatches) {
        if seq != self.search.seq {
            return;
        }
        self.insert_result_sorted(FileResult::new(f));
    }

    pub(crate) fn on_search_done(&mut self, seq: u64, summary: SearchSummary, error: Option<String>) {
        if seq != self.search.seq {
            return;
        }
        self.search.running = false;
        let complete = !summary.truncated && !summary.cancelled && error.is_none();
        if let Some((_, done)) = &mut self.search.last_key {
            *done = complete;
        }
        self.search.last_files = self.search.files.iter().map(|f| f.path.clone()).collect();
        self.search.summary = Some(summary);
        if error.is_some() {
            self.search.last_query = None;
        }
        self.search.error = error;
        if !self.search.pending_paths.is_empty() {
            self.search.update_due = Some(Instant::now());
        }
    }

    /// One-line status for the view.
    pub fn search_status(&self) -> String {
        let s = &self.search;
        if let Some(e) = &s.error {
            return e.clone();
        }
        if s.query.text.is_empty() {
            return "Type to search across all workspace folders".into();
        }
        let m = s.total_matches();
        let files = s.files.len();
        let mut t =
            format!("{m} result{} in {files} file{}", if m == 1 { "" } else { "s" }, if files == 1 { "" } else { "s" });
        if s.running {
            t = format!("Searching… {t}");
        } else if m == 0 {
            t = "No results found".into();
        }
        if s.summary.is_some_and(|x| x.truncated) {
            t.push_str(" (limit reached)");
        }
        t
    }

    fn search_move(&mut self, d: isize) {
        let n = self.search.rows().len();
        if n == 0 {
            return;
        }
        self.search.selected = (self.search.selected as isize + d).clamp(0, n as isize - 1) as usize;
    }

    /// Open the selected match (`preview` keeps focus in the view).
    pub(crate) fn search_open_selected(&mut self, preview: bool) {
        let rows = self.search.rows();
        let Some(row) = rows.get(self.search.selected).copied() else { return };
        match row {
            Row::Root(_) => {}
            Row::File(fi) => {
                if let Some(f) = self.search.files.get_mut(fi) {
                    f.expanded = !f.expanded;
                }
            }
            Row::Match(fi, mi) if self.search.replace_preview_active() => {
                self.search_preview_replace(fi, mi);
                if preview {
                    self.focus = Focus::Explorer;
                }
            }
            Row::Match(fi, mi) => {
                let Some(f) = self.search.files.get(fi) else { return };
                let path = f.path.clone();
                let lm = &f.matches[mi];
                let (line, col, end) =
                    (lm.line, lm.ranges.first().map_or(0, |r| r.0), lm.ranges.first().map_or(0, |r| r.1));
                self.record_jump();
                self.open_file(&path, preview);
                self.select_line_range(line, col, end);
                if preview {
                    self.focus = Focus::Explorer;
                }
            }
        }
    }

    /// Open a read-only unified diff of the file before and after Replace
    /// All (excluded matches left alone), with the cursor on the match.
    pub(crate) fn search_preview_replace(&mut self, fi: usize, mi: usize) {
        let Some(f) = self.search.files.get(fi) else { return };
        let Some(re) = self.search.regex.clone() else { return };
        let (path, rel, line) = (f.path.clone(), f.rel.clone(), f.matches.get(mi).map_or(0, |m| m.line));
        let skip = skip_set(f);
        let text = match self.docs.find_by_path(&path).and_then(|d| self.docs.doc(d)) {
            Some(doc) => doc.text(),
            None => match std::fs::read_to_string(&path) {
                Ok(t) => t,
                Err(e) => {
                    self.notify(Severity::Error, format!("{rel}: {e}"));
                    return;
                }
            },
        };
        let reps = plan_replacements(&text, &re, &self.search.replace.text, self.search.opts.regex, &skip);
        let diff = yace_git::diff::unified(&text, &apply_replacements(&text, &reps), &rel);
        if diff.is_empty() {
            self.notify(Severity::Info, format!("Nothing to replace in {rel}"));
            return;
        }
        let name = rel.rsplit('/').next().unwrap_or(&rel).to_string();
        self.open_virtual_doc(&format!("{name} ↔ {name} (Replace Preview)"), &diff, Some(path));
        if let Some(row) = diff_row_for_new_line(&diff, line) {
            self.set_cursor_line_col(row, 0);
            self.center_active_cursor();
        }
    }

    fn select_line_range(&mut self, line: usize, col: usize, end: usize) {
        self.set_selection_with(|rope, _| {
            let line = line.min(rope.len_lines().saturating_sub(1));
            let ls = rope.line_to_char(line);
            let len = yace_buffer::text::line_without_ending(rope, line).len_chars();
            yace_buffer::Selection::single(yace_buffer::Range::new(ls + col.min(len), ls + end.min(len)))
        });
        self.center_active_cursor();
    }

    fn search_toggle_exclude(&mut self) {
        let rows = self.search.rows();
        match rows.get(self.search.selected).copied() {
            Some(Row::File(fi)) => {
                if let Some(f) = self.search.files.get_mut(fi) {
                    f.file_excluded = !f.file_excluded;
                }
            }
            Some(Row::Match(fi, mi)) => {
                if let Some(f) = self.search.files.get_mut(fi)
                    && !f.excluded.remove(&mi)
                {
                    f.excluded.insert(mi);
                }
            }
            _ => {}
        }
    }

    /// Ask before replacing across files.
    pub(crate) fn search_replace_all_confirm(&mut self) {
        if self.search.running {
            self.notify(Severity::Info, "Wait for the search to finish before replacing");
            return;
        }
        let (m, files) = self.search.replace_counts();
        if m == 0 {
            self.notify(Severity::Info, "Nothing to replace");
            return;
        }
        let with = &self.search.replace.text;
        self.open_overlay(Overlay::Confirm(Confirm {
            title: "Replace in files".into(),
            message: vec![
                format!(
                    "Replace {m} occurrence{} in {files} file{} with '{with}'?",
                    if m == 1 { "" } else { "s" },
                    if files == 1 { "" } else { "s" }
                ),
                "Open files are changed in their editors (unsaved, undoable); other files are written to disk.".into(),
            ],
            buttons: vec![("Replace".into(), ConfirmAction::ReplaceInFiles), ("Cancel".into(), ConfirmAction::Nothing)],
            selected: 0,
            cancel: ConfirmAction::Nothing,
        }));
    }

    pub(crate) fn search_replace_all(&mut self) {
        let re = match find::build_regex(&self.search.query.text, self.search.opts) {
            Ok(r) => r,
            Err(e) => {
                self.notify(Severity::Error, e);
                return;
            }
        };
        let replacement = self.search.replace.text.clone();
        let regex_mode = self.search.opts.regex;
        let mut replaced = 0;
        let mut files_done = 0;
        let mut problems = Vec::new();
        for f in self.search.files.clone().into_iter().filter(|f| !f.file_excluded) {
            let skip = skip_set(&f);
            if skip.len() >= f.matches.iter().map(|m| m.ranges.len()).sum::<usize>() && !f.matches.is_empty() {
                continue;
            }
            if let Some(doc) = self.docs.find_by_path(&f.path) {
                // Open document: undoable edit, left unsaved.
                let Some(d) = self.docs.doc(doc) else { continue };
                let text = d.text();
                let rope = d.rope().clone();
                let changes: Vec<Change> = plan_replacements(&text, &re, &replacement, regex_mode, &skip)
                    .into_iter()
                    .map(|(a, b, r)| Change::replace(a, b, r))
                    .collect();
                let n = changes.len();
                if n > 0 {
                    self.apply_doc_changes(doc, ChangeSet::new(changes, rope.len_chars()));
                    replaced += n;
                    files_done += 1;
                }
            } else {
                match replace_in_file(&f.path, &re, &replacement, regex_mode, f.hash, &skip) {
                    Ok(n) => {
                        if n > 0 {
                            replaced += n;
                            files_done += 1;
                        }
                    }
                    Err(e) => problems.push(format!("{}: {e}", f.rel)),
                }
            }
        }
        let mut msg = format!(
            "Replaced {replaced} occurrence{} in {files_done} file{}",
            if replaced == 1 { "" } else { "s" },
            if files_done == 1 { "" } else { "s" }
        );
        if !problems.is_empty() {
            msg.push_str(&format!("; skipped {}: {}", problems.len(), problems.join("; ")));
            self.notify(Severity::Warning, msg);
        } else {
            self.notify(Severity::Info, msg);
        }
        self.run_search();
    }

    pub(crate) fn search_clear(&mut self) {
        self.search.query = LineInput::default();
        self.search.replace = LineInput::default();
        self.run_search();
    }

    fn search_cycle_field(&mut self, forward: bool) {
        let fields = self.search.fields();
        let i = fields.iter().position(|f| *f == self.search.field).unwrap_or(0) as isize;
        let n = fields.len() as isize;
        self.search.field = fields[((i + if forward { 1 } else { -1 }).rem_euclid(n)) as usize];
    }

    /// Keys while the Search view has focus.
    pub(crate) fn search_input_key(&mut self, key: KeyPress, ch: Option<char>) {
        let alt = key.mods == Mods::ALT;
        let ctrl_alt = key.mods.contains(Mods::CTRL) && key.mods.contains(Mods::ALT);
        match key.code {
            KeyCode::Char('c') if alt => {
                self.search.opts.case_sensitive = !self.search.opts.case_sensitive;
                self.schedule_search();
                return;
            }
            KeyCode::Char('w') if alt => {
                self.search.opts.whole_word = !self.search.opts.whole_word;
                self.schedule_search();
                return;
            }
            KeyCode::Char('r') if alt => {
                self.search.opts.regex = !self.search.opts.regex;
                self.schedule_search();
                return;
            }
            KeyCode::Enter if ctrl_alt => {
                self.search_replace_all_confirm();
                return;
            }
            KeyCode::Tab => {
                self.search_cycle_field(!key.mods.contains(Mods::SHIFT));
                return;
            }
            KeyCode::Escape => {
                if self.search.field == SearchField::Results {
                    self.search.field = SearchField::Query;
                } else {
                    self.focus_editor();
                }
                return;
            }
            _ => {}
        }
        if self.search.field == SearchField::Results {
            match key.code {
                KeyCode::Up if self.search.selected == 0 => self.search.field = SearchField::Query,
                KeyCode::Up => self.search_move(-1),
                KeyCode::Down => self.search_move(1),
                KeyCode::PageUp => self.search_move(-10),
                KeyCode::PageDown => self.search_move(10),
                KeyCode::Home => self.search.selected = 0,
                KeyCode::End => self.search.selected = self.search.rows().len().saturating_sub(1),
                KeyCode::Enter => self.search_open_selected(false),
                KeyCode::Space => self.search_open_selected(true),
                KeyCode::Left | KeyCode::Right => {
                    let rows = self.search.rows();
                    match rows.get(self.search.selected).copied() {
                        Some(Row::File(fi)) => {
                            if let Some(f) = self.search.files.get_mut(fi) {
                                f.expanded = key.code == KeyCode::Right;
                            }
                        }
                        Some(Row::Match(fi, _)) if key.code == KeyCode::Left => {
                            if let Some(i) = rows.iter().position(|r| *r == Row::File(fi)) {
                                self.search.selected = i;
                            }
                        }
                        _ => {}
                    }
                }
                KeyCode::Delete => self.search_toggle_exclude(),
                _ => {}
            }
            return;
        }
        if matches!(key.code, KeyCode::Down) {
            self.search_cycle_field(true);
            return;
        }
        if matches!(key.code, KeyCode::Up) {
            if self.search.field != SearchField::Query {
                self.search_cycle_field(false);
            }
            return;
        }
        if key.code == KeyCode::Enter {
            self.debounce_now();
            if !self.search.files.is_empty() {
                self.search.field = SearchField::Results;
            }
            return;
        }
        let field = self.search.field;
        let Some(input) = self.search.field_mut() else { return };
        let before = input.text.clone();
        input.handle(key, ch);
        if input.text != before && field != SearchField::Replace {
            self.schedule_search();
        }
    }

    fn debounce_now(&mut self) {
        if self.search.debounce.take().is_some() || (self.search.files.is_empty() && !self.search.running) {
            self.run_search();
        }
    }

    pub(crate) fn search_paste(&mut self, text: &str) {
        let field = self.search.field;
        if let Some(input) = self.search.field_mut() {
            input.insert(text.lines().next().unwrap_or(""));
            if field != SearchField::Replace {
                self.schedule_search();
            }
        }
    }

    /// Root name for a result.
    pub fn search_root_name(&self, root: RootKey) -> String {
        self.workspace.root(RootId(root)).map(|r| r.display_name()).unwrap_or_default()
    }

    pub(crate) fn search_click_row(&mut self, row: usize, double: bool) {
        self.search.field = SearchField::Results;
        self.search.selected = row;
        self.search_open_selected(!double);
    }
}

/// The row of a unified diff that shows new-file line `line` (0-based):
/// its `+` line, or the context line, or the hunk header above it.
fn diff_row_for_new_line(diff: &str, line: usize) -> Option<usize> {
    let mut new_line = None::<usize>;
    let mut best = None;
    for (row, l) in diff.lines().enumerate() {
        if let Some(rest) = l.strip_prefix("@@ ") {
            let start = rest
                .split_whitespace()
                .find_map(|p| p.strip_prefix('+'))
                .and_then(|p| p.split(',').next())
                .and_then(|n| n.parse::<usize>().ok())
                .unwrap_or(1);
            new_line = Some(start.saturating_sub(1));
            if start.saturating_sub(1) <= line {
                best = Some(row);
            }
            continue;
        }
        let Some(n) = new_line.as_mut() else { continue };
        if l.starts_with('-') {
            continue;
        }
        if *n == line {
            return Some(row);
        }
        *n += 1;
    }
    best
}

#[cfg(test)]
mod tests {
    #[test]
    fn diff_rows() {
        let d = "--- a/f\n+++ b/f\n@@ -2,3 +2,3 @@\n a\n-b\n+B\n c\n";
        assert_eq!(super::diff_row_for_new_line(d, 2), Some(5));
        assert_eq!(super::diff_row_for_new_line(d, 1), Some(3));
        assert_eq!(super::diff_row_for_new_line(d, 9), Some(2));
        assert_eq!(super::diff_row_for_new_line(d, 0), None);
    }
}
