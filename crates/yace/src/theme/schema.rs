//! Theme files (spec §16.2, §16.4): parsing, inheritance (`extends`) and
//! user overrides. UI keys are VS Code colour ids, so imported VS Code
//! themes and `workbench.colorCustomizations` use the same table.

use std::{
    collections::HashSet,
    path::{Path, PathBuf},
    sync::Arc,
};

use ratatui::style::{Color, Modifier};
use toml::{Table, Value};
use yace_config::ConfigError;
use yace_syntax::Scope;

use super::{Theme, ThemeKind};

pub struct ColorKey {
    /// VS Code colour id (or `yace.*` where VS Code has none).
    pub id: &'static str,
    pub get: fn(&mut Theme) -> &mut Color,
    /// One-line description for docs/themes.md.
    pub doc: &'static str,
}

macro_rules! keys {
    ($( $id:literal => $($field:ident)+ $([$i:literal])? , $doc:literal; )*) => {
        &[ $( ColorKey { id: $id, get: |t| &mut t.$($field)+ $([$i])?, doc: $doc } ),* ]
    };
}

/// Every UI colour a theme can set, in documentation order.
pub static COLOR_KEYS: &[ColorKey] = keys! {
    "editor.background" => bg, "Editor and main background";
    "editor.foreground" => fg, "Default text";
    "descriptionForeground" => muted, "Secondary text (details, hints, placeholders)";
    "sideBar.background" => sidebar_bg, "Explorer and search sidebar";
    "sideBar.foreground" => sidebar_fg, "Sidebar text";
    "sideBarTitle.foreground" => sidebar_title, "Sidebar section titles";
    "tab.activeBackground" => tab_active_bg, "Active tab";
    "tab.activeForeground" => tab_active_fg, "Active tab text";
    "tab.inactiveBackground" => tab_inactive_bg, "Inactive tabs";
    "tab.inactiveForeground" => tab_inactive_fg, "Inactive tab text";
    "editorGroupHeader.tabsBackground" => tab_bar_bg, "Tab bar";
    "editorGroup.border" => border, "Borders between parts";
    "focusBorder" => focus_border, "Focused part border and titles";
    "editor.selectionBackground" => selection_bg, "Selection";
    "editor.inactiveSelectionBackground" => selection_inactive_bg, "Selection in an unfocused editor";
    "editor.lineHighlightBackground" => cursor_line_bg, "Current line";
    "editorLineNumber.foreground" => line_number, "Line numbers";
    "editorLineNumber.activeForeground" => line_number_active, "Current line number";
    "editorCursor.background" => cursor_fg, "Character under a drawn block cursor";
    "editorCursor.foreground" => cursor_bg, "Drawn cursor (secondary cursors)";
    "editorBracketMatch.background" => bracket_match_bg, "Matching bracket";
    "statusBar.background" => status_bg, "Status bar";
    "statusBar.foreground" => status_fg, "Status bar text";
    "statusBar.noFolderBackground" => status_no_folder_bg, "Status bar with no folder open";
    "list.activeSelectionBackground" => list_active_bg, "Selected row in a focused list";
    "list.activeSelectionForeground" => list_active_fg, "Selected row text";
    "list.inactiveSelectionBackground" => list_inactive_selection_bg, "Selected row in an unfocused list";
    "list.highlightForeground" => list_hover_match, "Matched characters in lists";
    "editorWidget.background" => overlay_bg, "Pickers, prompts and popups";
    "editorWidget.foreground" => overlay_fg, "Popup text";
    "editorWidget.border" => overlay_border, "Popup border";
    "quickInputList.focusBackground" => overlay_selected_bg, "Selected row in a picker";
    "input.background" => input_bg, "Input fields";
    "editorError.foreground" => error, "Errors";
    "editorWarning.foreground" => warning, "Warnings";
    "editorInfo.foreground" => info, "Information";
    "gitDecoration.modifiedResourceForeground" => git_modified, "Git: modified";
    "gitDecoration.addedResourceForeground" => git_added, "Git: added";
    "gitDecoration.deletedResourceForeground" => git_deleted, "Git: deleted";
    "yace.dirtyForeground" => dirty, "Unsaved-changes marker";
    "yace.missingRootForeground" => missing_root, "Unavailable workspace folder";
    "scrollbarSlider.background" => scrollbar, "Scrollbar thumb";
    "editorWhitespace.foreground" => whitespace, "Rendered whitespace";
    "editor.findMatchHighlightBackground" => find_match_bg, "Find matches";
    "editor.findMatchBackground" => find_current_bg, "Current find match";
    "editorSuggestWidget.highlightForeground" => match_fg, "Matched characters in completion and pickers";
    "terminal.background" => terminal_bg, "Integrated terminal";
    "terminal.foreground" => terminal_fg, "Terminal text";
    "terminal.ansiBlack" => terminal_ansi[0], "Terminal ANSI 0";
    "terminal.ansiRed" => terminal_ansi[1], "Terminal ANSI 1";
    "terminal.ansiGreen" => terminal_ansi[2], "Terminal ANSI 2";
    "terminal.ansiYellow" => terminal_ansi[3], "Terminal ANSI 3";
    "terminal.ansiBlue" => terminal_ansi[4], "Terminal ANSI 4";
    "terminal.ansiMagenta" => terminal_ansi[5], "Terminal ANSI 5";
    "terminal.ansiCyan" => terminal_ansi[6], "Terminal ANSI 6";
    "terminal.ansiWhite" => terminal_ansi[7], "Terminal ANSI 7";
    "terminal.ansiBrightBlack" => terminal_ansi[8], "Terminal ANSI 8";
    "terminal.ansiBrightRed" => terminal_ansi[9], "Terminal ANSI 9";
    "terminal.ansiBrightGreen" => terminal_ansi[10], "Terminal ANSI 10";
    "terminal.ansiBrightYellow" => terminal_ansi[11], "Terminal ANSI 11";
    "terminal.ansiBrightBlue" => terminal_ansi[12], "Terminal ANSI 12";
    "terminal.ansiBrightMagenta" => terminal_ansi[13], "Terminal ANSI 13";
    "terminal.ansiBrightCyan" => terminal_ansi[14], "Terminal ANSI 14";
    "terminal.ansiBrightWhite" => terminal_ansi[15], "Terminal ANSI 15";
};

