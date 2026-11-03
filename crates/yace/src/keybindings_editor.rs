//! The keybinding editor (spec §5.4, `Ctrl+K Ctrl+S`): every command with
//! its keys, when-clause and source; search by text or by recorded keys;
//! define, remove or reset bindings. The app turns `KbEdit`s into edits of
//! `keybindings.toml`.

use yace_keymap::{KeyCode, KeyPress, KeySeq, Mods, Source};

use crate::overlay::LineInput;

#[derive(Debug, Clone, PartialEq)]
pub struct KbRow {
    pub command: String,
    /// "Category: Title", or the command id for commands without a title.
    pub title: String,
    pub keys: Option<KeySeq>,
    /// When-clause as written (empty: always).
    pub when: String,
    /// None for a command with no binding.
    pub source: Option<Source>,
    /// Same keys and when-clause as a binding of another command.
    pub conflict: bool,
}

impl KbRow {
    pub fn keys_text(&self) -> String {
        self.keys.as_ref().map(ToString::to_string).unwrap_or_default()
    }
    pub fn source_text(&self) -> &'static str {
        match self.source {
            None => "",
            Some(Source::Default) => "Default",
            Some(Source::Plugin) => "Plugin",
            Some(Source::User) => "User",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum KbMode {
    /// Typing filters the list.
    Search,
    /// Key presses filter the list (`Alt+K`).
    RecordSearch(Vec<KeyPress>),
    /// Recording new keys for the selected row (Enter accepts).
    Define(Vec<KeyPress>),
}

/// An edit requested by the editor, applied by the app.
#[derive(Debug, Clone, PartialEq)]
pub enum KbEdit {
    Define { row: KbRow, keys: KeySeq },
    Remove(KbRow),
    Reset(KbRow),
}

#[derive(Debug, Clone)]
pub struct KeybindingEditor {
    pub input: LineInput,
    pub rows: Vec<KbRow>,
    pub filtered: Vec<usize>,
    pub selected: usize,
    pub mode: KbMode,
    /// Result of the last edit, shown in the footer.
    pub note: String,
}

/// Longest key sequence the editor records (chords of two, like VS Code).
const MAX_CHORD: usize = 2;

impl KeybindingEditor {
    pub fn new(rows: Vec<KbRow>) -> Self {
        let mut e = Self {
            input: LineInput::default(),
            filtered: Vec::new(),
            rows,
            selected: 0,
            mode: KbMode::Search,
            note: String::new(),
        };
        e.refilter();
        e
    }

    /// Replace the rows (after an edit), keeping the filter and the
    /// selection on the same command when possible.
    pub fn set_rows(&mut self, rows: Vec<KbRow>, select_command: Option<&str>) {
        let prev = select_command.map(str::to_string).or_else(|| self.current().map(|r| r.command.clone()));
        self.rows = rows;
        self.refilter();
        if let Some(cmd) = prev
            && let Some(i) = self.filtered.iter().position(|&r| self.rows[r].command == cmd)
        {
            self.selected = i;
        }
    }

    pub fn current(&self) -> Option<&KbRow> {
        self.filtered.get(self.selected).map(|&i| &self.rows[i])
    }

    fn matches(&self, row: &KbRow) -> bool {
        if let KbMode::RecordSearch(keys) = &self.mode {
            return !keys.is_empty() && row.keys.as_ref().is_some_and(|k| k.starts_with(keys));
        }
        let keys = row.keys_text().to_lowercase();
        self.input.text.split_whitespace().all(|word| {
            let w = word.to_lowercase();
            match w.as_str() {
                "@conflict" | "@conflicts" => row.conflict,
                "@user" | "@source:user" => row.source == Some(Source::User),
                "@default" | "@source:default" => row.source == Some(Source::Default),
                "@plugin" | "@source:plugin" => row.source == Some(Source::Plugin),
                "@unbound" => row.source.is_none(),
                _ => {
                    row.title.to_lowercase().contains(&w)
                        || row.command.to_lowercase().contains(&w)
                        || keys.contains(&w)
                        || row.when.to_lowercase().contains(&w)
                }
            }
        })
    }

    pub fn refilter(&mut self) {
        self.filtered = (0..self.rows.len()).filter(|&i| self.matches(&self.rows[i])).collect();
        self.selected = self.selected.min(self.filtered.len().saturating_sub(1));
    }

    fn move_by(&mut self, d: isize) {
        if self.filtered.is_empty() {
            return;
        }
        let n = self.filtered.len() as isize;
        self.selected = (self.selected as isize + d).clamp(0, n - 1) as usize;
    }

    /// Other rows bound to `keys` (shown while defining).
    pub fn bound_to(&self, keys: &[KeyPress]) -> Vec<&KbRow> {
        self.rows.iter().filter(|r| r.keys.as_ref().is_some_and(|k| k.presses() == keys)).collect()
    }

    /// Handle a key. `Some(Ok(edit))` asks the app to apply an edit,
    /// `Some(Err(()))` closes the editor, `None` means handled.
    pub fn handle_key(&mut self, key: KeyPress, ch: Option<char>) -> Option<Result<KbEdit, ()>> {
        let record = |keys: &mut Vec<KeyPress>| {
            if keys.len() >= MAX_CHORD {
                keys.clear();
            }
            keys.push(key);
        };
        match &mut self.mode {
            KbMode::Define(keys) => {
                match key.code {
                    KeyCode::Escape if key.mods.is_empty() => self.mode = KbMode::Search,
                    KeyCode::Enter if key.mods.is_empty() => {
                        if keys.is_empty() {
                            return None;
                        }
                        let seq = KeySeq(keys.iter().copied().collect());
                        let row = self.current().cloned();
                        self.mode = KbMode::Search;
                        return row.map(|row| Ok(KbEdit::Define { row, keys: seq }));
                    }
                    _ => record(keys),
                }
                None
            }
            KbMode::RecordSearch(keys) => {
                match key.code {
                    KeyCode::Escape if key.mods.is_empty() => {
                        self.leave_record_search();
                        return None;
                    }
                    KeyCode::Char('k') if key.mods == Mods::ALT => {
                        self.leave_record_search();
                        return None;
                    }
                    KeyCode::Up | KeyCode::Down | KeyCode::PageUp | KeyCode::PageDown if key.mods.is_empty() => {
                        self.navigate(key.code);
                        return None;
                    }
                    KeyCode::Enter if key.mods.is_empty() && !keys.is_empty() => {
                        self.mode = KbMode::Define(Vec::new());
                        return None;
                    }
                    _ => record(keys),
                }
                self.refilter();
                None
            }
            KbMode::Search => {
                match key.code {
                    KeyCode::Escape => return Some(Err(())),
                    KeyCode::Up | KeyCode::Down | KeyCode::PageUp | KeyCode::PageDown => self.navigate(key.code),
                    KeyCode::Enter => {
                        if self.current().is_some() {
                            self.mode = KbMode::Define(Vec::new());
                        }
                    }
                    KeyCode::Char('k') if key.mods == Mods::ALT => {
                        self.mode = KbMode::RecordSearch(Vec::new());
                        self.refilter();
                    }
                    KeyCode::Char('r') if key.mods == Mods::ALT => {
                        return self.current().filter(|r| r.source.is_some()).cloned().map(|r| Ok(KbEdit::Reset(r)));
                    }
                    // Delete at the end of the search text removes the binding.
                    KeyCode::Delete if self.input.cursor == self.input.text.len() => {
                        return self.current().filter(|r| r.source.is_some()).cloned().map(|r| Ok(KbEdit::Remove(r)));
                    }
                    _ => {
                        let before = self.input.text.clone();
                        self.input.handle(key, ch);
                        if self.input.text != before {
                            self.selected = 0;
                            self.refilter();
                        }
                    }
                }
                None
            }
        }
    }

    /// Back to text search, keeping the selected command selected.
    fn leave_record_search(&mut self) {
        let cmd = self.current().map(|r| (r.command.clone(), r.keys.clone()));
        self.mode = KbMode::Search;
        self.refilter();
        if let Some((cmd, keys)) = cmd
            && let Some(i) =
                self.filtered.iter().position(|&r| self.rows[r].command == cmd && self.rows[r].keys == keys)
        {
            self.selected = i;
        }
    }

    fn navigate(&mut self, code: KeyCode) {
        match code {
            KeyCode::Up => self.move_by(-1),
            KeyCode::Down => self.move_by(1),
            KeyCode::PageUp => self.move_by(-10),
            KeyCode::PageDown => self.move_by(10),
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn k(s: &str) -> KeyPress {
        s.parse().unwrap()
    }
    fn row(cmd: &str, title: &str, keys: Option<&str>, source: Option<Source>) -> KbRow {
        KbRow {
            command: cmd.into(),
            title: title.into(),
            keys: keys.map(|k| k.parse().unwrap()),
            when: String::new(),
            source,
            conflict: false,
        }
    }
    fn editor() -> KeybindingEditor {
        let mut rows = vec![
            row("a.save", "File: Save", Some("ctrl+s"), Some(Source::Default)),
            row("a.theme", "Preferences: Color Theme", Some("ctrl+k ctrl+t"), Some(Source::Default)),
            row("a.mine", "Mine: Thing", Some("ctrl+alt+j"), Some(Source::User)),
            row("a.none", "View: Unbound", None, None),
        ];
        rows[0].conflict = true;
        KeybindingEditor::new(rows)
    }
    fn typ(e: &mut KeybindingEditor, s: &str) {
        for c in s.chars() {
            let kp = if c == ' ' { k("space") } else { KeyPress::new(KeyCode::Char(c), Mods::NONE) };
            e.handle_key(kp, Some(c));
        }
    }

    #[test]
    fn filters_by_text_tokens_and_keys() {
        let mut e = editor();
        assert_eq!(e.filtered.len(), 4);
        typ(&mut e, "theme");
        assert_eq!(e.filtered, vec![1]);
        e.input = LineInput::default();
        typ(&mut e, "ctrl+k");
        assert_eq!(e.filtered, vec![1], "key text matches");
        e.input = LineInput::default();
        typ(&mut e, "@user");
        assert_eq!(e.filtered, vec![2]);
        e.input = LineInput::default();
        typ(&mut e, "@conflicts");
        assert_eq!(e.filtered, vec![0]);
        e.input = LineInput::default();
        typ(&mut e, "@unbound view");
        assert_eq!(e.filtered, vec![3]);
    }

    #[test]
    fn record_keys_search() {
        let mut e = editor();
        e.handle_key(k("alt+k"), None);
        assert!(e.filtered.is_empty(), "nothing recorded yet");
        e.handle_key(k("ctrl+k"), None);
        assert_eq!(e.filtered, vec![1], "chord prefix");
        e.handle_key(k("ctrl+t"), None);
        assert_eq!(e.filtered, vec![1]);
        // A third key starts over.
        e.handle_key(k("ctrl+s"), None);
        assert_eq!(e.filtered, vec![0]);
        e.handle_key(k("escape"), None);
        assert_eq!(e.mode, KbMode::Search);
        assert_eq!(e.filtered.len(), 4);
    }

    #[test]
    fn define_remove_reset() {
        let mut e = editor();
        typ(&mut e, "unbound");
        assert_eq!(e.handle_key(k("enter"), None), None);
        assert_eq!(e.mode, KbMode::Define(vec![]));
        e.handle_key(k("ctrl+k"), None);
        e.handle_key(k("ctrl+u"), None);
        assert_eq!(e.bound_to(&[k("ctrl+k"), k("ctrl+u")]).len(), 0);
        match e.handle_key(k("enter"), None) {
            Some(Ok(KbEdit::Define { row, keys })) => {
                assert_eq!(row.command, "a.none");
                assert_eq!(keys.to_string(), "Ctrl+K Ctrl+U");
            }
            other => panic!("{other:?}"),
        }
        assert_eq!(e.mode, KbMode::Search);
        // Escape while defining cancels without an edit.
        e.handle_key(k("enter"), None);
        e.handle_key(k("ctrl+s"), None);
        assert_eq!(e.bound_to(&[k("ctrl+s")]).len(), 1);
        assert_eq!(e.handle_key(k("escape"), None), None);
        assert_eq!(e.mode, KbMode::Search);
        // Unbound rows can't be removed or reset.
        assert_eq!(e.handle_key(k("delete"), None), None);
        e.input = LineInput::default();
        e.refilter();
        e.selected = 2;
        assert!(matches!(e.handle_key(k("delete"), None), Some(Ok(KbEdit::Remove(r))) if r.command == "a.mine"));
        assert!(matches!(e.handle_key(k("alt+r"), None), Some(Ok(KbEdit::Reset(r))) if r.command == "a.mine"));
        assert_eq!(e.handle_key(k("escape"), None), Some(Err(())));
    }
}
