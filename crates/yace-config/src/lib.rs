//! # yace-config
//!
//! Layered settings (spec §17): built-in defaults < user `config.toml` <
//! workspace settings < per-root (repository) settings. Each key is
//! validated on its own: an invalid value is reported with its file and key
//! and the previous valid value is kept, so one typo never breaks the editor.
//!
//! VS Code settings (from `.code-workspace` files and `<root>/.vscode/settings.json`)
//! are mapped onto YACE keys for a known subset; everything else is ignored
//! (and never rewritten).

mod settings;

use std::{
    fmt, fs, io,
    path::{Path, PathBuf},
};

use serde::Deserialize;
pub use settings::*;
use toml::{Table, Value};
pub use toml_edit;
use yace_keymap::BindingSpec;

/// Which layer a setting came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Level {
    User,
    Workspace,
    Root,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ConfigError {
    /// File (or label) the problem came from.
    pub origin: String,
    /// `section.key`, when the problem is a single setting.
    pub key: Option<String>,
    pub message: String,
}

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.key {
            Some(k) => write!(f, "{}: {k}: {}", self.origin, self.message),
            None => write!(f, "{}: {}", self.origin, self.message),
        }
    }
}

/// One layer of raw settings.
#[derive(Clone, Debug)]
pub struct Layer {
    pub level: Level,
    pub origin: String,
    pub values: Table,
}

impl Layer {
    pub fn new(level: Level, origin: impl Into<String>, values: Table) -> Self {
        Self { level, origin: origin.into(), values }
    }
}

fn defaults_table() -> Table {
    match Value::try_from(Settings::default()) {
        Ok(Value::Table(t)) => t,
        _ => unreachable!("Settings serializes to a table"),
    }
}

fn level_allows(level: Level, scope: Scope) -> bool {
    match level {
        Level::User => true,
        Level::Workspace => scope >= Scope::Window,
        Level::Root => scope == Scope::Resource,
    }
}

/// Merge `files.exclude` additively: `"!glob"` removes a glob.
fn merge_exclude(current: &Value, incoming: &Value) -> Option<Value> {
    let mut list: Vec<Value> = current.as_array()?.clone();
    for item in incoming.as_array()? {
        let s = item.as_str()?;
        if let Some(removed) = s.strip_prefix('!') {
            list.retain(|v| v.as_str() != Some(removed));
        } else if !list.iter().any(|v| v.as_str() == Some(s)) {
            list.push(item.clone());
        }
    }
    Some(Value::Array(list))
}

/// Merge colour override tables key by key (one level deep, so
/// `"[Theme]"` sections merge too), like VS Code merges objects.
fn merge_tables(current: Option<&Value>, incoming: &Value) -> Value {
    let (Some(Value::Table(cur)), Value::Table(inc)) = (current, incoming) else { return incoming.clone() };
    let mut out = cur.clone();
    for (k, v) in inc {
        let merged = match (out.get(k), v) {
            (Some(Value::Table(a)), Value::Table(b)) => {
                let mut t = a.clone();
                t.extend(b.iter().map(|(k, v)| (k.clone(), v.clone())));
                Value::Table(t)
            }
            _ => v.clone(),
        };
        out.insert(k.clone(), merged);
    }
    Value::Table(out)
}

/// VS Code theme names → YACE built-in theme names (`workbench.colorTheme`).
pub fn map_vscode_theme_name(name: &str) -> &str {
    match name {
        "Default Dark+"
        | "Default Dark Modern"
        | "Dark+"
        | "Dark Modern"
        | "Visual Studio Dark"
        | "Dark (Visual Studio)"
        | "Default Dark+ Experimental" => "YACE Dark",
        "Default Light+"
        | "Default Light Modern"
        | "Light+"
        | "Light Modern"
        | "Visual Studio Light"
        | "Light (Visual Studio)" => "YACE Light",
        n if n.starts_with("One Dark") || n.starts_with("Atom One Dark") => "One Dark",
        n if n.starts_with("Dracula") => "Dracula",
        n if n.starts_with("Catppuccin") => "Catppuccin Mocha",
        n if n.starts_with("Tokyo Night") => "Tokyo Night",
        n => n,
    }
}