/// Other VS Code ids accepted for a key (lower priority than the key
/// itself), so imported themes and overrides written for VS Code work.
pub static ALIASES: &[(&str, &str)] = &[
    ("quickInput.background", "editorWidget.background"),
    ("quickInput.foreground", "editorWidget.foreground"),
    ("editorSuggestWidget.background", "editorWidget.background"),
    ("editorSuggestWidget.border", "editorWidget.border"),
    ("widget.border", "editorWidget.border"),
    ("list.focusBackground", "quickInputList.focusBackground"),
    ("editorSuggestWidget.selectedBackground", "quickInputList.focusBackground"),
    ("panel.border", "editorGroup.border"),
    ("sideBar.border", "editorGroup.border"),
    ("editorGroupHeader.tabsBorder", "editorGroup.border"),
    ("sideBarSectionHeader.foreground", "sideBarTitle.foreground"),
    ("foreground", "editor.foreground"),
    ("errorForeground", "editorError.foreground"),
    ("editorOverviewRuler.errorForeground", "editorError.foreground"),
    ("list.errorForeground", "editorError.foreground"),
    ("list.warningForeground", "editorWarning.foreground"),
    ("gitDecoration.untrackedResourceForeground", "gitDecoration.addedResourceForeground"),
    ("editorGutter.modifiedBackground", "gitDecoration.modifiedResourceForeground"),
    ("editorGutter.addedBackground", "gitDecoration.addedResourceForeground"),
    ("editorGutter.deletedBackground", "gitDecoration.deletedResourceForeground"),
    ("tab.unfocusedActiveBackground", "tab.activeBackground"),
    ("panel.background", "terminal.background"),
    ("editor.selectionHighlightBackground", "editor.inactiveSelectionBackground"),
    ("editor.wordHighlightBackground", "editorBracketMatch.background"),
];

/// When a layer sets the source but not the target, the target follows it,
/// so a theme that only changes the background still looks consistent.
static DERIVED: &[(&str, &str)] = &[
    ("tab.activeBackground", "editor.background"),
    ("editorCursor.background", "editor.background"),
    ("terminal.background", "editor.background"),
    ("terminal.foreground", "editor.foreground"),
    ("editorWidget.foreground", "editor.foreground"),
    ("sideBar.foreground", "editor.foreground"),
    ("editorLineNumber.activeForeground", "editor.foreground"),
    ("quickInputList.focusBackground", "list.activeSelectionBackground"),
];

fn key_index(id: &str) -> Option<usize> {
    COLOR_KEYS.iter().position(|k| k.id == id)
}

/// Parse `#rgb`, `#rrggbb`, `#rrggbbaa` (alpha blended over `under`), an
/// ANSI colour name (`red`, `bright-blue`), `ansi:N` (0–255) or `default`.
pub fn parse_color(s: &str, under: Color) -> Result<Color, String> {
    let s = s.trim();
    if let Some(hex) = s.strip_prefix('#') {
        let digit = |i: usize| u8::from_str_radix(&hex[i..i + 1], 16);
        let byte = |i: usize| u8::from_str_radix(&hex[i..i + 2], 16);
        let bad = || format!("invalid colour `{s}`");
        if !hex.is_ascii() {
            return Err(bad());
        }
        let (r, g, b, a) = match hex.len() {
            3 | 4 => {
                let v = |i| digit(i).map(|d| d * 17).map_err(|_| bad());
                (v(0)?, v(1)?, v(2)?, if hex.len() == 4 { v(3)? } else { 255 })
            }
            6 | 8 => {
                let v = |i| byte(i).map_err(|_| bad());
                (v(0)?, v(2)?, v(4)?, if hex.len() == 8 { v(6)? } else { 255 })
            }
            _ => return Err(bad()),
        };
        if a == 255 {
            return Ok(Color::Rgb(r, g, b));
        }
        let (ur, ug, ub) = match under {
            Color::Rgb(r, g, b) => (r, g, b),
            _ => (0, 0, 0),
        };
        let mix =
            |c: u8, u: u8| ((u16::from(c) * u16::from(a) + u16::from(u) * (255 - u16::from(a)) + 127) / 255) as u8;
        return Ok(Color::Rgb(mix(r, ur), mix(g, ug), mix(b, ub)));
    }
    if let Some(n) = s.strip_prefix("ansi:") {
        return n.parse::<u8>().map(Color::Indexed).map_err(|_| format!("invalid colour `{s}` (ansi:0–255)"));
    }
    Ok(match s.to_ascii_lowercase().replace('_', "-").as_str() {
        "default" | "reset" | "none" => Color::Reset,
        "black" => Color::Black,
        "red" => Color::Red,
        "green" => Color::Green,
        "yellow" => Color::Yellow,
        "blue" => Color::Blue,
        "magenta" => Color::Magenta,
        "cyan" => Color::Cyan,
        "white" | "gray" | "grey" => Color::Gray,
        "bright-black" | "dark-gray" | "dark-grey" => Color::DarkGray,
        "bright-red" => Color::LightRed,
        "bright-green" => Color::LightGreen,
        "bright-yellow" => Color::LightYellow,
        "bright-blue" => Color::LightBlue,
        "bright-magenta" => Color::LightMagenta,
        "bright-cyan" => Color::LightCyan,
        "bright-white" => Color::White,
        _ => return Err(format!("invalid colour `{s}`")),
    })
}

