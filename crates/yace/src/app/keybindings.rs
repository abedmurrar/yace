//! Keyboard Shortcuts editor glue (spec §5.4): builds the rows from the
//! registry and keymap, and applies edits to `keybindings.toml` (comments
//! and order kept), then reloads the keymap.

use std::collections::HashSet;

use yace_config::toml_edit::{self, ArrayOfTables, Item, Table, value};
use yace_keymap::{KeySeq, Source, When};

use super::{App, Severity};
use crate::{
    actions::ConfirmAction,
    keybindings_editor::{KbEdit, KbRow, KeybindingEditor},
    overlay::{Confirm, Overlay},
};

/// The `keybindings.toml` spelling of a key sequence (`ctrl+k ctrl+t`).
pub fn key_text(keys: &KeySeq) -> String {
    keys.to_string().to_lowercase()
}

fn same_when(a: &str, b: &str) -> bool {
    match (When::parse(a), When::parse(b)) {
        (Ok(x), Ok(y)) => x == y,
        _ => a.trim() == b.trim(),
    }
}

/// Does a `[[bindings]]` entry describe this (user) row?
fn entry_matches(t: &Table, row: &KbRow) -> bool {
    let cmd = t.get("command").and_then(Item::as_str);
    let key = t.get("key").and_then(Item::as_str).and_then(|k| k.parse::<KeySeq>().ok());
    let when = t.get("when").and_then(Item::as_str).unwrap_or("");
    cmd == Some(row.command.as_str()) && key.as_ref() == row.keys.as_ref() && same_when(when, &row.when)
}

fn entry(keys: Option<&KeySeq>, command: &str, when: &str) -> Table {
    let mut t = Table::new();
    if let Some(k) = keys {
        t.insert("key", value(key_text(k)));
    }
    t.insert("command", value(command));
    if !when.trim().is_empty() {
        t.insert("when", value(when));
    }
    t
}

impl App {
    /// Rows: every binding, then every palette command without one.
    pub(crate) fn keybinding_rows(&self) -> Vec<KbRow> {
        let conflicts = self.keymap.conflicts();
        let title = |cmd: &str| match self.commands.get(cmd) {
            Some(c) => format!("{}: {}", c.category, c.title),
            None => cmd.to_string(),
        };
        let mut rows = Vec::new();
        let mut bound: HashSet<&str> = HashSet::new();
        for b in self.keymap.bindings() {
            bound.insert(b.command.as_str());
            rows.push(KbRow {
                command: b.command.clone(),
                title: title(&b.command),
                keys: Some(b.keys.clone()),
                when: b.when.to_string(),
                source: Some(b.source),
                conflict: conflicts.iter().any(|c| c.keys == b.keys && c.when == b.when),
            });
        }
        for c in self.commands.all() {
            if c.palette && !bound.contains(c.id) {
                rows.push(KbRow {
                    command: c.id.to_string(),
                    title: title(c.id),
                    keys: None,
                    when: String::new(),
                    source: None,
                    conflict: false,
                });
            }
        }
        rows.sort_by(|a, b| a.title.to_lowercase().cmp(&b.title.to_lowercase()).then_with(|| a.keys.cmp(&b.keys)));
        rows
    }

    /// Preferences: Open Keyboard Shortcuts (`Ctrl+K Ctrl+S`).
    pub(crate) fn open_keybinding_editor(&mut self) {
        let rows = self.keybinding_rows();
        self.open_overlay(Overlay::Keybindings(Box::new(KeybindingEditor::new(rows))));
    }

    fn edit_bindings(&mut self, f: impl FnOnce(&mut ArrayOfTables)) -> Result<(), String> {
        let path = self.paths.keybindings();
        if !path.exists() {
            yace_config::atomic_write(&path, yace_config::DEFAULT_KEYBINDINGS_TEMPLATE.as_bytes())
                .map_err(|e| format!("{}: {e}", path.display()))?;
        }
        yace_config::edit_toml(&path, |doc| {
            // A file of only comments keeps them in the document's trailing
            // text, which would end up below the first new table: move them
            // above it.
            let mut header = String::new();
            if !doc.contains_array_of_tables("bindings") {
                if doc.as_table().is_empty() {
                    header = doc.trailing().as_str().unwrap_or("").to_string();
                    doc.set_trailing("");
                }
                doc.insert("bindings", Item::ArrayOfTables(ArrayOfTables::new()));
            }
            if let Some(arr) = doc.get_mut("bindings").and_then(Item::as_array_of_tables_mut) {
                f(arr);
                if !header.is_empty()
                    && let Some(first) = arr.get_mut(0)
                {
                    let sep = if header.ends_with("\n\n") { "" } else { "\n" };
                    first.decor_mut().set_prefix(format!("{header}{sep}"));
                }
            }
        })
    }