/// Resolve layers (in precedence order) into settings plus errors.
pub fn resolve(layers: &[Layer]) -> (Settings, Vec<ConfigError>) {
    let mut current = defaults_table();
    let mut errors = Vec::new();
    for layer in layers {
        for (section, value) in &layer.values {
            let Value::Table(entries) = value else {
                errors.push(ConfigError {
                    origin: layer.origin.clone(),
                    key: Some(section.clone()),
                    message: "expected a table of settings".into(),
                });
                continue;
            };
            for (key, v) in entries {
                let full = format!("{section}.{key}");
                if !level_allows(layer.level, scope_of(section, key)) {
                    errors.push(ConfigError {
                        origin: layer.origin.clone(),
                        key: Some(full),
                        message: format!("cannot be set in {:?} settings", layer.level).to_lowercase(),
                    });
                    continue;
                }
                let mut candidate = current.clone();
                let Some(Value::Table(sec)) = candidate.get_mut(section.as_str()) else {
                    errors.push(ConfigError {
                        origin: layer.origin.clone(),
                        key: Some(full),
                        message: format!("unknown section `{section}`"),
                    });
                    continue;
                };
                let merged = if (section == "files" || section == "search") && key == "exclude" {
                    sec.get(key).and_then(|c| merge_exclude(c, v)).unwrap_or_else(|| v.clone())
                } else if key == "colorCustomizations" || key == "tokenColorCustomizations" {
                    merge_tables(sec.get(key), v)
                } else {
                    v.clone()
                };
                sec.insert(key.clone(), merged);
                match Settings::deserialize(Value::Table(candidate.clone())) {
                    Ok(_) => current = candidate,
                    Err(e) => errors.push(ConfigError {
                        origin: layer.origin.clone(),
                        key: Some(full),
                        message: clean_serde_message(&e.to_string()),
                    }),
                }
            }
        }
    }
    let settings = Settings::deserialize(Value::Table(current)).unwrap_or_default();
    (settings, errors)
}

fn clean_serde_message(m: &str) -> String {
    // toml's messages include a source snippet we don't have; keep line 1.
    m.lines().next().unwrap_or(m).trim().to_string()
}

/// Parse a TOML settings file into a layer. A missing file is an empty layer.
pub fn load_toml_layer(path: &Path, level: Level) -> (Layer, Vec<ConfigError>) {
    let origin = path.display().to_string();
    match fs::read_to_string(path) {
        Ok(text) => match text.parse::<Table>() {
            Ok(t) => (Layer::new(level, origin, t), vec![]),
            Err(e) => (
                Layer::new(level, origin.clone(), Table::new()),
                vec![ConfigError { origin, key: None, message: clean_serde_message(&e.to_string()) }],
            ),
        },
        Err(e) if e.kind() == io::ErrorKind::NotFound => (Layer::new(level, origin, Table::new()), vec![]),
        Err(e) => (
            Layer::new(level, origin.clone(), Table::new()),
            vec![ConfigError { origin, key: None, message: e.to_string() }],
        ),
    }
}

/// Parse JSONC (VS Code dialect: comments, trailing commas).
pub fn parse_jsonc(text: &str) -> Result<serde_json::Value, String> {
    let opts = jsonc_parser::ParseOptions::default();
    jsonc_parser::parse_to_serde_value::<Option<serde_json::Value>>(text, &opts)
        .map(|v| v.unwrap_or(serde_json::Value::Null))
        .map_err(|e| e.to_string())
}

fn json_to_toml(v: &serde_json::Value) -> Option<Value> {
    Some(match v {
        serde_json::Value::Null => return None,
        serde_json::Value::Bool(b) => Value::Boolean(*b),
        serde_json::Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                Value::Integer(i)
            } else {
                Value::Float(n.as_f64()?)
            }
        }
        serde_json::Value::String(s) => Value::String(s.clone()),
        serde_json::Value::Array(a) => Value::Array(a.iter().filter_map(json_to_toml).collect()),
        serde_json::Value::Object(o) => {
            Value::Table(o.iter().filter_map(|(k, v)| Some((k.clone(), json_to_toml(v)?))).collect())
        }
    })
}