/// Format a colour for writing a theme file.
pub fn color_to_string(c: Color) -> String {
    match c {
        Color::Rgb(r, g, b) => format!("#{r:02x}{g:02x}{b:02x}"),
        Color::Indexed(i) => format!("ansi:{i}"),
        Color::Reset => "default".into(),
        Color::Black => "black".into(),
        Color::Red => "red".into(),
        Color::Green => "green".into(),
        Color::Yellow => "yellow".into(),
        Color::Blue => "blue".into(),
        Color::Magenta => "magenta".into(),
        Color::Cyan => "cyan".into(),
        Color::Gray => "white".into(),
        Color::DarkGray => "bright-black".into(),
        Color::LightRed => "bright-red".into(),
        Color::LightGreen => "bright-green".into(),
        Color::LightYellow => "bright-yellow".into(),
        Color::LightBlue => "bright-blue".into(),
        Color::LightMagenta => "bright-magenta".into(),
        Color::LightCyan => "bright-cyan".into(),
        Color::White => "bright-white".into(),
    }
}

/// Where a theme comes from.
#[derive(Clone, Debug, PartialEq)]
pub enum Origin {
    Builtin(&'static str),
    File(PathBuf),
}

#[derive(Clone, Debug, PartialEq)]
pub struct ThemeSource {
    pub id: Arc<str>,
    pub name: Arc<str>,
    pub kind: ThemeKind,
    pub origin: Origin,
}

impl ThemeSource {
    fn label(&self) -> String {
        match &self.origin {
            Origin::Builtin(_) => format!("built-in theme {}", self.id),
            Origin::File(p) => p.display().to_string(),
        }
    }
    fn text(&self) -> Result<std::borrow::Cow<'static, str>, ConfigError> {
        match &self.origin {
            Origin::Builtin(t) => Ok((*t).into()),
            Origin::File(p) => std::fs::read_to_string(p).map(Into::into).map_err(|e| ConfigError {
                origin: self.label(),
                key: None,
                message: e.to_string(),
            }),
        }
    }
}

/// Built-in themes: (id, name, kind, file). Names and kinds are listed here
/// so startup never parses a theme it doesn't use.
pub static BUILTIN: &[(&str, &str, ThemeKind, &str)] = &[
    ("yace-dark", "YACE Dark", ThemeKind::Dark, include_str!("../../themes/yace-dark.toml")),
    ("yace-light", "YACE Light", ThemeKind::Light, include_str!("../../themes/yace-light.toml")),
    ("one-dark", "One Dark", ThemeKind::Dark, include_str!("../../themes/one-dark.toml")),
    ("dracula", "Dracula", ThemeKind::Dark, include_str!("../../themes/dracula.toml")),
    ("catppuccin-mocha", "Catppuccin Mocha", ThemeKind::Dark, include_str!("../../themes/catppuccin-mocha.toml")),
    ("tokyo-night", "Tokyo Night", ThemeKind::Dark, include_str!("../../themes/tokyo-night.toml")),
];

/// The themes available by name or id: built-ins, then user themes (and,
/// from M8, plugin themes).
#[derive(Clone, Debug, Default)]
pub struct Catalog {
    pub entries: Vec<ThemeSource>,
}

impl Catalog {
    pub fn builtin() -> Self {
        let entries = BUILTIN
            .iter()
            .map(|(id, name, kind, text)| ThemeSource {
                id: Arc::from(*id),
                name: Arc::from(*name),
                kind: *kind,
                origin: Origin::Builtin(text),
            })
            .collect();
        Self { entries }
    }

    /// Built-ins plus every `*.toml` in `dir` (only `name`/`type` are read
    /// here; the rest is parsed when the theme is used).
    pub fn with_user_dir(dir: &Path) -> (Self, Vec<ConfigError>) {
        let mut cat = Self::builtin();
        let mut errors = Vec::new();
        let Ok(rd) = std::fs::read_dir(dir) else { return (cat, errors) };
        let mut files: Vec<PathBuf> =
            rd.flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|e| e == "toml")).collect();
        files.sort();
        for path in files {
            match cat.add_file(&path) {
                Ok(()) => {}
                Err(e) => errors.push(e),
            }
        }
        (cat, errors)
    }

    /// Add one theme file (a user theme or a plugin's). A later theme with
    /// the same name replaces an earlier one.
    pub fn add_file(&mut self, path: &Path) -> Result<(), ConfigError> {
        let origin = path.display().to_string();
        let text = std::fs::read_to_string(path).map_err(|e| ConfigError {
            origin: origin.clone(),
            key: None,
            message: e.to_string(),
        })?;
        let table = parse_table(&text, &origin)?;
        let id: Arc<str> = Arc::from(path.file_stem().and_then(|s| s.to_str()).unwrap_or("theme"));
        let name: Arc<str> = table.get("name").and_then(Value::as_str).map(Arc::from).unwrap_or_else(|| id.clone());
        let kind = match table.get("type").and_then(Value::as_str) {
            Some("light") => ThemeKind::Light,
            _ => ThemeKind::Dark,
        };
        self.entries.retain(|e| !e.name.eq_ignore_ascii_case(&name) && e.id != id);
        self.entries.push(ThemeSource { id, name, kind, origin: Origin::File(path.to_path_buf()) });
        Ok(())
    }

    /// By display name (case-insensitive) or id.
    pub fn find(&self, name: &str) -> Option<&ThemeSource> {
        let name = name.trim();
        self.entries.iter().rev().find(|e| e.name.eq_ignore_ascii_case(name) || &*e.id == name)
    }
}

