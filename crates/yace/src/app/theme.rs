//! Colour themes in the app (spec §16): loading `workbench.colorTheme` with
//! overrides, the theme picker with live preview (§16.3), and theme
//! commands (create from current, open folder, inspect tokens).

use std::sync::Arc;

use yace_config::ConfigError;

use super::{App, Severity};
use crate::{
    actions::PickerAction,
    overlay::{Overlay, Picker, PickerItem},
    theme::{Catalog, Origin, Overrides, Theme, ThemeKind, detect_depth, load_theme},
};

/// What the current theme was built from.
#[derive(PartialEq)]
pub(crate) struct ThemeKey {
    name: String,
    colors: toml::Table,
    tokens: toml::Table,
    depth: yace_config::ColorDepth,
}

/// The theme picker's state while it is open.
pub(crate) struct ThemePreview {
    /// The theme to restore on Esc.
    saved: Theme,
    /// Theme ids by picker item data.
    ids: Vec<Arc<str>>,
    /// The item currently previewed.
    shown: Option<usize>,
    catalog: Catalog,
}

impl App {
    /// Built-in and user themes (and, from M8, plugin themes).
    pub(crate) fn theme_catalog(&self) -> (Catalog, Vec<ConfigError>) {
        let mut cat = Catalog::builtin();
        let mut errors = Vec::new();
        for f in self.plugin_theme_files() {
            if let Err(e) = cat.add_file(&f) {
                errors.push(e);
            }
        }
        let (user, e) = Catalog::with_user_dir(&self.paths.themes_dir());
        errors.extend(e);
        for entry in user.entries.into_iter().filter(|e| matches!(e.origin, Origin::File(_))) {
            cat.entries.retain(|x| !x.name.eq_ignore_ascii_case(&entry.name) && x.id != entry.id);
            cat.entries.push(entry);
        }
        (cat, errors)
    }

    fn build_theme(&self, cat: &Catalog, name: &str) -> Result<(Theme, Vec<ConfigError>), ConfigError> {
        let ov = Overrides {
            colors: Some(&self.settings.workbench.color_customizations),
            tokens: Some(&self.settings.editor.token_color_customizations),
        };
        let depth = detect_depth(self.settings.workbench.color_depth, self.terminal_depth);
        load_theme(cat, name, &ov).map(|(t, w)| (t.for_depth(depth), w))
    }

    /// Load the theme the settings name. On failure the current theme stays
    /// (spec §32.6) and the problem is returned for reporting. Unchanged
    /// theme settings are not re-parsed (theme files changing on disk call
    /// `invalidate_theme` first).
    pub(crate) fn load_theme_from_settings(&mut self) -> Vec<ConfigError> {
        let wb = &self.settings.workbench;
        let key = ThemeKey {
            name: wb.color_theme.clone(),
            colors: wb.color_customizations.clone(),
            tokens: self.settings.editor.token_color_customizations.clone(),
            depth: wb.color_depth,
        };
        if self.theme_key.as_ref() == Some(&key) {
            return Vec::new();
        }
        self.theme_key = Some(key);
        let (cat, mut errors) = self.theme_catalog();
        let name = self.settings.workbench.color_theme.clone();
        match self.build_theme(&cat, &name) {
            Ok((t, warnings)) => {
                self.theme = t;
                errors.extend(warnings);
            }
            Err(e) => {
                errors.push(e);
                if self.theme.name.is_empty() {
                    // First load: fall back to the default theme.
                    self.theme = Theme::yace_dark()
                        .for_depth(detect_depth(self.settings.workbench.color_depth, self.terminal_depth));
                }
            }
        }
        self.needs_render = true;
        errors
    }

    /// Theme files changed: the next load re-reads them.
    pub(crate) fn invalidate_theme(&mut self) {
        self.theme_key = None;
    }

