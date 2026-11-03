//! Transient overlays drawn above the workbench (spec §3.1): text prompts,
//! confirmation dialogs and the filterable picker (the base for Quick Open
//! in M4). Overlays receive keys before the keymap.

use std::path::{Path, PathBuf};

use unicode_segmentation::UnicodeSegmentation;
use yace_keymap::{KeyCode, KeyPress, Mods};

use crate::actions::{ConfirmAction, PickerAction, PromptAction};

/// A single-line text input with grapheme-aware editing.
#[derive(Debug, Clone, Default)]
pub struct LineInput {
    pub text: String,
    /// Cursor as a byte offset (always on a grapheme boundary).
    pub cursor: usize,
    /// Select-all state: the next typed char replaces everything.
    pub all_selected: bool,
}

impl LineInput {
    pub fn new(text: impl Into<String>) -> Self {
        let text = text.into();
        Self { cursor: text.len(), text, all_selected: false }
    }
    pub fn with_selection(text: impl Into<String>) -> Self {
        Self { all_selected: true, ..Self::new(text) }
    }
    fn prev(&self) -> usize {
        self.text[..self.cursor].grapheme_indices(true).next_back().map_or(0, |(i, _)| i)
    }
    fn next(&self) -> usize {
        self.text[self.cursor..].graphemes(true).next().map_or(self.cursor, |g| self.cursor + g.len())
    }
    fn word_left(&self) -> usize {
        let before = &self.text[..self.cursor];
        let trimmed = before.trim_end_matches(|c: char| !c.is_alphanumeric());
        trimmed.rfind(|c: char| !c.is_alphanumeric()).map_or(0, |i| i + 1)
    }
    pub fn insert(&mut self, s: &str) {
        if self.all_selected {
            self.text.clear();
            self.cursor = 0;
            self.all_selected = false;
        }
        let s: String = s.chars().filter(|c| *c != '\n' && *c != '\r').collect();
        self.text.insert_str(self.cursor, &s);
        self.cursor += s.len();
    }
    /// Handle an editing key. Returns true if it was consumed.
    pub fn handle(&mut self, key: KeyPress, ch: Option<char>) -> bool {
        let ctrl = key.mods.contains(Mods::CTRL);
        let had_all = std::mem::replace(&mut self.all_selected, false);
        match key.code {
            KeyCode::Backspace if had_all => {
                self.text.clear();
                self.cursor = 0;
            }
            KeyCode::Backspace if ctrl => {
                let start = self.word_left();
                self.text.replace_range(start..self.cursor, "");
                self.cursor = start;
            }
            KeyCode::Backspace => {
                let p = self.prev();
                self.text.replace_range(p..self.cursor, "");
                self.cursor = p;
            }
            KeyCode::Delete => {
                let n = self.next();
                self.text.replace_range(self.cursor..n, "");
            }
            KeyCode::Left if ctrl => self.cursor = self.word_left(),
            KeyCode::Left => self.cursor = self.prev(),
            KeyCode::Right => self.cursor = self.next(),
            KeyCode::Home => self.cursor = 0,
            KeyCode::End => self.cursor = self.text.len(),
            KeyCode::Char('a') if ctrl => self.all_selected = true,
            KeyCode::Char('u') if ctrl => {
                self.text.replace_range(..self.cursor, "");
                self.cursor = 0;
            }
            _ => {
                if let Some(c) = key.text_char(ch) {
                    self.all_selected = had_all;
                    self.insert(&c.to_string());
                } else {
                    self.all_selected = had_all;
                    return false;
                }
            }
        }
        true
    }
    /// Cursor position in display cells.
    pub fn cursor_cells(&self) -> usize {
        unicode_width::UnicodeWidthStr::width(&self.text[..self.cursor])
    }
}

#[derive(Debug, Clone)]
pub struct Prompt {
    pub title: String,
    pub hint: Option<String>,
    pub input: LineInput,
    pub action: PromptAction,
    /// Tab completes filesystem paths.
    pub path_completion: bool,
    pub error: Option<String>,
}