fn parse_table(text: &str, origin: &str) -> Result<Table, ConfigError> {
    text.parse::<Table>().map_err(|e| {
        let line = e.span().map(|s| text[..s.start.min(text.len())].lines().count().max(1));
        let msg = e.message().trim().to_string();
        ConfigError {
            origin: origin.to_string(),
            key: None,
            message: match line {
                Some(l) => format!("line {l}: {msg}"),
                None => msg,
            },
        }
    })
}

/// User overrides from settings (`workbench.colorCustomizations`,
/// `editor.tokenColorCustomizations`).
#[derive(Clone, Copy, Debug, Default)]
pub struct Overrides<'a> {
    pub colors: Option<&'a Table>,
    pub tokens: Option<&'a Table>,
}

/// Load a theme by name or id with inheritance and overrides. Problems that
/// don't stop the theme loading (unknown keys, bad values) come back as
/// warnings; an unreadable or cyclic theme is an error.
pub fn load_theme(cat: &Catalog, name: &str, ov: &Overrides) -> Result<(Theme, Vec<ConfigError>), ConfigError> {
    load_theme_with(cat, name, ov, true)
}

/// `compiled`: use the code generated from the built-in theme files
/// (`builtin_gen.rs`) instead of parsing them; only the test that checks
/// that code parses them.
fn load_theme_with(
    cat: &Catalog,
    name: &str,
    ov: &Overrides,
    compiled: bool,
) -> Result<(Theme, Vec<ConfigError>), ConfigError> {
    let src = cat.find(name).ok_or_else(|| ConfigError {
        origin: "workbench.colorTheme".into(),
        key: None,
        message: format!("unknown theme `{name}`"),
    })?;
    let mut warnings = Vec::new();
    let mut visited = HashSet::new();
    let mut theme = build(cat, src, &mut visited, &mut warnings, compiled)?;
    let origin = "settings";
    if let Some(colors) = ov.colors {
        apply_overrides(&mut theme, colors, origin, &mut warnings, apply_colors);
    }
    if let Some(tokens) = ov.tokens {
        apply_overrides(&mut theme, tokens, origin, &mut warnings, apply_token_overrides);
    }
    Ok((theme, warnings))
}

fn apply_overrides(
    theme: &mut Theme,
    table: &Table,
    origin: &str,
    warnings: &mut Vec<ConfigError>,
    apply: impl Fn(&mut Theme, &Table, &str, &mut Vec<ConfigError>),
) {
    let global: Table =
        table.iter().filter(|(k, _)| !k.starts_with('[')).map(|(k, v)| (k.clone(), v.clone())).collect();
    apply(theme, &global, origin, warnings);
    for (k, v) in table {
        let Some(names) = k.strip_prefix('[').and_then(|k| k.strip_suffix(']')) else { continue };
        // VS Code allows "[A][B]" for several themes.
        let matches = names.split("][").any(|n| n.eq_ignore_ascii_case(&theme.name) || n == &*theme.id);
        if matches && let Value::Table(t) = v {
            apply(theme, t, origin, warnings);
        }
    }
}

const MAX_DEPTH: usize = 16;