    /// Preferences: Color Theme (`Ctrl+K Ctrl+T`).
    pub(crate) fn open_theme_picker(&mut self) {
        let (catalog, errors) = self.theme_catalog();
        self.report_config_errors(errors);
        let mut order: Vec<usize> = (0..catalog.entries.len()).collect();
        // Dark, then light; built-ins before user themes; then by name.
        order.sort_by_key(|&i| {
            let e = &catalog.entries[i];
            (e.kind == ThemeKind::Light, matches!(e.origin, Origin::File(_)), e.name.to_lowercase())
        });
        let mut items = Vec::new();
        let mut ids = Vec::new();
        for i in order {
            let e = &catalog.entries[i];
            let kind = if e.kind == ThemeKind::Light { "light" } else { "dark" };
            let detail = match &e.origin {
                Origin::Builtin(_) => format!("{kind} · built-in"),
                Origin::File(p) => format!("{kind} · {}", p.display()),
            };
            items.push(PickerItem::new(e.name.to_string(), detail, "", ids.len()));
            ids.push(e.id.clone());
        }
        let current = ids.iter().position(|id| *id == self.theme.id);
        let mut p =
            Picker::new("Select Color Theme (Up/Down to preview, Esc to cancel)", items, PickerAction::SelectTheme);
        if let Some(c) = current
            && let Some(pos) = p.filtered.iter().position(|&f| p.items[f].data == c)
        {
            p.selected = pos;
        }
        self.theme_preview = Some(ThemePreview { saved: self.theme.clone(), ids, shown: current, catalog });
        self.open_overlay(Overlay::Picker(p));
    }

    /// After input: preview the theme under the picker's selection, or
    /// restore the saved theme if the picker went away without a choice.
    pub(crate) fn sync_theme_preview(&mut self) {
        let Some(preview) = &self.theme_preview else { return };
        let current = self.overlays.iter().rev().find_map(|o| match o {
            Overlay::Picker(p) if p.action == PickerAction::SelectTheme => Some(p.current().map(|i| i.data)),
            _ => None,
        });
        match current {
            None => self.end_theme_preview(None),
            Some(sel) if sel != preview.shown => {
                let Some(id) = sel.and_then(|i| preview.ids.get(i)).cloned() else { return };
                let built = self.build_theme(&preview.catalog, &id);
                if let Some(p) = &mut self.theme_preview {
                    p.shown = sel;
                }
                match built {
                    Ok((t, _)) => self.theme = t,
                    Err(e) => self.notify(Severity::Warning, e.to_string()),
                }
                self.needs_render = true;
            }
            Some(_) => {}
        }
    }

    /// Close the preview: `Some(item)` keeps and saves that theme, `None`
    /// restores the theme from before the picker opened.
    pub(crate) fn end_theme_preview(&mut self, picked: Option<usize>) {
        let Some(preview) = self.theme_preview.take() else { return };
        let Some(id) = picked.and_then(|i| preview.ids.get(i)).cloned() else {
            self.theme = preview.saved;
            self.needs_render = true;
            return;
        };
        let name = preview.catalog.find(&id).map(|e| e.name.clone()).unwrap_or(id);
        self.invalidate_theme();
        match self.build_theme(&preview.catalog, &name) {
            Ok((t, warnings)) => {
                self.theme = t;
                self.report_config_errors(warnings);
            }
            Err(e) => {
                self.theme = preview.saved;
                self.notify(Severity::Error, e.to_string());
                return;
            }
        }
        self.save_theme_setting(&name);
    }

    /// Persist `workbench.colorTheme` in the user settings file, keeping
    /// its comments and layout.
    fn save_theme_setting(&mut self, name: &str) {
        let path = self.paths.user_settings();
        if let Err(e) = yace_config::edit_setting(&path, "workbench", "colorTheme", Some(name.into())) {
            self.notify(Severity::Error, format!("Could not save the theme: {e}"));
            return;
        }
        let theme = self.theme.clone();
        self.reload_settings();
        if self.settings.workbench.color_theme != name {
            // A workspace setting wins over the user setting.
            self.theme = theme;
            let origin = self.workspace.file().map(|p| p.display().to_string()).unwrap_or_default();
            self.notify(
                Severity::Info,
                format!("Saved to user settings; this workspace sets its own theme ({origin}), so it applies to other workspaces"),
            );
        }
    }