/// Map VS Code settings (flat dotted keys) onto YACE settings. Unknown keys
/// are ignored; a `"yace"` object is taken verbatim as YACE settings.
pub fn from_vscode_settings(json: &serde_json::Value) -> Table {
    let mut out = Table::new();
    let Some(obj) = json.as_object() else { return out };
    let mut put = |section: &str, key: &str, v: Value| {
        let sec = out.entry(section.to_string()).or_insert_with(|| Value::Table(Table::new()));
        if let Value::Table(t) = sec {
            t.insert(key.to_string(), v);
        }
    };
    for (k, v) in obj {
        use serde_json::Value as J;
        match (k.as_str(), v) {
            ("editor.tabSize", J::Number(n)) if n.as_u64().is_some() => {
                put("editor", "tabSize", Value::Integer(n.as_u64().unwrap_or(4) as i64))
            }
            ("editor.insertSpaces", J::Bool(b)) => put("editor", "insertSpaces", Value::Boolean(*b)),
            ("editor.autoClosingBrackets", J::String(s)) => {
                put("editor", "autoClosingBrackets", Value::Boolean(s != "never"))
            }
            ("editor.lineNumbers", J::String(s)) if matches!(s.as_str(), "on" | "off" | "relative") => {
                put("editor", "lineNumbers", Value::String(s.clone()))
            }
            ("editor.renderLineHighlight", J::String(s)) => {
                put("editor", "renderLineHighlight", Value::Boolean(s != "none"))
            }
            ("editor.cursorSurroundingLines", J::Number(n)) if n.as_u64().is_some() => {
                put("editor", "cursorSurroundingLines", Value::Integer(n.as_u64().unwrap_or(0) as i64))
            }
            ("editor.scrollBeyondLastLine", J::Bool(b)) => put("editor", "scrollBeyondLastLine", Value::Boolean(*b)),
            ("editor.matchBrackets", J::String(s)) => put("editor", "matchBrackets", Value::Boolean(s != "never")),
            ("editor.wordWrap", J::String(s)) if matches!(s.as_str(), "off" | "on" | "wordWrapColumn" | "bounded") => {
                put("editor", "wordWrap", Value::String(s.clone()))
            }
            ("editor.wordWrapColumn", J::Number(n)) if n.as_u64().is_some() => {
                put("editor", "wordWrapColumn", Value::Integer(n.as_u64().unwrap_or(80) as i64))
            }
            ("editor.folding", J::Bool(b)) => put("editor", "folding", Value::Boolean(*b)),
            ("editor.formatOnSave", J::Bool(b)) => put("editor", "formatOnSave", Value::Boolean(*b)),
            ("editor.quickSuggestions", J::Bool(b)) => put("editor", "quickSuggestions", Value::Boolean(*b)),
            // The object form ({other, comments, strings}): "other" decides.
            ("editor.quickSuggestions", J::Object(o)) => {
                let on = match o.get("other") {
                    None => true,
                    Some(J::Bool(b)) => *b,
                    Some(J::String(s)) => s == "on",
                    Some(_) => true,
                };
                put("editor", "quickSuggestions", Value::Boolean(on))
            }
            ("editor.quickSuggestionsDelay", J::Number(n)) if n.as_u64().is_some() => {
                put("editor", "quickSuggestionsDelay", Value::Integer(n.as_u64().unwrap_or(50) as i64))
            }
            ("editor.suggestOnTriggerCharacters", J::Bool(b)) => {
                put("editor", "suggestOnTriggerCharacters", Value::Boolean(*b))
            }
            ("editor.showFoldingControls", J::String(s)) if matches!(s.as_str(), "always" | "mouseover" | "never") => {
                put("editor", "showFoldingControls", Value::String(s.clone()))
            }
            ("files.exclude", J::Object(globs)) => {
                let list = globs
                    .iter()
                    .filter_map(|(g, on)| match on {
                        J::Bool(true) => Some(Value::String(g.clone())),
                        J::Bool(false) => Some(Value::String(format!("!{g}"))),
                        _ => None, // `when` conditions are not supported
                    })
                    .collect();
                put("files", "exclude", Value::Array(list))
            }
            ("search.exclude", J::Object(globs)) => {
                let list = globs
                    .iter()
                    .filter_map(|(g, on)| match on {
                        J::Bool(true) => Some(Value::String(g.clone())),
                        J::Bool(false) => Some(Value::String(format!("!{g}"))),
                        _ => None,
                    })
                    .collect();
                put("search", "exclude", Value::Array(list))
            }
            ("files.associations", J::Object(map)) => {
                let t: Table = map
                    .iter()
                    .filter_map(|(g, id)| id.as_str().map(|id| (g.clone(), Value::String(id.into()))))
                    .collect();
                put("files", "associations", Value::Table(t))
            }
            ("search.useIgnoreFiles", J::Bool(b)) => put("search", "useIgnoreFiles", Value::Boolean(*b)),
            ("files.autoSave", J::String(s)) => {
                let mapped = match s.as_str() {
                    "off" => "off",
                    "afterDelay" => "afterDelay",
                    "onFocusChange" | "onWindowChange" => "onFocusChange",
                    _ => continue,
                };
                put("files", "autoSave", Value::String(mapped.into()))
            }
            ("files.autoSaveDelay", J::Number(n)) if n.as_u64().is_some() => {
                put("files", "autoSaveDelay", Value::Integer(n.as_u64().unwrap_or(1000) as i64))
            }
            ("files.insertFinalNewline", J::Bool(b)) => put("files", "insertFinalNewline", Value::Boolean(*b)),
            ("files.trimTrailingWhitespace", J::Bool(b)) => put("files", "trimTrailingWhitespace", Value::Boolean(*b)),
            ("files.eol", J::String(s)) => {
                let mapped = match s.as_str() {
                    "\n" => "lf",
                    "\r\n" => "crlf",
                    _ => "auto",
                };
                put("files", "eol", Value::String(mapped.into()))
            }
            ("workbench.editor.enablePreview", J::Bool(b)) => put("workbench", "enablePreview", Value::Boolean(*b)),
            ("workbench.colorTheme", J::String(s)) => {
                put("workbench", "colorTheme", Value::String(map_vscode_theme_name(s).to_string()))
            }
            ("workbench.colorCustomizations", J::Object(_)) => {
                if let Some(t) = json_to_toml(v) {
                    put("workbench", "colorCustomizations", t)
                }
            }
            ("editor.tokenColorCustomizations", J::Object(_)) => {
                if let Some(t) = json_to_toml(v) {
                    put("editor", "tokenColorCustomizations", t)
                }
            }
            ("git.enabled", J::Bool(b)) => put("git", "enabled", Value::Boolean(*b)),
            ("git.decorations.enabled", J::Bool(b)) => put("git", "decorationsEnabled", Value::Boolean(*b)),
            ("git.autorefresh", J::Bool(b)) => put("git", "autoRefresh", Value::Boolean(*b)),
            ("scm.diffDecorations", J::String(s)) if matches!(s.as_str(), "all" | "gutter" | "none") => {
                put("git", "diffDecorations", Value::String(s.clone()))
            }
            ("workbench.sideBar.location", J::String(s)) if matches!(s.as_str(), "left" | "right") => {
                put("workbench", "sideBarLocation", Value::String(s.clone()))
            }
            ("yace", J::Object(_)) => {
                if let Some(Value::Table(t)) = json_to_toml(v) {
                    for (section, entries) in t {
                        if let Value::Table(entries) = entries {
                            for (key, val) in entries {
                                put(&section, &key, val);
                            }
                        }
                    }
                }
            }
            _ => {}
        }
    }
    out
}