fn build(
    cat: &Catalog,
    src: &ThemeSource,
    visited: &mut HashSet<Arc<str>>,
    warnings: &mut Vec<ConfigError>,
    compiled: bool,
) -> Result<Theme, ConfigError> {
    let origin = src.label();
    if !visited.insert(src.id.clone()) || visited.len() > MAX_DEPTH {
        return Err(ConfigError {
            origin,
            key: Some("extends".into()),
            message: "themes extend each other in a cycle".into(),
        });
    }
    if compiled
        && matches!(src.origin, Origin::Builtin(_))
        && let Some((_, make)) = super::builtin_gen::COMPILED.iter().find(|(id, _)| *id == &*src.id)
    {
        return Ok(make());
    }
    let text = src.text()?;
    let table = parse_table(&text, &origin)?;
    let kind = match table.get("type").and_then(Value::as_str) {
        Some("light") => ThemeKind::Light,
        Some("dark") | None => src.kind,
        Some(other) => {
            warnings.push(ConfigError {
                origin: origin.clone(),
                key: Some("type".into()),
                message: format!("expected \"dark\" or \"light\", got \"{other}\""),
            });
            src.kind
        }
    };
    let parent_name = match table.get("extends").and_then(Value::as_str) {
        Some(p) => Some(p),
        None if matches!(&*src.id, "yace-dark" | "yace-light") => None,
        None if kind == ThemeKind::Light => Some("yace-light"),
        None => Some("yace-dark"),
    };
    let mut theme = match parent_name {
        Some(p) => {
            let parent = cat.find(p).ok_or_else(|| ConfigError {
                origin: origin.clone(),
                key: Some("extends".into()),
                message: format!("unknown theme `{p}`"),
            })?;
            build(cat, parent, visited, warnings, compiled)?
        }
        None => Theme::blank(kind),
    };
    theme.kind = kind;
    theme.id = src.id.clone();
    theme.name = table.get("name").and_then(Value::as_str).map(Arc::from).unwrap_or_else(|| src.name.clone());
    for (k, v) in &table {
        match (k.as_str(), v) {
            ("name" | "type" | "extends", _) => {}
            ("colors", Value::Table(t)) => apply_colors(&mut theme, t, &origin, warnings),
            ("syntax", Value::Table(t)) => apply_syntax(&mut theme, t, &origin, warnings),
            ("semantic", Value::Table(t)) => apply_semantic(&mut theme, t, &origin, warnings),
            _ => warnings.push(ConfigError {
                origin: origin.clone(),
                key: Some(k.clone()),
                message: "unknown key".into(),
            }),
        }
    }
    Ok(theme)
}

fn warn(warnings: &mut Vec<ConfigError>, origin: &str, key: &str, message: impl Into<String>) {
    warnings.push(ConfigError { origin: origin.to_string(), key: Some(key.to_string()), message: message.into() });
}

/// Apply a `[colors]` table: VS Code ids (and aliases) → tokens.
pub(crate) fn apply_colors(theme: &mut Theme, t: &Table, origin: &str, warnings: &mut Vec<ConfigError>) {
    let mut set = vec![false; COLOR_KEYS.len()];
    let assign =
        |theme: &mut Theme, idx: usize, key: &str, v: &Value, set: &mut Vec<bool>, warnings: &mut Vec<ConfigError>| {
            let Some(s) = v.as_str() else {
                warn(warnings, origin, key, "expected a colour string");
                return;
            };
            let under = if idx == 0 { Color::Reset } else { theme.bg };
            match parse_color(s, under) {
                Ok(c) => {
                    *(COLOR_KEYS[idx].get)(theme) = c;
                    set[idx] = true;
                }
                Err(e) => warn(warnings, origin, key, e),
            }
        };
    // The background first: other colours with alpha blend over it.
    if let Some(v) = t.get("editor.background") {
        assign(theme, 0, "editor.background", v, &mut set, warnings);
    }
    for (alias, canon) in ALIASES {
        if let Some(v) = t.get(*alias)
            && !t.contains_key(*canon)
            && let Some(i) = key_index(canon)
            && !set[i]
        {
            assign(theme, i, alias, v, &mut set, warnings);
        }
    }
    for (k, v) in t {
        if k == "editor.background" {
            continue;
        }
        match key_index(k) {
            Some(i) => assign(theme, i, k, v, &mut set, warnings),
            None if ALIASES.iter().any(|(a, _)| a == k) => {}
            None => warn(warnings, origin, k, "unknown colour key (see docs/themes.md)"),
        }
    }
    for (target, source) in DERIVED {
        if let (Some(ti), Some(si)) = (key_index(target), key_index(source))
            && set[si]
            && !set[ti]
        {
            let c = *(COLOR_KEYS[si].get)(theme);
            *(COLOR_KEYS[ti].get)(theme) = c;
        }
    }
    // `none` text follows the editor foreground unless set explicitly.
    if set[1] {
        theme.syntax[Scope::None as usize] = theme.fg;
    }
}

/// A syntax entry: `"#rrggbb"` or `{ fg, bold, italic, underline }`.
fn syn_entry(theme: &Theme, v: &Value) -> Result<(Option<Color>, Option<Modifier>, Modifier), String> {
    match v {
        Value::String(s) => Ok((Some(parse_color(s, theme.bg)?), None, Modifier::empty())),
        Value::Table(t) => {
            let fg = match t.get("fg").or_else(|| t.get("foreground")) {
                Some(Value::String(s)) => Some(parse_color(s, theme.bg)?),
                Some(_) => return Err("`fg` must be a colour string".into()),
                None => None,
            };
            let mut on = Modifier::empty();
            let mut off = Modifier::empty();
            for (name, m) in
                [("bold", Modifier::BOLD), ("italic", Modifier::ITALIC), ("underline", Modifier::UNDERLINED)]
            {
                match t.get(name) {
                    Some(Value::Boolean(true)) => on |= m,
                    Some(Value::Boolean(false)) => off |= m,
                    Some(_) => return Err(format!("`{name}` must be true or false")),
                    None => {}
                }
            }
            for k in t.keys() {
                if !matches!(k.as_str(), "fg" | "foreground" | "bold" | "italic" | "underline") {
                    return Err(format!("unknown style key `{k}`"));
                }
            }
            Ok((fg, Some(on), off))
        }
        _ => Err("expected a colour or a style table".into()),
    }
}

fn set_scope(theme: &mut Theme, scope: Scope, fg: Option<Color>, on: Option<Modifier>, off: Modifier) {
    let i = scope as usize;
    if let Some(c) = fg {
        theme.syntax[i] = c;
    }
    if let Some(on) = on {
        theme.syntax_mods[i] = (theme.syntax_mods[i] | on) - off;
    }
}