    /// Preferences: Create Theme from Current — writes a theme file that
    /// extends the current theme and carries the active overrides.
    pub(crate) fn create_theme_from_current(&mut self) {
        let dir = self.paths.themes_dir();
        let base = self.theme.id.to_string();
        let mut n = 1;
        let path = loop {
            let p = dir.join(if n == 1 { "my-theme.toml".to_string() } else { format!("my-theme-{n}.toml") });
            if !p.exists() {
                break p;
            }
            n += 1;
        };
        let stem = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
        let mut text = format!(
            "# A YACE theme (docs/themes.md). Keys not listed here come from `extends`.\n\
             # Select it with Preferences: Color Theme (Ctrl+K Ctrl+T); changes apply when saved.\n\
             name = \"{}\"\ntype = \"{}\"\nextends = \"{base}\"\n\n[colors]\n",
            stem.replace('-', " "),
            if self.theme.kind == ThemeKind::Light { "light" } else { "dark" },
        );
        let global = |t: &toml::Table| -> Vec<(String, String)> {
            t.iter()
                .filter(|(k, _)| !k.starts_with('['))
                .filter_map(|(k, v)| Some((k.clone(), v.as_str()?.to_string())))
                .collect()
        };
        let colors = global(&self.settings.workbench.color_customizations);
        if colors.is_empty() {
            text.push_str("# \"editor.background\" = \"#1e1f24\"\n");
        }
        for (k, v) in colors {
            text.push_str(&format!("\"{k}\" = \"{v}\"\n"));
        }
        text.push_str("\n[syntax]\n# comment = { fg = \"#6a9955\", italic = true }\n");
        if let Err(e) = yace_config::atomic_write(&path, text.as_bytes()) {
            self.notify(Severity::Error, format!("Could not create {}: {e}", path.display()));
            return;
        }
        self.open_file(&path, false);
    }

    /// Developer: Inspect Editor Tokens — the syntax scope and colour under
    /// the cursor, for theme authors.
    pub(crate) fn inspect_tokens(&mut self) {
        let Some((_, v, d)) = self.active() else { return };
        let head = self.layout.active_group().views[v].sel.primary().head;
        let Some(doc) = self.docs.doc(d) else { return };
        let rope = doc.rope();
        let lang = self.syntax.language(d);
        let byte = rope.char_to_byte(head.min(rope.len_chars()));
        let line = rope.char_to_line(head.min(rope.len_chars()));
        let (a, b) = (rope.line_to_byte(line), rope.line_to_byte((line + 1).min(rope.len_lines())));
        let spans = self.syntax.spans(d, rope, a..b.max(a));
        let scope =
            spans.as_deref().unwrap_or(&[]).iter().rev().find(|s| s.start <= byte && byte < s.end).map(|s| s.scope);
        let (name, color, mods) = match scope {
            Some(s) => (s.name(), self.theme.syntax[s as usize], self.theme.syntax_mods[s as usize]),
            None => ("(plain text)", self.theme.fg, Default::default()),
        };
        let lines = vec![
            format!("Language: {} ({})", lang.name, lang.id),
            format!("Scope: {name}"),
            format!("Colour: {}", crate::theme::color_to_string(color)),
            format!("Style: {mods:?}"),
            format!("Theme: {} ({})", self.theme.name, self.theme.id),
            String::new(),
            "Set it in a theme's [syntax] table or editor.tokenColorCustomizations.".into(),
        ];
        self.open_overlay(Overlay::Info { title: "Editor Token".into(), lines });
    }