#[derive(Debug, Clone)]
pub struct Confirm {
    pub title: String,
    pub message: Vec<String>,
    pub buttons: Vec<(String, ConfirmAction)>,
    pub selected: usize,
    /// Action for Escape.
    pub cancel: ConfirmAction,
}

#[derive(Debug, Clone)]
pub struct PickerItem {
    pub label: String,
    pub detail: String,
    pub hint: String,
    pub data: usize,
}

impl PickerItem {
    pub fn new(label: impl Into<String>, detail: impl Into<String>, hint: impl Into<String>, data: usize) -> Self {
        Self { label: label.into(), detail: detail.into(), hint: hint.into(), data }
    }
}

/// How a picker's items are filtered.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PickerKind {
    /// Items are fixed; the picker filters them itself (fuzzy).
    Static,
    /// The app supplies items as the query changes (Quick Open).
    Dynamic,
}

#[derive(Debug, Clone)]
pub struct Picker {
    pub title: String,
    pub input: LineInput,
    pub items: Vec<PickerItem>,
    pub filtered: Vec<usize>,
    pub selected: usize,
    pub scroll: usize,
    pub action: PickerAction,
    pub empty_text: String,
    pub kind: PickerKind,
    /// Char indices of `label` to highlight, parallel to `items`.
    pub highlights: Vec<Vec<usize>>,
    /// Extra status shown in the title (e.g. "indexing… 1,234 files").
    pub status: String,
}

impl Picker {
    pub fn new(title: impl Into<String>, items: Vec<PickerItem>, action: PickerAction) -> Self {
        let mut p = Self {
            title: title.into(),
            input: LineInput::default(),
            filtered: Vec::new(),
            items,
            selected: 0,
            scroll: 0,
            action,
            empty_text: "No matching results".into(),
            kind: PickerKind::Static,
            highlights: Vec::new(),
            status: String::new(),
        };
        p.refilter();
        p
    }

    /// A picker whose items are supplied by the app as the query changes.
    pub fn dynamic(title: impl Into<String>, action: PickerAction) -> Self {
        let mut p = Self::new(title, Vec::new(), action);
        p.kind = PickerKind::Dynamic;
        p
    }

    /// Replace the items (already ranked) of a dynamic picker, keeping the
    /// selection on the same item when possible.
    pub fn set_items(&mut self, items: Vec<PickerItem>, highlights: Vec<Vec<usize>>, keep_selection: bool) {
        let prev = self.current().map(|i| i.label.clone() + &i.detail);
        self.filtered = (0..items.len()).collect();
        self.items = items;
        self.highlights = highlights;
        self.selected = 0;
        if keep_selection
            && let Some(prev) = prev
            && let Some(i) = self.items.iter().position(|it| it.label.clone() + &it.detail == prev)
        {
            self.selected = i;
        }
        self.scroll = 0;
    }

    pub fn highlight(&self, item: usize) -> &[usize] {
        self.highlights.get(item).map(Vec::as_slice).unwrap_or(&[])
    }
    /// Fuzzy filter over label (and detail as a fallback), best first.
    pub fn refilter(&mut self) {
        if self.kind == PickerKind::Dynamic {
            return;
        }
        let q = yace_search::fuzzy::Query::new(&self.input.text);
        let mut scored: Vec<(i32, usize, Vec<usize>)> = self
            .items
            .iter()
            .enumerate()
            .filter_map(|(i, it)| match yace_search::fuzzy::match_label(&q, &it.label) {
                Some(m) => Some((m.score, i, m.indices)),
                None => yace_search::fuzzy::match_label(&q, &it.detail).map(|m| (m.score - 1000, i, Vec::new())),
            })
            .collect();
        // Stable: equal scores keep the original (e.g. recently used) order.
        scored.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
        self.highlights = vec![Vec::new(); self.items.len()];
        self.filtered = scored
            .into_iter()
            .map(|(_, i, idx)| {
                self.highlights[i] = idx;
                i
            })
            .collect();
        self.selected = 0;
        self.scroll = 0;
    }
    pub fn current(&self) -> Option<&PickerItem> {
        self.filtered.get(self.selected).map(|i| &self.items[*i])
    }
    pub fn move_by(&mut self, d: isize) {
        if self.filtered.is_empty() {
            return;
        }
        let n = self.filtered.len() as isize;
        self.selected = (self.selected as isize + d).rem_euclid(n) as usize;
    }
}