/// Load `<root>/.vscode/settings.json` (if present) as a root layer.
pub fn load_root_layer(root: &Path) -> (Option<Layer>, Vec<ConfigError>) {
    let path = root.join(".vscode").join("settings.json");
    match fs::read_to_string(&path) {
        Ok(text) => match parse_jsonc(&text) {
            Ok(json) => {
                // VS Code folder settings legitimately hold window-level keys
                // (theme, …); a root can only change resource settings, so
                // drop the rest silently instead of reporting them.
                let mut values = from_vscode_settings(&json);
                for (section, entries) in values.iter_mut() {
                    if let Value::Table(t) = entries {
                        t.retain(|k, _| scope_of(section, k) == Scope::Resource);
                    }
                }
                values.retain(|_, v| !matches!(v, Value::Table(t) if t.is_empty()));
                (Some(Layer::new(Level::Root, path.display().to_string(), values)), vec![])
            }
            Err(e) => (None, vec![ConfigError { origin: path.display().to_string(), key: None, message: e }]),
        },
        Err(_) => (None, vec![]),
    }
}

/// XDG locations (spec §17.2). Overridable with `YACE_CONFIG_DIR`,
/// `YACE_STATE_DIR` and `YACE_CACHE_DIR` (used by tests and portable setups).
#[derive(Clone, Debug)]
pub struct Paths {
    pub config_dir: PathBuf,
    pub state_dir: PathBuf,
    pub cache_dir: PathBuf,
}

