//! Find / replace in the active editor (spec §9.7). A non-modal bar at the
//! top of the active group; matching runs on the find worker against a
//! rope snapshot, so typing in large files never blocks.

use yace_buffer::{Change, ChangeSet, EditKind, Range, Selection, ops::Edit};
use yace_keymap::{KeyCode, KeyPress};
use yace_search::{FindOptions, FindResult, find};

use super::{App, Effect, Focus, Severity};
use crate::{editor::DocId, overlay::LineInput, workers::FindJob};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FindField {
    #[default]
    Query,
    Replace,
}

#[derive(Debug, Default)]
pub struct FindState {
    pub visible: bool,
    pub replace_visible: bool,
    pub field: FindField,
    pub query: LineInput,
    pub replace: LineInput,
    pub opts: FindOptions,
    /// Restrict to this char range (Find in Selection).
    pub within: Option<(usize, usize)>,
    pub matches: Vec<(usize, usize)>,
    pub truncated: bool,
    pub error: Option<String>,
    pub current: Option<usize>,
    pub seq: u64,
    /// Document and version the matches belong to.
    pub doc: Option<DocId>,
    pub version: u64,
    pub searching: bool,
    jump_on_result: bool,
    history: Vec<String>,
    hist_pos: Option<usize>,
}

impl FindState {
    fn push_history(&mut self) {
        let q = self.query.text.clone();
        if q.is_empty() {
            return;
        }
        self.history.retain(|h| *h != q);
        self.history.push(q);
        if self.history.len() > 50 {
            self.history.remove(0);
        }
        self.hist_pos = None;
    }

    /// "3 of 41", "No results", or an error.
    pub fn status(&self) -> String {
        if let Some(e) = &self.error {
            return e.clone();
        }
        if self.query.text.is_empty() {
            return String::new();
        }
        if self.searching && self.matches.is_empty() {
            return "…".into();
        }
        if self.matches.is_empty() {
            return "No results".into();
        }
        let total = if self.truncated { format!("{}+", self.matches.len()) } else { self.matches.len().to_string() };
        match self.current {
            Some(i) => format!("{} of {total}", i + 1),
            None => format!("? of {total}"),
        }
    }
}

impl App {
    /// `Ctrl+F` / `Ctrl+H`.
    pub(crate) fn find_open(&mut self, replace: bool) {
        let Some((g, v, d)) = self.active() else { return };
        // Seed from a single-line selection.
        let sel = self.layout.group(g).map(|gr| gr.views[v].sel.primary());
        if let (Some(r), Some(doc)) = (sel, self.docs.doc(d))
            && !r.is_empty()
        {
            let text: String = doc.rope().slice(r.from()..r.to()).into();
            if !text.contains('\n') {
                self.find.query = LineInput::with_selection(text);
            } else {
                self.find.within = Some((r.from(), r.to()));
            }
        } else {
            self.find.query.all_selected = true;
        }
        self.find.visible = true;
        self.find.replace_visible = replace || self.find.replace_visible && self.find.visible;
        if replace {
            self.find.replace_visible = true;
        }
        self.find.field =
            if replace && !self.find.query.text.is_empty() { FindField::Replace } else { FindField::Query };
        self.focus_changed(Focus::Find);
        self.request_find(true);
    }

    pub(crate) fn find_close(&mut self) {
        self.find.visible = false;
        self.find.within = None;
        self.find.matches.clear();
        self.find.current = None;
        if self.focus == Focus::Find {
            self.focus_changed(Focus::Editor);
        }
    }

    /// Ask the find worker for matches in the active document.
    pub(crate) fn request_find(&mut self, jump: bool) {
        let Some((_, _, d)) = self.active() else { return };
        let Some(doc) = self.docs.doc(d) else { return };
        self.find.doc = Some(d);
        self.find.version = doc.version();
        self.find.jump_on_result = jump;
        if self.find.query.text.is_empty() {
            self.find.matches.clear();
            self.find.current = None;
            self.find.error = None;
            self.find.searching = false;
            return;
        }
        self.find.seq += 1;
        self.find.searching = true;
        let job = FindJob {
            seq: self.find.seq,
            rope: doc.rope().clone(),
            query: self.find.query.text.clone(),
            opts: self.find.opts,
            within: self.find.within,
        };
        self.emit(Effect::Find(Box::new(job)));
    }