#[derive(Debug, Clone)]
pub enum Overlay {
    Prompt(Prompt),
    Confirm(Confirm),
    Picker(Picker),
    /// Read-only information (e.g. performance metrics).
    Info {
        title: String,
        lines: Vec<String>,
    },
}

/// Outcome of a key press inside an overlay.
#[derive(Debug, Clone, PartialEq)]
pub enum OverlayOutcome {
    /// Key consumed; overlay stays.
    Consumed,
    /// Close the overlay without action.
    Cancel,
    SubmitPrompt(String),
    Confirm(ConfirmAction),
    Pick(usize),
    /// The input of a dynamic picker changed; the app must refresh items.
    QueryChanged,
    /// Quick Open: toggle including ignored files (Alt+I).
    ToggleIgnored,
}

impl Overlay {
    pub fn handle_key(&mut self, key: KeyPress, ch: Option<char>) -> OverlayOutcome {
        let plain = key.mods.is_empty();
        match self {
            Overlay::Info { .. } => match key.code {
                KeyCode::Escape | KeyCode::Enter | KeyCode::Space => OverlayOutcome::Cancel,
                _ => OverlayOutcome::Consumed,
            },
            Overlay::Prompt(p) => match key.code {
                KeyCode::Escape => OverlayOutcome::Cancel,
                KeyCode::Enter => OverlayOutcome::SubmitPrompt(p.input.text.clone()),
                KeyCode::Tab if p.path_completion => {
                    if let Some(c) = complete_path(&p.input.text) {
                        p.input = LineInput::new(c);
                    }
                    OverlayOutcome::Consumed
                }
                _ => {
                    if p.input.handle(key, ch) {
                        p.error = None;
                    }
                    OverlayOutcome::Consumed
                }
            },
            Overlay::Confirm(c) => match key.code {
                KeyCode::Escape => OverlayOutcome::Confirm(c.cancel.clone()),
                KeyCode::Left | KeyCode::Up => {
                    c.selected = c.selected.saturating_sub(1);
                    OverlayOutcome::Consumed
                }
                KeyCode::Right | KeyCode::Down | KeyCode::Tab => {
                    c.selected = (c.selected + 1).min(c.buttons.len().saturating_sub(1));
                    OverlayOutcome::Consumed
                }
                KeyCode::Enter | KeyCode::Space => OverlayOutcome::Confirm(c.buttons[c.selected].1.clone()),
                KeyCode::Char(k) if plain || key.mods == Mods::SHIFT => {
                    // Accelerator: first letter of a button label.
                    let hit = c
                        .buttons
                        .iter()
                        .position(|(l, _)| l.chars().next().is_some_and(|f| f.eq_ignore_ascii_case(&k)));
                    match hit {
                        Some(i) => OverlayOutcome::Confirm(c.buttons[i].1.clone()),
                        None => OverlayOutcome::Consumed,
                    }
                }
                _ => OverlayOutcome::Consumed,
            },
            Overlay::Picker(p) => match key.code {
                KeyCode::Escape => OverlayOutcome::Cancel,
                KeyCode::Enter => match p.current() {
                    Some(it) => OverlayOutcome::Pick(it.data),
                    None => OverlayOutcome::Consumed,
                },
                KeyCode::Up => {
                    p.move_by(-1);
                    OverlayOutcome::Consumed
                }
                KeyCode::Down => {
                    p.move_by(1);
                    OverlayOutcome::Consumed
                }
                KeyCode::PageUp => {
                    p.selected = p.selected.saturating_sub(10);
                    OverlayOutcome::Consumed
                }
                KeyCode::PageDown => {
                    p.selected = (p.selected + 10).min(p.filtered.len().saturating_sub(1));
                    OverlayOutcome::Consumed
                }
                KeyCode::Char('i') if key.mods == Mods::ALT && p.kind == PickerKind::Dynamic => {
                    OverlayOutcome::ToggleIgnored
                }
                _ => {
                    let before = p.input.text.clone();
                    p.input.handle(key, ch);
                    if p.input.text != before {
                        if p.kind == PickerKind::Dynamic {
                            return OverlayOutcome::QueryChanged;
                        }
                        p.refilter();
                    }
                    OverlayOutcome::Consumed
                }
            },
        }
    }