fn scope_for(key: &str) -> Option<Scope> {
    match Scope::from_capture(key) {
        Scope::None if key != "none" => None,
        s => Some(s),
    }
}

pub(crate) fn apply_syntax(theme: &mut Theme, t: &Table, origin: &str, warnings: &mut Vec<ConfigError>) {
    for (k, v) in t {
        let Some(scope) = scope_for(k) else {
            warn(warnings, origin, k, "unknown syntax scope (see docs/themes.md)");
            continue;
        };
        match syn_entry(theme, v) {
            Ok((fg, on, off)) => set_scope(theme, scope, fg, on, off),
            Err(e) => warn(warnings, origin, k, e),
        }
    }
}

fn apply_semantic(theme: &mut Theme, t: &Table, origin: &str, warnings: &mut Vec<ConfigError>) {
    for (k, v) in t {
        match syn_entry(theme, v) {
            Ok((fg, on, _)) => {
                theme.semantic.retain(|(n, _, _)| &**n != k);
                theme.semantic.push((k.as_str().into(), fg.unwrap_or(theme.fg), on.unwrap_or_default()));
            }
            Err(e) => warn(warnings, origin, k, e),
        }
    }
}

/// VS Code's simple `editor.tokenColorCustomizations` keys.
const SIMPLE_TOKEN_KEYS: &[(&str, &[Scope])] = &[
    ("comments", &[Scope::Comment]),
    ("strings", &[Scope::String, Scope::Character]),
    ("keywords", &[Scope::Keyword, Scope::KeywordControl]),
    ("numbers", &[Scope::Number]),
    ("types", &[Scope::Type, Scope::TypeBuiltin]),
    ("functions", &[Scope::Function, Scope::FunctionBuiltin, Scope::FunctionMethod]),
    ("variables", &[Scope::Variable, Scope::VariableParameter]),
];