    pub(crate) fn on_keybinding_edit(&mut self, edit: KbEdit) {
        let (result, note, select) = match &edit {
            KbEdit::Define { row, keys } => {
                let r = self.edit_bindings(|arr| match row.source {
                    Some(Source::User) => {
                        let found = arr.iter().position(|t| entry_matches(t, row));
                        match found.and_then(|i| arr.get_mut(i)) {
                            Some(t) => {
                                t.insert("key", value(key_text(keys)));
                            }
                            None => arr.push(entry(Some(keys), &row.command, &row.when)),
                        }
                    }
                    Some(_) => {
                        // Replace a default: remove it, then add the new keys.
                        arr.push(entry(row.keys.as_ref(), &format!("-{}", row.command), &row.when));
                        arr.push(entry(Some(keys), &row.command, &row.when));
                    }
                    None => arr.push(entry(Some(keys), &row.command, "")),
                });
                (r, format!("{} → {keys}", row.title), row.command.clone())
            }
            KbEdit::Remove(row) => {
                let r = self.edit_bindings(|arr| match row.source {
                    Some(Source::User) => arr.retain(|t| !entry_matches(t, row)),
                    _ => arr.push(entry(row.keys.as_ref(), &format!("-{}", row.command), &row.when)),
                });
                (r, format!("Removed {} from {}", row.keys_text(), row.title), row.command.clone())
            }
            KbEdit::Reset(row) => {
                let undo = format!("-{}", row.command);
                let r = self.edit_bindings(|arr| {
                    arr.retain(|t| {
                        let c = t.get("command").and_then(Item::as_str);
                        c != Some(row.command.as_str()) && c != Some(undo.as_str())
                    })
                });
                (r, format!("Reset {} to its default keys", row.title), row.command.clone())
            }
        };
        if let Err(e) = result {
            self.notify(Severity::Error, format!("Could not save keybindings: {e}"));
            return;
        }
        self.reload_keymap();
        let rows = self.keybinding_rows();
        if let Some(Overlay::Keybindings(e)) = self.overlays.last_mut() {
            e.set_rows(rows, Some(&select));
            e.note = note;
        }
        self.needs_render = true;
    }

    /// Preferences: Restore Default Keybindings — after confirmation, the
    /// user file is moved to `keybindings.toml.bak`.
    pub(crate) fn restore_default_keybindings_confirm(&mut self) {
        let path = self.paths.keybindings();
        if !path.exists() {
            self.notify(Severity::Info, "No user keybindings to remove; the defaults are in use");
            return;
        }
        self.open_overlay(Overlay::Confirm(Confirm {
            title: "Restore Default Keybindings".into(),
            message: vec![
                format!("Move {} to keybindings.toml.bak", path.display()),
                "and use only the default keybindings?".into(),
            ],
            buttons: vec![
                ("Restore Defaults".into(), ConfirmAction::RestoreDefaultKeybindings),
                ("Cancel".into(), ConfirmAction::Nothing),
            ],
            selected: 1,
            cancel: ConfirmAction::Nothing,
        }));
    }

    pub(crate) fn restore_default_keybindings(&mut self) {
        let path = self.paths.keybindings();
        let bak = path.with_extension("toml.bak");
        if let Err(e) = std::fs::rename(&path, &bak) {
            self.notify(Severity::Error, format!("Could not move {}: {e}", path.display()));
            return;
        }
        self.reload_keymap();
        self.notify(Severity::Info, format!("Default keybindings restored; your file is saved as {}", bak.display()));
    }

    /// Preferences: Export Keybindings — the effective keymap as TOML.
    pub(crate) fn export_keybindings(&mut self) {
        let mut text = String::from(
            "# The effective YACE keymap (defaults, plugins and your keybindings.toml),\n\
             # exported by Preferences: Export Keybindings. Later entries win.\n",
        );
        for b in self.keymap.bindings() {
            let src = match b.source {
                Source::Default => "default",
                Source::Plugin => "plugin",
                Source::User => "user",
            };
            text.push_str(&format!(
                "\n[[bindings]] # {src}\nkey = {}\ncommand = {}\n",
                toml_str(&key_text(&b.keys)),
                toml_str(&b.command)
            ));
            let when = b.when.to_string();
            if !when.is_empty() {
                text.push_str(&format!("when = {}\n", toml_str(&when)));
            }
        }
        self.new_untitled_with(&text);
    }
}

fn toml_str(s: &str) -> String {
    toml_edit::Value::from(s).to_string().trim().to_string()
}