    /// Re-run find when the document under the bar changed.
    pub(crate) fn refresh_find_if_stale(&mut self) {
        if !self.find.visible {
            return;
        }
        let Some((_, _, d)) = self.active() else { return };
        let version = self.docs.doc(d).map(|x| x.version()).unwrap_or(0);
        if self.find.doc != Some(d) || self.find.version != version {
            self.request_find(false);
        }
    }

    pub(crate) fn on_find_done(&mut self, seq: u64, result: Result<FindResult, String>) {
        if seq != self.find.seq {
            return;
        }
        self.find.searching = false;
        match result {
            Ok(r) => {
                self.find.matches = r.matches;
                self.find.truncated = r.truncated;
                self.find.error = None;
            }
            Err(e) => {
                self.find.matches.clear();
                self.find.error = Some(e);
            }
        }
        let head = self.active_head();
        self.find.current = match self.find.current {
            _ if self.find.matches.is_empty() => None,
            _ => Some(self.find.matches.partition_point(|(a, _)| *a < head).min(self.find.matches.len() - 1)),
        };
        if std::mem::take(&mut self.find.jump_on_result) {
            // While typing: move the selection to the nearest match at/after
            // where the search started.
            let from = self.active_selection().map(|s| s.primary().from()).unwrap_or(0);
            if let Some(i) = self.nearest_match(from) {
                self.select_match(i);
            }
        }
    }

    fn active_head(&self) -> usize {
        self.active_selection().map(|s| s.primary().head).unwrap_or(0)
    }

    fn active_selection(&self) -> Option<Selection> {
        let (g, v, _) = self.active()?;
        Some(self.layout.group(g)?.views[v].sel.clone())
    }

    fn nearest_match(&self, pos: usize) -> Option<usize> {
        if self.find.matches.is_empty() {
            return None;
        }
        let i = self.find.matches.partition_point(|(a, _)| *a < pos);
        Some(if i >= self.find.matches.len() { 0 } else { i })
    }

    fn select_match(&mut self, i: usize) {
        let Some(&(a, b)) = self.find.matches.get(i) else { return };
        self.find.current = Some(i);
        if let Some((g, v, _)) = self.active()
            && let Some(view) = self.layout.group_mut(g).and_then(|gr| gr.views.get_mut(v))
        {
            view.sel = Selection::single(Range::new(a, b));
        }
        // Keep the match in view with the usual margins.
        self.reveal_cursor = true;
    }

    /// `Enter` / `F3` (and `Shift+…` for previous).
    pub(crate) fn find_next(&mut self, forward: bool) {
        self.find.push_history();
        if self.find.matches.is_empty() {
            if !self.find.visible {
                self.find_open(false);
            }
            return;
        }
        let sel = self.active_selection().map(|s| s.primary()).unwrap_or(Range::point(0));
        let n = self.find.matches.len();
        let on_match = self.find.matches.iter().any(|m| m.0 == sel.from() && m.1 == sel.to());
        let i = if forward {
            let pos = if on_match { sel.from() + 1 } else { sel.from() };
            self.find.matches.partition_point(|(a, _)| *a < pos) % n
        } else {
            (self.find.matches.partition_point(|(a, _)| *a < sel.from()) + n - 1) % n
        };
        self.select_match(i);
    }

    /// `Alt+Enter`: every match becomes a cursor.
    pub(crate) fn find_select_all(&mut self) {
        if self.find.matches.is_empty() {
            return;
        }
        let ranges = self.find.matches.iter().map(|(a, b)| Range::new(*a, *b));
        let primary = self.find.current.unwrap_or(0);
        let sel = Selection::new(ranges, primary);
        if let Some((g, v, _)) = self.active()
            && let Some(view) = self.layout.group_mut(g).and_then(|gr| gr.views.get_mut(v))
        {
            view.sel = sel;
        }
        self.find_close();
    }

    fn find_regex(&mut self) -> Option<yace_search::regex::Regex> {
        match find::build_regex(&self.find.query.text, self.find.opts) {
            Ok(r) => Some(r),
            Err(e) => {
                self.find.error = Some(e);
                None
            }
        }
    }