    pub fn paste(&mut self, text: &str) {
        match self {
            Overlay::Prompt(p) => p.input.insert(text),
            Overlay::Picker(p) => {
                p.input.insert(text);
                p.refilter();
            }
            _ => {}
        }
    }
}

/// Complete the last path component (longest common prefix of matches).
pub fn complete_path(input: &str) -> Option<String> {
    let expanded = yace_workspace::paths::expand_tilde(input);
    let (dir, prefix) = if input.ends_with('/') {
        (expanded.clone(), String::new())
    } else {
        let dir = expanded.parent().map(Path::to_path_buf).unwrap_or_else(|| PathBuf::from("."));
        let prefix = expanded.file_name().map(|f| f.to_string_lossy().into_owned()).unwrap_or_default();
        (if dir.as_os_str().is_empty() { PathBuf::from(".") } else { dir }, prefix)
    };
    let mut matches: Vec<(String, bool)> = std::fs::read_dir(&dir)
        .ok()?
        .filter_map(Result::ok)
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            name.starts_with(&prefix).then(|| (name, e.path().is_dir()))
        })
        .collect();
    if matches.is_empty() {
        return None;
    }
    matches.sort();
    let first = &matches[0].0;
    let mut common = first.len();
    for (m, _) in &matches[1..] {
        common = common.min(first.bytes().zip(m.bytes()).take_while(|(a, b)| a == b).count());
    }
    while !first.is_char_boundary(common) {
        common -= 1;
    }
    let completed = &first[..common];
    let base = &input[..input.len() - prefix.len()];
    let mut out = format!("{base}{completed}");
    if matches.len() == 1 && matches[0].1 {
        out.push('/');
    }
    (out != input).then_some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn k(s: &str) -> KeyPress {
        s.parse().unwrap()
    }

    #[test]
    fn line_input_editing() {
        let mut li = LineInput::new("héllo wörld");
        li.handle(k("ctrl+backspace"), None);
        assert_eq!(li.text, "héllo ");
        li.handle(k("left"), None);
        li.handle(k("backspace"), None);
        assert_eq!(li.text, "héll ");
        li.handle(KeyPress::new(KeyCode::Char('x'), Mods::NONE), Some('x'));
        assert_eq!(li.text, "héllx ");
        let mut sel = LineInput::with_selection("old");
        sel.handle(KeyPress::new(KeyCode::Char('n'), Mods::NONE), Some('n'));
        assert_eq!(sel.text, "n");
    }

    #[test]
    fn picker_filters_and_wraps() {
        let items = ["View: Toggle Sidebar", "File: Save", "Workspace: Add Folder"]
            .iter()
            .enumerate()
            .map(|(i, l)| PickerItem { label: l.to_string(), detail: String::new(), hint: String::new(), data: i })
            .collect();
        let mut p = Picker::new("Commands", items, PickerAction::RunCommand);
        p.input.insert("fold");
        p.refilter();
        assert_eq!(p.filtered, vec![2]);
        p.input = LineInput::default();
        p.refilter();
        p.move_by(-1);
        assert_eq!(p.selected, 2);
    }

    #[test]
    fn path_completion() {
        let d = tempfile::tempdir().unwrap();
        std::fs::create_dir(d.path().join("alpha-dir")).unwrap();
        std::fs::write(d.path().join("beta.txt"), "").unwrap();
        let base = format!("{}/al", d.path().display());
        assert_eq!(complete_path(&base).unwrap(), format!("{}/alpha-dir/", d.path().display()));
        let b = format!("{}/b", d.path().display());
        assert_eq!(complete_path(&b).unwrap(), format!("{}/beta.txt", d.path().display()));
    }
}