impl Paths {
    pub fn discover() -> Self {
        let dirs = directories::ProjectDirs::from("", "", "yace");
        let home =
            directories::BaseDirs::new().map(|b| b.home_dir().to_path_buf()).unwrap_or_else(|| PathBuf::from("."));
        let env = |k: &str| std::env::var_os(k).filter(|v| !v.is_empty()).map(PathBuf::from);
        let config_dir = env("YACE_CONFIG_DIR")
            .or_else(|| dirs.as_ref().map(|d| d.config_dir().to_path_buf()))
            .unwrap_or_else(|| home.join(".config/yace"));
        let state_dir = env("YACE_STATE_DIR")
            .or_else(|| dirs.as_ref().and_then(|d| d.state_dir().map(Path::to_path_buf)))
            .unwrap_or_else(|| home.join(".local/state/yace"));
        let cache_dir = env("YACE_CACHE_DIR")
            .or_else(|| dirs.as_ref().map(|d| d.cache_dir().to_path_buf()))
            .unwrap_or_else(|| home.join(".cache/yace"));
        Self { config_dir, state_dir, cache_dir }
    }

    pub fn in_dir(base: &Path) -> Self {
        Self { config_dir: base.join("config"), state_dir: base.join("state"), cache_dir: base.join("cache") }
    }

    pub fn user_settings(&self) -> PathBuf {
        self.config_dir.join("config.toml")
    }
    pub fn keybindings(&self) -> PathBuf {
        self.config_dir.join("keybindings.toml")
    }
    /// User themes (`*.toml`, spec §17.2).
    pub fn themes_dir(&self) -> PathBuf {
        self.config_dir.join("themes")
    }
    pub fn workspaces_state_dir(&self) -> PathBuf {
        self.state_dir.join("workspaces")
    }
    pub fn recent_file(&self) -> PathBuf {
        self.state_dir.join("recent.json")
    }
    pub fn index_cache_dir(&self) -> PathBuf {
        self.cache_dir.clone()
    }
    pub fn log_file(&self) -> PathBuf {
        self.state_dir.join("yace.log")
    }
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct KeybindingsFile {
    #[serde(default)]
    bindings: Vec<BindingSpec>,
}

/// Load `keybindings.toml`. A missing file means no user bindings.
pub fn load_keybindings(path: &Path) -> (Vec<BindingSpec>, Vec<ConfigError>) {
    let origin = path.display().to_string();
    match fs::read_to_string(path) {
        Ok(text) => match toml::from_str::<KeybindingsFile>(&text) {
            Ok(f) => (f.bindings, vec![]),
            Err(e) => (vec![], vec![ConfigError { origin, key: None, message: clean_serde_message(&e.to_string()) }]),
        },
        Err(e) if e.kind() == io::ErrorKind::NotFound => (vec![], vec![]),
        Err(e) => (vec![], vec![ConfigError { origin, key: None, message: e.to_string() }]),
    }
}

/// Atomically write `bytes` to `path` (temp file + fsync + rename),
/// creating parent directories. Used for all state files (spec §26.2).
pub fn atomic_write(path: &Path, bytes: &[u8]) -> io::Result<()> {
    use std::io::Write;
    let dir = path.parent().filter(|p| !p.as_os_str().is_empty()).unwrap_or(Path::new("."));
    fs::create_dir_all(dir)?;
    let mut tmp = tempfile::Builder::new().prefix(".yace-tmp-").tempfile_in(dir)?;
    tmp.write_all(bytes)?;
    tmp.as_file().sync_all()?;
    tmp.persist(path).map_err(|e| e.error)?;
    Ok(())
}

/// Starter `config.toml` written by "Open User Settings" when none exists.
pub const DEFAULT_CONFIG_TEMPLATE: &str = r#"# YACE user settings. See docs/configuration.md for every key.
# Invalid values are reported and ignored; defaults are used instead.

[editor]
# tabSize = 4
# insertSpaces = true
# lineNumbers = "on"        # "on" | "off" | "relative"

[files]
# autoSave = "off"          # "off" | "afterDelay" | "onFocusChange"
# trimTrailingWhitespace = false
# insertFinalNewline = false
# exclude = ["**/node_modules", "!**/.git"]   # additive; "!" removes

[workbench]
# enablePreview = true
# glyphs = "unicode"        # "unicode" | "nerdFont" | "ascii"
"#;

/// Edit a TOML file in place, keeping comments, order and formatting of
/// everything not touched. A missing file starts empty. Written atomically.
pub fn edit_toml(path: &Path, f: impl FnOnce(&mut toml_edit::DocumentMut)) -> Result<(), String> {
    let text = match fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) if e.kind() == io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(format!("{}: {e}", path.display())),
    };
    let mut doc: toml_edit::DocumentMut =
        text.parse().map_err(|e: toml_edit::TomlError| format!("{}: {}", path.display(), e.message()))?;
    f(&mut doc);
    atomic_write(path, doc.to_string().as_bytes()).map_err(|e| format!("{}: {e}", path.display()))
}

