//! `yace-plugin.toml` (spec §18.7): parsed and validated before any plugin
//! code is loaded. Everything a plugin contributes declaratively (commands,
//! keybindings, themes, grammars, languages, views, settings…) is known
//! from the manifest alone, so nothing is loaded until an activation event.

use std::{
    collections::{BTreeMap, HashSet},
    path::{Component, Path, PathBuf},
};

pub use semver::{Version, VersionReq};
use serde::Deserialize;

pub const MANIFEST_FILE: &str = "yace-plugin.toml";

/// Capabilities a plugin can declare. They are shown before installing and
/// checked at the host API boundary; they are **not** a sandbox (spec
/// §18.9): native code runs with the editor's permissions.
pub const CAPABILITIES: &[(&str, &str)] = &[
    ("workspace.read", "list workspace folders"),
    ("documents.read", "read the text of open documents"),
    ("documents.write", "edit open documents"),
    ("terminal", "open integrated terminals and run commands in them"),
    ("process", "start programs"),
    ("network", "use the network"),
    ("filesystem", "read and write files"),
];

/// Activation events (spec §18.6).
pub const ACTIVATION_PREFIXES: &[&str] =
    &["onCommand:", "onLanguage:", "onView:", "workspaceContains:", "onEvent:", "onQuickOpen:"];
pub const ACTIVATION_PLAIN: &[&str] = &["onStartupFinished", "*"];

/// Events a plugin can subscribe to (spec §18.4).
pub const EVENTS: &[&str] = &[
    "workspaceOpened",
    "workspaceClosed",
    "rootAdded",
    "rootRemoved",
    "documentOpened",
    "documentClosed",
    "documentSaved",
    "documentChanged",
    "activeDocumentChanged",
    "selectionChanged",
    "themeChanged",
    "terminalCreated",
    "terminalExited",
    "configurationChanged",
    "gitStatusChanged",
];

/// Command id prefixes reserved for the editor.
const RESERVED_PREFIXES: &[&str] = &["workbench.", "editor.", "yace.", "explorer.", "search.", "terminal."];

#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub struct Manifest {
    pub id: String,
    pub name: String,
    pub version: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub publisher: String,
    #[serde(default)]
    pub license: String,
    #[serde(default)]
    pub homepage: String,
    /// Required YACE version, e.g. `">=0.6"`.
    #[serde(default = "any_version")]
    pub yace: String,
    /// Plugin ABI major version the library was built for.
    #[serde(default = "abi_v1")]
    pub abi: u32,
    /// Supported operating systems (`linux`, `macos`, …); empty = any.
    #[serde(default)]
    pub os: Vec<String>,
    /// Supported architectures (`x86_64`, `aarch64`, …); empty = any.
    #[serde(default)]
    pub arch: Vec<String>,
    /// Entry library, relative to the plugin folder. None: data only.
    #[serde(default)]
    pub library: Option<String>,
    #[serde(default)]
    pub activation: Vec<String>,
    #[serde(default)]
    pub capabilities: Vec<String>,
    #[serde(default)]
    pub events: Vec<String>,
    /// Whether the library may be unloaded and loaded again in one session.
    #[serde(default)]
    pub unloadable: bool,
    #[serde(default)]
    pub commands: Vec<CommandDecl>,
    #[serde(default)]
    pub keybindings: Vec<KeybindingDecl>,
    #[serde(default)]
    pub status_items: Vec<StatusItemDecl>,
    #[serde(default)]
    pub views: Vec<ViewDecl>,
    #[serde(default)]
    pub quick_open: Vec<QuickOpenDecl>,
    #[serde(default)]
    pub themes: Vec<ThemeDecl>,
    #[serde(default)]
    pub grammars: Vec<GrammarDecl>,
    #[serde(default)]
    pub languages: Vec<LanguageDecl>,
    /// Language servers that run alongside a language's main server.
    #[serde(default)]
    pub lsp_servers: Vec<LspServerDecl>,
    /// Plugin settings schema, by key.
    #[serde(default)]
    pub settings: BTreeMap<String, SettingDecl>,
    /// Other plugins this one needs: id → version requirement.
    #[serde(default)]
    pub dependencies: BTreeMap<String, String>,
    /// External programs it uses (shown, not checked).
    #[serde(default)]
    pub external_programs: Vec<String>,
}

