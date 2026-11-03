//! Workspace search view in the sidebar (spec §4.6, §14): query, replace,
//! include/exclude, streamed results grouped by root → file → line, and
//! preview-before-apply replacement that never silently overwrites open
//! buffers (§14.3).

use std::{
    collections::{HashMap, HashSet},
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant},
};

use yace_buffer::{Change, ChangeSet};
use yace_keymap::{KeyCode, KeyPress, Mods};
use yace_search::{
    ContentQuery, FileMatches, FindOptions, LineMatch, RootKey, SearchRoot, SearchSummary, WalkOptions,
    content::replace_in_file, find,
};
use yace_workspace::{RootId, RootStatus};

use super::{App, Effect, Focus, Severity, SidebarView};
use crate::{
    actions::ConfirmAction,
    overlay::{Confirm, LineInput, Overlay},
    workers::SearchJob,
};

const DEBOUNCE: Duration = Duration::from_millis(150);

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
}

impl SearchState {
    pub fn input_focused(&self) -> bool {
        self.field != SearchField::Results
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
        if pattern.is_empty() {
            self.search.running = false;
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
            },
            max_results: s.search.max_results.max(1),
            max_per_file: 1_000,
        };
        // Dirty open documents are searched as the user sees them.
        let mut overlays: HashMap<PathBuf, Arc<str>> = HashMap::new();
        for (_, e) in self.docs.iter() {
            if e.doc.is_dirty()
                && let Some(p) = e.doc.path()
            {
                overlays.insert(p.to_path_buf(), Arc::from(e.doc.text()));
            }
        }
        self.search.root_order = self.workspace.roots().iter().map(|r| r.id.0).collect();
        self.search.running = true;
        let job = SearchJob { seq: self.search.seq, roots: self.search_roots(), query, overlays };
        self.emit(Effect::Search(Box::new(job)));
    }

    pub(crate) fn on_search_file(&mut self, seq: u64, f: FileMatches) {
        if seq != self.search.seq {
            return;
        }
        let order = |r: RootKey| self.search.root_order.iter().position(|x| *x == r).unwrap_or(usize::MAX);
        let key = (order(f.root), f.rel.clone());
        let at = self.search.files.partition_point(|x| (order(x.root), x.rel.clone()) < key);
        let result = FileResult {
            root: f.root,
            rel: f.rel,
            path: f.path,
            hash: f.hash,
            from_buffer: f.from_buffer,
            matches: f.matches,
            expanded: true,
            excluded: HashSet::new(),
            file_excluded: false,
        };
        self.search.files.insert(at, result);
    }

    pub(crate) fn on_search_done(&mut self, seq: u64, summary: SearchSummary, error: Option<String>) {
        if seq != self.search.seq {
            return;
        }
        self.search.running = false;
        self.search.summary = Some(summary);
        self.search.error = error;
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
            let mut skip: HashSet<(usize, usize)> = HashSet::new();
            for mi in &f.excluded {
                let lm = &f.matches[*mi];
                skip.extend(lm.ranges.iter().map(|r| (lm.line, r.0)));
            }
            if skip.len() >= f.matches.iter().map(|m| m.ranges.len()).sum::<usize>() && !f.matches.is_empty() {
                continue;
            }
            if let Some(doc) = self.docs.find_by_path(&f.path) {
                // Open document: undoable edit, left unsaved.
                let Some(d) = self.docs.doc(doc) else { continue };
                let text = d.text();
                let rope = d.rope().clone();
                let changes: Vec<Change> = find::replacements(&text, &re, &replacement, regex_mode)
                    .into_iter()
                    .filter(|(a, _, _)| {
                        let line = rope.char_to_line(*a);
                        !skip.contains(&(line, a - rope.line_to_char(line)))
                    })
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