/// Set (or with `None`, remove) `section.key` in a settings file.
pub fn edit_setting(path: &Path, section: &str, key: &str, value: Option<toml_edit::Value>) -> Result<(), String> {
    edit_toml(path, |doc| match value {
        Some(v) => {
            if !doc.contains_table(section) {
                doc.insert(section, toml_edit::Item::Table(toml_edit::Table::new()));
            }
            if let Some(t) = doc.get_mut(section).and_then(|i| i.as_table_like_mut()) {
                match t.get_mut(key).and_then(|i| i.as_value_mut()) {
                    // Keep the comment decoration around the old value.
                    Some(old) => {
                        let decor = old.decor().clone();
                        *old = v;
                        *old.decor_mut() = decor;
                    }
                    None => {
                        t.insert(key, toml_edit::Item::Value(v));
                    }
                }
            }
        }
        None => {
            if let Some(t) = doc.get_mut(section).and_then(|i| i.as_table_like_mut()) {
                t.remove(key);
            }
        }
    })
}

/// Starter `keybindings.toml`.
pub const DEFAULT_KEYBINDINGS_TEMPLATE: &str = r#"# YACE user keybindings. Later entries win; "-command" removes a default.
# Key syntax follows VS Code: "ctrl+shift+p", "ctrl+k ctrl+t".
#
# [[bindings]]
# key = "ctrl+alt+s"
# command = "workbench.action.files.saveAll"
#
# [[bindings]]
# command = "-workbench.action.quickOpen"   # remove the default Ctrl+P
"#;

#[cfg(test)]
mod tests {
    use super::*;

    fn table(s: &str) -> Table {
        s.parse().unwrap()
    }