fn apply_token_overrides(theme: &mut Theme, t: &Table, origin: &str, warnings: &mut Vec<ConfigError>) {
    for (k, v) in t {
        if k == "textMateRules" {
            if let Value::Array(rules) = v {
                super::vscode::apply_textmate_rules(theme, rules);
            }
            continue;
        }
        if k == "semanticHighlighting" {
            continue;
        }
        // VS Code's shape: "#rrggbb" or { foreground, fontStyle }.
        let v = match v {
            Value::Table(tt) if tt.contains_key("fontStyle") || tt.contains_key("foreground") => {
                let mut out = Table::new();
                if let Some(fg) = tt.get("foreground") {
                    out.insert("fg".into(), fg.clone());
                }
                if let Some(Value::String(fs)) = tt.get("fontStyle") {
                    for m in ["bold", "italic", "underline"] {
                        out.insert(m.into(), Value::Boolean(fs.contains(m)));
                    }
                }
                Value::Table(out)
            }
            other => other.clone(),
        };
        let scopes: Vec<Scope> = match SIMPLE_TOKEN_KEYS.iter().find(|(n, _)| n == k) {
            Some((_, s)) => s.to_vec(),
            None => match scope_for(k) {
                Some(s) => vec![s],
                None => {
                    warn(warnings, origin, &format!("editor.tokenColorCustomizations.{k}"), "unknown token key");
                    continue;
                }
            },
        };
        match syn_entry(theme, &v) {
            Ok((fg, on, off)) => {
                for s in scopes {
                    set_scope(theme, s, fg, on, off);
                }
            }
            Err(e) => warn(warnings, origin, &format!("editor.tokenColorCustomizations.{k}"), e),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colours_parse() {
        assert_eq!(parse_color("#fff", Color::Reset), Ok(Color::Rgb(255, 255, 255)));
        assert_eq!(parse_color("#102030", Color::Reset), Ok(Color::Rgb(0x10, 0x20, 0x30)));
        // 50% white over black.
        assert_eq!(parse_color("#ffffff80", Color::Rgb(0, 0, 0)), Ok(Color::Rgb(128, 128, 128)));
        assert_eq!(parse_color("#f008", Color::Rgb(0, 0, 0)), Ok(Color::Rgb(136, 0, 0)));
        assert_eq!(parse_color("ansi:3", Color::Reset), Ok(Color::Indexed(3)));
        assert_eq!(parse_color("bright-blue", Color::Reset), Ok(Color::LightBlue));
        assert_eq!(parse_color("default", Color::Reset), Ok(Color::Reset));
        assert!(parse_color("#12", Color::Reset).is_err());
        assert!(parse_color("#gg0000", Color::Reset).is_err());
        assert!(parse_color("ansi:300", Color::Reset).is_err());
        assert!(parse_color("#ééé", Color::Reset).is_err());
        for c in [Color::Rgb(1, 2, 3), Color::Indexed(9), Color::LightCyan, Color::Reset, Color::Gray] {
            assert_eq!(parse_color(&color_to_string(c), Color::Reset), Ok(c));
        }
    }

    #[test]
    fn every_builtin_loads_cleanly_and_sets_every_key() {
        let cat = Catalog::builtin();
        for (id, name, kind, _) in BUILTIN {
            let (t, warnings) = load_theme(&cat, id, &Overrides::default()).unwrap();
            assert!(warnings.is_empty(), "{id}: {warnings:?}");
            assert_eq!(&*t.name, *name);
            assert_eq!(t.kind, *kind, "{id}");
            let mut t = t;
            for k in COLOR_KEYS {
                assert_ne!(*(k.get)(&mut t), Color::Reset, "{id}: {} unset", k.id);
            }
            for s in Scope::ALL {
                assert_ne!(t.syntax[s as usize], Color::Reset, "{id}: {} unset", s.name());
            }
        }
    }

    /// Built-in themes are compiled from their TOML files (no parsing at
    /// startup). This test regenerates that code and fails if it is stale:
    /// run `YACE_REGENERATE=1 cargo test -p yace --lib compiled_builtins` to
    /// update it after editing a theme file.
    #[test]
    fn compiled_builtins_match_toml() {
        let cat = Catalog::builtin();
        let mut out = String::from(
            "//! @generated from `crates/yace/themes/*.toml` by the test\n\
             //! `theme::schema::tests::compiled_builtins_match_toml`; do not edit.\n\
             //! Regenerate: `YACE_REGENERATE=1 cargo test -p yace --lib compiled_builtins`.\n\n\
             use std::sync::Arc;\n\n\
             use ratatui::style::{Color, Modifier};\n\n\
             use super::{Theme, ThemeKind, schema::COLOR_KEYS};\n\n\
             /// Builds one built-in theme.\n\
             pub type MakeTheme = fn() -> Theme;\n\n\
             pub static COMPILED: &[(&str, MakeTheme)] = &[\n",
        );
        for (id, ..) in BUILTIN {
            out.push_str(&format!("    (\"{id}\", {}),\n", id.replace('-', "_")));
        }
        out.push_str("];\n");
        for (id, ..) in BUILTIN {
            let (mut t, w) = load_theme_with(&cat, id, &Overrides::default(), false).unwrap();
            assert!(w.is_empty());
            assert!(t.semantic.is_empty(), "built-in [semantic] tables are not compiled yet");
            let colors: Vec<String> = COLOR_KEYS.iter().map(|k| format!("Color::{:?}", *(k.get)(&mut t))).collect();
            let syntax: Vec<String> = t.syntax.iter().map(|c| format!("Color::{c:?}")).collect();
            let mods: Vec<String> =
                t.syntax_mods.iter().map(|m| format!("Modifier::from_bits_retain({})", m.bits())).collect();
            out.push_str(&format!(
                "\nfn {fname}() -> Theme {{\n    let mut t = Theme::blank(ThemeKind::{kind:?});\n    \
                 t.name = Arc::from(\"{name}\");\n    t.id = Arc::from(\"{id}\");\n    \
                 let colors = [\n        {colors}\n    ];\n    \
                 for (k, c) in COLOR_KEYS.iter().zip(colors) {{\n        *(k.get)(&mut t) = c;\n    }}\n    \
                 t.syntax = [\n        {syntax}\n    ];\n    \
                 t.syntax_mods = [\n        {mods}\n    ];\n    t\n}}\n",
                fname = id.replace('-', "_"),
                kind = t.kind,
                name = t.name,
                colors = colors.join(",\n        "),
                syntax = syntax.join(",\n        "),
                mods = mods.join(",\n        "),
            ));
            // And the compiled code gives the same theme.
            let compiled = load_theme(&cat, id, &Overrides::default()).unwrap().0;
            assert_eq!(format!("{compiled:?}"), format!("{t:?}"), "{id}: builtin_gen.rs is stale (see above)");
        }
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/src/theme/builtin_gen.rs");
        let current = std::fs::read_to_string(path).unwrap_or_default();
        // Compared without whitespace: rustfmt may reformat the file.
        let squash = |s: &str| s.split_whitespace().collect::<String>().replace(",]", "]").replace(",)", ")");
        if std::env::var_os("YACE_REGENERATE").is_some() {
            std::fs::write(path, &out).unwrap();
        } else {
            assert!(
                squash(&current) == squash(&out),
                "builtin_gen.rs is stale: run YACE_REGENERATE=1 cargo test -p yace --lib compiled_builtins"
            );
        }
    }

    #[test]
    fn docs_list_every_key_and_scope() {
        let docs = include_str!("../../../../docs/themes.md");
        for k in COLOR_KEYS {
            assert!(docs.contains(&format!("| `{}` | {} |", k.id, k.doc)), "docs/themes.md misses {}", k.id);
        }
        for s in Scope::ALL {
            assert!(docs.contains(&format!("`{}`", s.name())), "docs/themes.md misses {}", s.name());
        }
    }

    #[test]
    fn roots_define_every_key() {
        // The roots start from a blank theme, so a missing key would be
        // invisible; check their files list every key and scope directly.
        for id in ["yace-dark", "yace-light"] {
            let text = BUILTIN.iter().find(|b| b.0 == id).unwrap().3;
            let t: Table = text.parse().unwrap();
            let colors = t["colors"].as_table().unwrap();
            for k in COLOR_KEYS {
                assert!(colors.contains_key(k.id), "{id} misses {}", k.id);
            }
            let syn = t["syntax"].as_table().unwrap();
            for s in Scope::ALL {
                assert!(syn.contains_key(s.name()), "{id} misses {}", s.name());
            }
        }
    }

    fn cat_with(files: &[(&str, &str)]) -> (tempfile::TempDir, Catalog) {
        let d = tempfile::tempdir().unwrap();
        for (name, text) in files {
            std::fs::write(d.path().join(name), text).unwrap();
        }
        let (cat, errors) = Catalog::with_user_dir(d.path());
        assert!(errors.is_empty(), "{errors:?}");
        (d, cat)
    }

    #[test]
    fn inheritance_overrides_and_derived_keys() {
        let (_d, cat) = cat_with(&[
            ("mine.toml", "name = \"Mine\"\nextends = \"Dracula\"\n[colors]\n\"statusBar.background\" = \"#010203\"\n"),
            (
                "deeper.toml",
                "name = \"Deeper\"\nextends = \"mine\"\n[colors]\n\"editor.background\" = \"#000000\"\n[syntax]\ncomment = { italic = true }\n",
            ),
        ]);
        let dracula = load_theme(&cat, "dracula", &Overrides::default()).unwrap().0;
        let (t, w) = load_theme(&cat, "Deeper", &Overrides::default()).unwrap();
        assert!(w.is_empty(), "{w:?}");
        assert_eq!(t.status_bg, Color::Rgb(1, 2, 3));
        assert_eq!(t.bg, Color::Rgb(0, 0, 0));
        // Derived: the active tab follows the new background.
        assert_eq!(t.tab_active_bg, Color::Rgb(0, 0, 0));
        assert_eq!(t.sidebar_bg, dracula.sidebar_bg);
        assert_eq!(t.syntax[Scope::Comment as usize], dracula.syntax[Scope::Comment as usize]);
        assert!(t.syntax_mods[Scope::Comment as usize].contains(Modifier::ITALIC));
        // A light theme with no `extends` starts from YACE Light.
        let (_d2, cat2) = cat_with(&[("l.toml", "name = \"L\"\ntype = \"light\"\n")]);
        let l = load_theme(&cat2, "L", &Overrides::default()).unwrap().0;
        assert_eq!(l.bg, Theme::builtin("yace-light").bg);
        assert_eq!(l.kind, ThemeKind::Light);
    }

    #[test]
    fn cycles_unknown_parents_and_bad_files() {
        let (_d, cat) = cat_with(&[
            ("a.toml", "name = \"A\"\nextends = \"b\"\n"),
            ("b.toml", "name = \"B\"\nextends = \"a\"\n"),
            ("c.toml", "name = \"C\"\nextends = \"nope\"\n"),
        ]);
        let e = load_theme(&cat, "A", &Overrides::default()).unwrap_err();
        assert!(e.message.contains("cycle"), "{e}");
        let e = load_theme(&cat, "C", &Overrides::default()).unwrap_err();
        assert!(e.message.contains("unknown theme `nope`"), "{e}");
        assert!(load_theme(&cat, "missing", &Overrides::default()).is_err());
        let d = tempfile::tempdir().unwrap();
        std::fs::write(d.path().join("bad.toml"), "name = \"Bad\"\n[colors\n").unwrap();
        let (_, errors) = Catalog::with_user_dir(d.path());
        assert_eq!(errors.len(), 1);
        assert!(errors[0].message.starts_with("line 2"), "{}", errors[0]);
    }

    #[test]
    fn unknown_keys_warn_but_load() {
        let (_d, cat) = cat_with(&[(
            "w.toml",
            "name = \"W\"\nfoo = 1\n[colors]\n\"nope.color\" = \"#000\"\n\"focusBorder\" = \"blue-ish\"\n[syntax]\n\"not.a.scope\" = \"#fff\"\n",
        )]);
        let (t, w) = load_theme(&cat, "W", &Overrides::default()).unwrap();
        assert_eq!(&*t.name, "W");
        let keys: Vec<_> = w.iter().filter_map(|e| e.key.clone()).collect();
        assert_eq!(keys.len(), 4, "{w:?}");
        assert!(keys.contains(&"foo".to_string()) && keys.contains(&"nope.color".to_string()));
        assert!(w.iter().all(|e| e.origin.ends_with("w.toml")));
    }

    #[test]
    fn user_overrides_apply_in_order() {
        let cat = Catalog::builtin();
        let colors: Table = toml::from_str(
            r##""statusBar.background" = "#111111"
"quickInput.background" = "#222222"
"[Dracula]" = { "statusBar.background" = "#333333" }
"[One Dark]" = { "statusBar.background" = "#444444" }
"##,
        )
        .unwrap();
        let tokens: Table = toml::from_str(
            r##"comments = "#555555"
keywords = { foreground = "#666666", fontStyle = "bold" }
"function.method" = { fg = "#777777", italic = true }
"##,
        )
        .unwrap();
        let ov = Overrides { colors: Some(&colors), tokens: Some(&tokens) };
        let (t, w) = load_theme(&cat, "Dracula", &ov).unwrap();
        assert!(w.is_empty(), "{w:?}");
        assert_eq!(t.status_bg, Color::Rgb(0x33, 0x33, 0x33));
        assert_eq!(t.overlay_bg, Color::Rgb(0x22, 0x22, 0x22));
        assert_eq!(t.syntax[Scope::Comment as usize], Color::Rgb(0x55, 0x55, 0x55));
        assert_eq!(t.syntax[Scope::KeywordControl as usize], Color::Rgb(0x66, 0x66, 0x66));
        assert!(t.syntax_mods[Scope::Keyword as usize].contains(Modifier::BOLD));
        assert!(t.syntax_mods[Scope::FunctionMethod as usize].contains(Modifier::ITALIC));
        let (t, _) = load_theme(&cat, "YACE Dark", &ov).unwrap();
        assert_eq!(t.status_bg, Color::Rgb(0x11, 0x11, 0x11));
    }
}