    /// Replace the current match, then move to the next.
    pub(crate) fn replace_one(&mut self) {
        let Some((_, _, d)) = self.active() else { return };
        let Some(re) = self.find_regex() else { return };
        let Some(i) = self.find.current.or_else(|| self.nearest_match(self.active_head())) else { return };
        let Some(&(a, b)) = self.find.matches.get(i) else { return };
        let Some(text) = self.docs.doc(d).map(|x| x.text()) else { return };
        let byte_a = text.char_indices().nth(a).map_or(text.len(), |(bi, _)| bi);
        let rep = find::expand_replacement(&re, &text, byte_a, &self.find.replace.text, self.find.opts.regex);
        let len = self.docs.doc(d).map(|x| x.len_chars()).unwrap_or(0);
        let n = rep.chars().count();
        let cs = ChangeSet::new(vec![Change::replace(a, b, rep)], len);
        self.apply_edit(Edit { changes: cs, selection: Selection::point(a + n), kind: EditKind::Other });
        self.find.push_history();
        // Matches are refreshed by the worker; jump to the next one.
        self.request_find(true);
    }

    /// Replace every match as a single undo step.
    pub(crate) fn replace_all(&mut self) {
        let Some((_, _, d)) = self.active() else { return };
        let Some(re) = self.find_regex() else { return };
        let Some(doc) = self.docs.doc(d) else { return };
        let text = doc.text();
        let len = doc.len_chars();
        let within = self.find.within;
        let reps: Vec<Change> = find::replacements(&text, &re, &self.find.replace.text, self.find.opts.regex)
            .into_iter()
            .filter(|(a, b, _)| within.is_none_or(|(x, y)| *a >= x && *b <= y))
            .map(|(a, b, r)| Change::replace(a, b, r))
            .collect();
        let count = reps.len();
        if count == 0 {
            return;
        }
        let cs = ChangeSet::new(reps, len);
        let sel = self.active_selection().unwrap_or_default().map(&cs);
        self.apply_edit(Edit { changes: cs, selection: sel, kind: EditKind::Other });
        self.find.push_history();
        self.notify(Severity::Info, format!("Replaced {count} occurrence{}", if count == 1 { "" } else { "s" }));
        self.request_find(false);
    }

    pub(crate) fn find_toggle(&mut self, what: char) {
        match what {
            'c' => self.find.opts.case_sensitive = !self.find.opts.case_sensitive,
            'w' => self.find.opts.whole_word = !self.find.opts.whole_word,
            'r' => self.find.opts.regex = !self.find.opts.regex,
            'l' => {
                self.find.within = match self.find.within {
                    Some(_) => None,
                    None => self
                        .active_selection()
                        .map(|s| s.primary())
                        .filter(|r| !r.is_empty())
                        .map(|r| (r.from(), r.to())),
                }
            }
            _ => {}
        }
        self.request_find(true);
    }

    pub(crate) fn find_toggle_field(&mut self) {
        if self.find.replace_visible {
            self.find.field = match self.find.field {
                FindField::Query => FindField::Replace,
                FindField::Replace => FindField::Query,
            };
        }
    }

    pub(crate) fn find_input_key(&mut self, key: KeyPress, ch: Option<char>) {
        if self.find.field == FindField::Query && matches!(key.code, KeyCode::Up | KeyCode::Down) && key.mods.is_empty()
        {
            let h = &self.find.history;
            if h.is_empty() {
                return;
            }
            let pos = match (self.find.hist_pos, key.code) {
                (None, KeyCode::Up) => h.len() - 1,
                (Some(p), KeyCode::Up) => p.saturating_sub(1),
                (Some(p), KeyCode::Down) if p + 1 < h.len() => p + 1,
                _ => return,
            };
            self.find.hist_pos = Some(pos);
            self.find.query = LineInput::new(h[pos].clone());
            self.request_find(true);
            return;
        }
        let field = match self.find.field {
            FindField::Query => &mut self.find.query,
            FindField::Replace => &mut self.find.replace,
        };
        let before = field.text.clone();
        field.handle(key, ch);
        if self.find.field == FindField::Query && self.find.query.text != before {
            self.request_find(true);
        }
    }

    pub(crate) fn find_paste(&mut self, text: &str) {
        let first_line = text.lines().next().unwrap_or("");
        match self.find.field {
            FindField::Query => {
                self.find.query.insert(first_line);
                self.request_find(true);
            }
            FindField::Replace => self.find.replace.insert(first_line),
        }
    }
}