    #[test]
    fn precedence_and_per_key_fallback() {
        let layers = vec![
            Layer::new(
                Level::User,
                "user",
                table("[editor]\ntabSize = 2\ninsertSpaces = \"yes\"\n[files]\nautoSave = \"afterDelay\""),
            ),
            Layer::new(Level::Workspace, "ws", table("[editor]\ntabSize = 8")),
        ];
        let (s, errors) = resolve(&layers);
        assert_eq!(s.editor.tab_size, 8, "workspace wins");
        assert!(s.editor.insert_spaces, "invalid value falls back to default");
        assert_eq!(s.files.auto_save, AutoSave::AfterDelay);
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].key.as_deref(), Some("editor.insertSpaces"));
        assert_eq!(errors[0].origin, "user");
    }

    #[test]
    fn scopes_are_enforced() {
        let layers = vec![
            Layer::new(Level::Root, "root", table("[editor]\ntabSize = 3\n[workbench]\nsidebarWidth = 10")),
            Layer::new(Level::Workspace, "ws", table("[keyboard]\nchordTimeoutMs = 1")),
        ];
        let (s, errors) = resolve(&layers);
        assert_eq!(s.editor.tab_size, 3);
        assert_eq!(s.workbench.sidebar_width, 32, "layout not settable per root");
        assert_eq!(s.keyboard.chord_timeout_ms, 3000, "application setting not settable in workspace");
        assert_eq!(errors.len(), 2);
    }

    #[test]
    fn unknown_keys_reported() {
        let (_, errors) = resolve(&[Layer::new(Level::User, "u", table("[editor]\nfontSize = 12\n[nope]\nx = 1"))]);
        assert_eq!(errors.len(), 2);
    }

    #[test]
    fn exclude_is_additive_with_removal() {
        let (s, _) =
            resolve(&[Layer::new(Level::User, "u", table("[files]\nexclude = [\"**/node_modules\", \"!**/.git\"]"))]);
        assert!(s.files.exclude.contains(&"**/node_modules".to_string()));
        assert!(!s.files.exclude.contains(&"**/.git".to_string()));
        assert!(s.files.exclude.contains(&"**/.hg".to_string()));
    }

    #[test]
    fn maps_vscode_settings_from_jsonc() {
        let json = parse_jsonc(
            r#"{
              // comment
              "editor.tabSize": 2,
              "files.exclude": { "**/dist": true, "**/.git": false },
              "files.eol": "\r\n",
              "editor.fontFamily": "ignored",
              "yace": { "workbench": { "glyphs": "ascii" } },
            }"#,
        )
        .unwrap();
        let t = from_vscode_settings(&json);
        let (s, errors) = resolve(&[Layer::new(Level::Workspace, "ws", t)]);
        assert_eq!(errors.len(), 1, "only the application-scoped key is rejected: {errors:?}");
        assert_eq!(errors[0].key.as_deref(), Some("workbench.glyphs"));
        assert_eq!(s.editor.tab_size, 2);
        assert_eq!(s.files.eol, Eol::Crlf);
        assert!(s.files.exclude.contains(&"**/dist".to_string()));
        assert!(!s.files.exclude.contains(&"**/.git".to_string()));
        // glyphs is application-scoped, so the workspace cannot set it
        assert_eq!(s.workbench.glyphs, Glyphs::Unicode);
    }

    #[test]
    fn edit_setting_keeps_comments() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("config.toml");
        std::fs::write(&p, "# my settings\n[editor]\ntabSize = 2 # two!\n\n[workbench]\n# the theme\ncolorTheme = \"Dracula\" # nice\n").unwrap();
        edit_setting(&p, "workbench", "colorTheme", Some("One Dark".into())).unwrap();
        let text = std::fs::read_to_string(&p).unwrap();
        assert_eq!(
            text,
            "# my settings\n[editor]\ntabSize = 2 # two!\n\n[workbench]\n# the theme\ncolorTheme = \"One Dark\" # nice\n"
        );
        edit_setting(&p, "files", "autoSave", Some("afterDelay".into())).unwrap();
        edit_setting(&p, "editor", "tabSize", None).unwrap();
        let t: Table = std::fs::read_to_string(&p).unwrap().parse().unwrap();
        assert_eq!(t["files"]["autoSave"].as_str(), Some("afterDelay"));
        assert!(t["editor"].as_table().unwrap().is_empty());
        // A missing file (and directory) is created.
        let q = dir.path().join("new").join("config.toml");
        edit_setting(&q, "workbench", "colorTheme", Some("Dracula".into())).unwrap();
        assert!(std::fs::read_to_string(&q).unwrap().contains("[workbench]\ncolorTheme = \"Dracula\""));
        // Invalid TOML is refused, never overwritten.
        std::fs::write(&p, "[broken").unwrap();
        assert!(edit_setting(&p, "a", "b", Some(1.into())).is_err());
        assert_eq!(std::fs::read_to_string(&p).unwrap(), "[broken");
    }

    #[test]
    fn keybindings_file_and_atomic_write() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("sub").join("keybindings.toml");
        atomic_write(&p, b"[[bindings]]\nkey = \"ctrl+alt+s\"\ncommand = \"x\"\n").unwrap();
        let (specs, errors) = load_keybindings(&p);
        assert!(errors.is_empty());
        assert_eq!(specs.len(), 1);
        atomic_write(&p, b"[[bindings]\n").unwrap();
        let (_, errors) = load_keybindings(&p);
        assert_eq!(errors.len(), 1);
        let (_, errors) = load_keybindings(&dir.path().join("missing.toml"));
        assert!(errors.is_empty());
    }

    #[test]
    fn templates_parse() {
        assert!(DEFAULT_CONFIG_TEMPLATE.parse::<Table>().is_ok());
        assert!(toml::from_str::<KeybindingsFile>(DEFAULT_KEYBINDINGS_TEMPLATE).is_ok());
    }
}