fn any_version() -> String {
    "*".into()
}
fn abi_v1() -> u32 {
    1
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct CommandDecl {
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub category: String,
    /// Listed in the command palette (default true).
    #[serde(default = "yes")]
    pub palette: bool,
}

fn yes() -> bool {
    true
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct KeybindingDecl {
    pub key: String,
    pub command: String,
    #[serde(default)]
    pub when: String,
    #[serde(default)]
    pub args: Option<toml::Value>,
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct StatusItemDecl {
    pub id: String,
    /// `left` or `right` (default).
    #[serde(default = "right")]
    pub alignment: String,
    /// Higher is further from the edge... further left on the right side.
    #[serde(default)]
    pub priority: i32,
    /// Shown before the plugin activates (optional).
    #[serde(default)]
    pub text: String,
    #[serde(default)]
    pub command: String,
}

fn right() -> String {
    "right".into()
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ViewDecl {
    pub id: String,
    pub title: String,
    /// `sidebar` (default) or `panel`.
    #[serde(default = "sidebar")]
    pub location: String,
    /// Shown while the view has no items.
    #[serde(default)]
    pub empty_text: String,
}

fn sidebar() -> String {
    "sidebar".into()
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct QuickOpenDecl {
    /// Typed first in Quick Open (e.g. `!`).
    pub prefix: String,
    pub title: String,
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ThemeDecl {
    pub path: String,
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct GrammarDecl {
    /// Language id (a built-in one, or one from `[[languages]]`).
    pub language: String,
    /// Shared library with the parser, relative to the plugin folder.
    pub library: String,
    /// Exported function returning the `TSLanguage` (default
    /// `tree_sitter_<language>`).
    #[serde(default)]
    pub symbol: String,
    /// Highlight queries, concatenated in order (base first).
    #[serde(default)]
    pub highlights: Vec<String>,
    #[serde(default)]
    pub injections: Vec<String>,
    #[serde(default)]
    pub locals: Vec<String>,
}

impl GrammarDecl {
    pub fn symbol(&self) -> String {
        if self.symbol.is_empty() {
            format!("tree_sitter_{}", self.language.replace('-', "_"))
        } else {
            self.symbol.clone()
        }
    }
}

/// A language server a plugin starts for some languages, next to their
/// main server (ESLint, Biome…). The program is not part of the plugin.
#[derive(Debug, Clone, Deserialize, PartialEq, Default)]
#[serde(deny_unknown_fields)]
pub struct LspServerDecl {
    pub name: String,
    /// A program name (found on `PATH`, or in `node_modules/.bin` first
    /// with `local_bin`).
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    pub languages: Vec<String>,
    /// Start only where one of these files is in the file's folder or a
    /// folder above it, up to the workspace folder.
    #[serde(default)]
    pub root_files: Vec<String>,
    /// Prefer the project's `node_modules/.bin/<command>`.
    #[serde(default)]
    pub local_bin: bool,
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    /// Answers `workspace/configuration`; `${workspaceFolder}`,
    /// `${workspaceFolderUri}` and `${workspaceFolderName}` are filled in.
    #[serde(default)]
    pub settings: Option<toml::Value>,
    #[serde(default)]
    pub initialization_options: Option<toml::Value>,
    /// Shown when the program isn't found.
    #[serde(default)]
    pub install_hint: String,
}

impl LspServerDecl {
    /// How the install prompt names it: `biome lsp-proxy`.
    pub fn command_line(&self) -> String {
        std::iter::once(self.command.as_str()).chain(self.args.iter().map(String::as_str)).collect::<Vec<_>>().join(" ")
    }
}

#[derive(Debug, Clone, Deserialize, PartialEq, Default)]
#[serde(deny_unknown_fields)]
pub struct LanguageDecl {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub extensions: Vec<String>,
    #[serde(default)]
    pub filenames: Vec<String>,
    #[serde(default)]
    pub shebangs: Vec<String>,
    #[serde(default)]
    pub line_comment: String,
    #[serde(default)]
    pub block_comment: Vec<String>,
    #[serde(default)]
    pub brackets: Vec<Vec<String>>,
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct SettingDecl {
    /// `string`, `boolean`, `integer`, `number`, `array` or `object`.
    #[serde(rename = "type")]
    pub kind: String,
    pub default: toml::Value,
    #[serde(default)]
    pub description: String,
    /// Allowed values (strings or numbers).
    #[serde(default, rename = "enum")]
    pub allowed: Vec<toml::Value>,
    #[serde(default)]
    pub minimum: Option<f64>,
    #[serde(default)]
    pub maximum: Option<f64>,
}

impl SettingDecl {
    /// Check a value against the declaration.
    pub fn validate(&self, v: &toml::Value) -> Result<(), String> {
        use toml::Value as V;
        let ok = matches!(
            (self.kind.as_str(), v),
            ("string", V::String(_))
                | ("boolean", V::Boolean(_))
                | ("integer", V::Integer(_))
                | ("number", V::Integer(_) | V::Float(_))
                | ("array", V::Array(_))
                | ("object", V::Table(_))
        );
        if !ok {
            return Err(format!("expected {}", self.kind));
        }
        if !self.allowed.is_empty() && !self.allowed.contains(v) {
            let list: Vec<String> = self.allowed.iter().map(ToString::to_string).collect();
            return Err(format!("expected one of {}", list.join(", ")));
        }
        let n = match v {
            V::Integer(i) => Some(*i as f64),
            V::Float(f) => Some(*f),
            _ => None,
        };
        if let Some(n) = n {
            if self.minimum.is_some_and(|m| n < m) {
                return Err(format!("must be at least {}", self.minimum.unwrap_or_default()));
            }
            if self.maximum.is_some_and(|m| n > m) {
                return Err(format!("must be at most {}", self.maximum.unwrap_or_default()));
            }
        }
        Ok(())
    }
}

/// A relative path inside the plugin folder (no `..`, not absolute).
pub fn safe_relative(p: &str) -> Option<PathBuf> {
    let path = Path::new(p);
    if p.is_empty() || path.is_absolute() {
        return None;
    }
    path.components().all(|c| matches!(c, Component::Normal(_) | Component::CurDir)).then(|| path.to_path_buf())
}

/// A plugin id: lowercase dotted words (`io.github.you.name`).
pub fn valid_id(id: &str) -> bool {
    let parts: Vec<&str> = id.split('.').collect();
    parts.len() >= 2
        && id.len() <= 100
        && parts.iter().all(|p| {
            !p.is_empty()
                && p.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_')
                && !p.starts_with('-')
        })
}

/// Parse and validate a manifest. `dir` (the plugin folder) is used to
/// check that referenced files exist. Errors are complete sentences, one
/// per problem.
pub fn parse(text: &str, dir: Option<&Path>) -> Result<Manifest, Vec<String>> {
    let table: toml::Table =
        text.parse().map_err(|e: toml::de::Error| vec![format!("invalid TOML: {}", e.message())])?;
    // Unknown top-level keys are errors, so a typo never silently drops a
    // contribution.
    const KNOWN: &[&str] = &[
        "id",
        "name",
        "version",
        "description",
        "publisher",
        "license",
        "homepage",
        "yace",
        "abi",
        "os",
        "arch",
        "library",
        "activation",
        "capabilities",
        "events",
        "unloadable",
        "commands",
        "keybindings",
        "status_items",
        "views",
        "quick_open",
        "themes",
        "grammars",
        "languages",
        "lsp_servers",
        "settings",
        "dependencies",
        "external_programs",
    ];
    let mut errors: Vec<String> =
        table.keys().filter(|k| !KNOWN.contains(&k.as_str())).map(|k| format!("unknown key `{k}`")).collect();
    let m: Manifest = match toml::Value::Table(table).try_into() {
        Ok(m) => m,
        Err(e) => {
            errors.push(e.message().trim().to_string());
            return Err(errors);
        }
    };
    let mut err = |s: String| errors.push(s);
    if !valid_id(&m.id) {
        err(format!("`id` \"{}\" must be lowercase dotted words, e.g. \"dev.example.my-plugin\"", m.id));
    }
    if m.name.trim().is_empty() {
        err("`name` is empty".into());
    }
    if semver::Version::parse(&m.version).is_err() {
        err(format!("`version` \"{}\" is not a semantic version (e.g. 1.0.0)", m.version));
    }
    if semver::VersionReq::parse(&m.yace).is_err() {
        err(format!("`yace` \"{}\" is not a version requirement (e.g. \">=0.6\")", m.yace));
    }
    let check_file = |rel: &str, what: &str, errs: &mut Vec<String>| match safe_relative(rel) {
        None => errs.push(format!("{what} `{rel}` must be a relative path inside the plugin folder")),
        Some(p) => {
            if let Some(d) = dir
                && !d.join(&p).is_file()
            {
                errs.push(format!("{what} `{rel}` does not exist"));
            }
        }
    };
    let mut errs = Vec::new();
    if let Some(lib) = &m.library {
        check_file(lib, "library", &mut errs);
    }
    for a in &m.activation {
        let ok = ACTIVATION_PLAIN.contains(&a.as_str())
            || ACTIVATION_PREFIXES.iter().any(|p| a.strip_prefix(p).is_some_and(|rest| !rest.is_empty()));
        if !ok {
            errs.push(format!("unknown activation event `{a}`"));
        }
    }
    if m.library.is_none() && !m.activation.is_empty() {
        errs.push("`activation` needs a `library` (data-only plugins never activate)".into());
    }
    for c in &m.capabilities {
        if !CAPABILITIES.iter().any(|(k, _)| k == c) {
            errs.push(format!("unknown capability `{c}`"));
        }
    }
    for e in &m.events {
        if !EVENTS.contains(&e.as_str()) {
            errs.push(format!("unknown event `{e}`"));
        }
    }
    let mut seen = HashSet::new();
    for c in &m.commands {
        if !c.id.contains('.') || RESERVED_PREFIXES.iter().any(|p| c.id.starts_with(p)) {
            errs.push(format!(
                "command id `{}` must be namespaced (e.g. \"myplugin.doThing\") and not start with {}",
                c.id,
                RESERVED_PREFIXES.join(", ")
            ));
        }
        if !seen.insert(c.id.as_str()) {
            errs.push(format!("command `{}` is declared twice", c.id));
        }
        if c.title.trim().is_empty() {
            errs.push(format!("command `{}` has no title", c.id));
        }
    }
    for k in &m.keybindings {
        if !m.commands.iter().any(|c| c.id == k.command) {
            errs.push(format!("keybinding `{}` refers to `{}`, which the plugin doesn't declare", k.key, k.command));
        }
    }
    let mut ids = HashSet::new();
    for s in &m.status_items {
        if !ids.insert(format!("status:{}", s.id)) {
            errs.push(format!("status item `{}` is declared twice", s.id));
        }
        if !matches!(s.alignment.as_str(), "left" | "right") {
            errs.push(format!("status item `{}`: alignment must be \"left\" or \"right\"", s.id));
        }
    }
    for v in &m.views {
        if !ids.insert(format!("view:{}", v.id)) {
            errs.push(format!("view `{}` is declared twice", v.id));
        }
        if !matches!(v.location.as_str(), "sidebar" | "panel") {
            errs.push(format!("view `{}`: location must be \"sidebar\" or \"panel\"", v.id));
        }
    }
    for q in &m.quick_open {
        let p = q.prefix.trim();
        if p.is_empty() || p.chars().count() > 3 || matches!(p, ">" | ":" | "?" | "@" | "#") {
            errs.push(format!("quick open prefix `{}` must be 1–3 characters and not one of > : ? @ #", q.prefix));
        }
    }
    for t in &m.themes {
        check_file(&t.path, "theme", &mut errs);
    }
    for g in &m.grammars {
        check_file(&g.library, "grammar library", &mut errs);
        for q in g.highlights.iter().chain(&g.injections).chain(&g.locals) {
            check_file(q, "query", &mut errs);
        }
        if g.language.is_empty() {
            errs.push("a grammar has no `language`".into());
        }
    }
    for l in &m.languages {
        if l.id.is_empty() || l.name.is_empty() {
            errs.push("a language needs `id` and `name`".into());
        }
        if !(l.block_comment.is_empty() || l.block_comment.len() == 2) {
            errs.push(format!("language `{}`: block_comment is [open, close]", l.id));
        }
        if l.brackets.iter().any(|b| b.len() != 2) {
            errs.push(format!("language `{}`: each bracket pair is [open, close]", l.id));
        }
    }
    let mut server_names = std::collections::HashSet::new();
    for l in &m.lsp_servers {
        let what = format!("language server `{}`", l.name);
        if l.name.is_empty() || !server_names.insert(l.name.as_str()) {
            errs.push("each language server needs a unique `name`".into());
        }
        if l.command.is_empty() || l.command.contains('/') || l.command.contains(char::is_whitespace) {
            errs.push(format!("{what}: `command` must be a program name (arguments go in `args`)"));
        }
        if l.languages.is_empty() || l.languages.iter().any(|x| x.is_empty()) {
            errs.push(format!("{what}: `languages` lists at least one language id"));
        }
        if l.root_files.iter().any(|f| f.is_empty() || f.contains('/')) {
            errs.push(format!("{what}: `root_files` are file names"));
        }
    }
    for (key, s) in &m.settings {
        if !matches!(s.kind.as_str(), "string" | "boolean" | "integer" | "number" | "array" | "object") {
            errs.push(format!("setting `{key}`: unknown type `{}`", s.kind));
        } else if let Err(e) = s.validate(&s.default) {
            errs.push(format!("setting `{key}`: default {e}"));
        }
    }
    for (dep, req) in &m.dependencies {
        if semver::VersionReq::parse(req).is_err() {
            errs.push(format!("dependency `{dep}`: \"{req}\" is not a version requirement"));
        }
    }
    errors.extend(errs);
    if errors.is_empty() { Ok(m) } else { Err(errors) }
}

impl Manifest {
    /// Why this plugin can't run here (OS, architecture, YACE or ABI
    /// version), or `Ok`.
    pub fn compatible(&self, yace_version: &str) -> Result<(), String> {
        if !self.os.is_empty() && !self.os.iter().any(|o| o == std::env::consts::OS) {
            return Err(format!("built for {} (this is {})", self.os.join(", "), std::env::consts::OS));
        }
        if !self.arch.is_empty() && !self.arch.iter().any(|a| a == std::env::consts::ARCH) {
            return Err(format!("built for {} (this is {})", self.arch.join(", "), std::env::consts::ARCH));
        }
        if self.library.is_some() && self.abi != yace_plugin_api::ABI_VERSION {
            return Err(format!(
                "uses plugin ABI {} (this YACE supports ABI {})",
                self.abi,
                yace_plugin_api::ABI_VERSION
            ));
        }
        let req = semver::VersionReq::parse(&self.yace).map_err(|e| e.to_string())?;
        let ours = semver::Version::parse(yace_version).map_err(|e| e.to_string())?;
        // Pre-releases of a matching version count as that version.
        let ours = semver::Version::new(ours.major, ours.minor, ours.patch);
        if !req.matches(&ours) {
            return Err(format!("needs YACE {} (this is {yace_version})", self.yace));
        }
        Ok(())
    }

    pub fn has_capability(&self, c: &str) -> bool {
        self.capabilities.iter().any(|x| x == c)
    }

    /// Every file holding code (the library and grammar libraries).
    pub fn code_files(&self) -> Vec<String> {
        let mut v: Vec<String> = self.library.iter().cloned().collect();
        v.extend(self.grammars.iter().map(|g| g.library.clone()));
        v.sort();
        v.dedup();
        v
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn language_servers() {
        let base = "id = \"dev.example.lint\"\nname = \"Lint\"\nversion = \"1.0.0\"\n";
        let good = format!(
            "{base}[[lsp_servers]]\nname = \"biome\"\ncommand = \"biome\"\nargs = [\"lsp-proxy\"]\n\
             languages = [\"typescript\"]\nroot_files = [\"biome.json\"]\nlocal_bin = true\n\
             settings = {{ workspaceFolder = {{ uri = \"${{workspaceFolderUri}}\" }} }}\n"
        );
        let m = parse(&good, None).unwrap();
        assert_eq!(m.lsp_servers[0].command_line(), "biome lsp-proxy");
        assert!(m.lsp_servers[0].local_bin && m.library.is_none());
        for (decl, want) in [
            ("name = \"x\"\ncommand = \"./x\"\nlanguages = [\"rust\"]", "program name"),
            ("name = \"x\"\ncommand = \"x --stdio\"\nlanguages = [\"rust\"]", "program name"),
            ("name = \"x\"\ncommand = \"x\"\nlanguages = []", "at least one language"),
            ("name = \"x\"\ncommand = \"x\"\nlanguages = [\"rust\"]\nroot_files = [\"a/b\"]", "file names"),
            ("name = \"x\"\ncommand = \"x\"\nlanguages = [\"rust\"]\nport = 1", "port"),
        ] {
            let errs = parse(&format!("{base}[[lsp_servers]]\n{decl}\n"), None).unwrap_err().join("; ");
            assert!(errs.contains(want), "{decl}: {errs}");
        }
        let twice = format!("{base}[[lsp_servers]]\nname = \"x\"\ncommand = \"x\"\nlanguages = [\"rust\"]\n");
        let twice = format!("{twice}{}", twice.trim_start_matches(base));
        assert!(parse(&twice, None).unwrap_err().join("; ").contains("unique"));
    }

    const GOOD: &str = r#"
id = "dev.example.hello"
name = "Hello"
version = "1.2.3"
yace = ">=0.5"
library = "lib/libhello.so"
activation = ["onCommand:hello.greet", "onLanguage:rust", "workspaceContains:package.json"]
capabilities = ["documents.read"]
events = ["documentSaved"]

[[commands]]
id = "hello.greet"
title = "Say Hello"
category = "Hello"

[[keybindings]]
key = "ctrl+alt+h"
command = "hello.greet"

[[status_items]]
id = "hello.status"

[[views]]
id = "hello.view"
title = "Hello"

[[quick_open]]
prefix = "!"
title = "Hello items"

[settings.greeting]
type = "string"
default = "Hello"
enum = ["Hello", "Hi"]

[settings.count]
type = "integer"
default = 3
minimum = 1
"#;

    #[test]
    fn a_good_manifest_parses() {
        let m = parse(GOOD, None).unwrap();
        assert_eq!(m.id, "dev.example.hello");
        assert_eq!(m.commands[0].category, "Hello");
        assert_eq!(m.status_items[0].alignment, "right");
        assert_eq!(m.views[0].location, "sidebar");
        assert_eq!(m.abi, 1);
        assert!(m.compatible("0.6.0").is_ok());
        assert!(m.compatible("0.4.2").unwrap_err().contains("needs YACE >=0.5"));
        let s = &m.settings["greeting"];
        assert!(s.validate(&toml::Value::String("Hi".into())).is_ok());
        assert!(s.validate(&toml::Value::String("Yo".into())).is_err());
        assert!(s.validate(&toml::Value::Integer(1)).is_err());
        assert!(m.settings["count"].validate(&toml::Value::Integer(0)).unwrap_err().contains("at least 1"));
    }

    #[test]
    fn every_problem_is_reported() {
        let bad = r#"
id = "Bad Id"
name = ""
version = "one"
yace = "soon"
library = "../escape.so"
activation = ["onMoon", "onCommand:"]
capabilities = ["root"]
events = ["keyPressed"]
typo = 1

[[commands]]
id = "workbench.action.quit"
title = ""

[[commands]]
id = "workbench.action.quit"
title = "Again"

[[keybindings]]
key = "ctrl+x"
command = "missing.cmd"

[[status_items]]
id = "s"
alignment = "middle"

[[views]]
id = "v"
title = "V"
location = "floating"

[[quick_open]]
prefix = ">"
title = "no"

[settings.x]
type = "string"
default = 3
"#;
        let errors = parse(bad, None).unwrap_err();
        let all = errors.join("\n");
        for needle in [
            "unknown key `typo`",
            "`id` \"Bad Id\"",
            "`name` is empty",
            "`version` \"one\"",
            "`yace` \"soon\"",
            "library `../escape.so`",
            "unknown activation event `onMoon`",
            "unknown activation event `onCommand:`",
            "unknown capability `root`",
            "unknown event `keyPressed`",
            "must be namespaced",
            "declared twice",
            "has no title",
            "refers to `missing.cmd`",
            "alignment must be",
            "location must be",
            "quick open prefix `>`",
            "setting `x`: default expected string",
        ] {
            assert!(all.contains(needle), "missing {needle:?} in:\n{all}");
        }
    }

    #[test]
    fn files_must_exist_and_types_must_match() {
        let d = tempfile::tempdir().unwrap();
        let errors = parse(GOOD, Some(d.path())).unwrap_err();
        assert_eq!(errors, vec!["library `lib/libhello.so` does not exist".to_string()]);
        assert!(parse("id = 3", None).unwrap_err().iter().any(|e| e.contains("invalid type")));
        assert!(parse("[broken", None).unwrap_err()[0].starts_with("invalid TOML"));
        // Data-only plugins can't declare activation.
        let data_only = "id = \"a.b\"\nname = \"A\"\nversion = \"1.0.0\"\nactivation = [\"*\"]\n";
        assert!(parse(data_only, None).unwrap_err()[0].contains("needs a `library`"));
    }

    #[test]
    fn compatibility() {
        let mut m = parse(GOOD, None).unwrap();
        m.os = vec!["plan9".into()];
        assert!(m.compatible("1.0.0").unwrap_err().contains("plan9"));
        m.os.clear();
        m.arch = vec!["sparc".into()];
        assert!(m.compatible("1.0.0").unwrap_err().contains("sparc"));
        m.arch.clear();
        m.abi = 2;
        assert!(m.compatible("1.0.0").unwrap_err().contains("plugin ABI 2"));
        m.abi = 1;
        assert!(m.compatible("0.6.0-rc.1").is_ok());
        assert_eq!(safe_relative("a/./b.so"), Some(PathBuf::from("a/./b.so")));
        assert_eq!(safe_relative("/etc/passwd"), None);
        assert_eq!(safe_relative("a/../../b"), None);
    }
}