    /// Preferences: Import VS Code Color Theme… — themes of installed VS
    /// Code extensions (data only; no extension code runs).
    pub(crate) fn import_vscode_theme(&mut self) {
        let home = directories::BaseDirs::new().map(|b| b.home_dir().to_path_buf()).unwrap_or_default();
        let found = crate::theme::vscode::discover(&crate::theme::vscode::extension_dirs(&home));
        if found.is_empty() {
            self.notify(
                Severity::Info,
                "No VS Code themes found in ~/.vscode/extensions; use \"Import VS Code Color Theme from File…\"",
            );
            return;
        }
        let items = found
            .iter()
            .enumerate()
            .map(|(i, t)| {
                let kind = if matches!(t.ui_theme.as_str(), "vs" | "hc-light") { "light" } else { "dark" };
                PickerItem::new(t.label.clone(), format!("{kind} · {}", t.extension), "", i)
            })
            .collect();
        self.vscode_themes = found;
        self.open_overlay(Overlay::Picker(Picker::new(
            "Import VS Code Color Theme",
            items,
            PickerAction::ImportVsCodeTheme,
        )));
    }

    pub(crate) fn import_vscode_theme_pick(&mut self, i: usize) {
        let Some(t) = self.vscode_themes.get(i).cloned() else { return };
        self.vscode_themes.clear();
        self.import_theme_from(&t.path, &t.label, &t.ui_theme);
    }

    pub(crate) fn import_vscode_theme_file_prompt(&mut self) {
        let initial = self.cwd_with_slash();
        self.prompt(
            "Import VS Code Color Theme",
            "Path of a VS Code theme .json file (Tab completes)",
            initial,
            crate::actions::PromptAction::ImportThemeFile,
            true,
        );
    }

    /// Convert a VS Code theme file, write it to the themes folder and
    /// switch to it.
    pub(crate) fn import_theme_from(&mut self, path: &std::path::Path, label: &str, ui_theme: &str) {
        let label = if label.is_empty() {
            path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "Imported".into())
        } else {
            label.to_string()
        };
        let (text, warnings) = match crate::theme::vscode::convert(path, &label, ui_theme) {
            Ok(r) => r,
            Err(e) => {
                self.notify(Severity::Error, format!("Could not import the theme: {e}"));
                return;
            }
        };
        let out = self.paths.themes_dir().join(format!("{}.toml", crate::theme::vscode::slug(&label)));
        if let Err(e) = yace_config::atomic_write(&out, text.as_bytes()) {
            self.notify(Severity::Error, format!("Could not write {}: {e}", out.display()));
            return;
        }
        self.invalidate_theme();
        let (catalog, errors) = self.theme_catalog();
        self.report_config_errors(errors);
        match self.build_theme(&catalog, &label) {
            Ok((t, w)) => {
                self.theme = t;
                self.report_config_errors(w);
                self.save_theme_setting(&label);
                let note = warnings.first().map(|w| format!(" ({w})")).unwrap_or_default();
                self.notify(Severity::Info, format!("Imported \"{label}\" to {}{note}", out.display()));
            }
            Err(e) => self.notify(Severity::Error, e.to_string()),
        }
    }

    /// Preferences: Edit User Theme… — open a theme file from the themes
    /// folder, or create one.
    pub(crate) fn edit_user_theme(&mut self) {
        let (catalog, _) = self.theme_catalog();
        let mut items =
            vec![PickerItem::new("New Theme from Current…", "creates a file that extends the current theme", "", 0)];
        self.user_theme_files = catalog
            .entries
            .iter()
            .filter_map(|e| match &e.origin {
                Origin::File(p) => Some((e.name.to_string(), p.clone())),
                Origin::Builtin(_) => None,
            })
            .collect();
        for (i, (name, p)) in self.user_theme_files.iter().enumerate() {
            items.push(PickerItem::new(name.clone(), p.display().to_string(), "", i + 1));
        }
        self.open_overlay(Overlay::Picker(Picker::new("Edit User Theme", items, PickerAction::EditUserTheme)));
    }

    pub(crate) fn edit_user_theme_pick(&mut self, i: usize) {
        if i == 0 {
            self.create_theme_from_current();
        } else if let Some((_, p)) = self.user_theme_files.get(i - 1).cloned() {
            self.open_file(&p, false);
        }
    }
}
