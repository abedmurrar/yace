//! VS Code colour theme import (spec §16.7): reads a theme's JSON (data
//! only, no extension code runs) and writes a YACE theme file.

use std::path::{Path, PathBuf};

use ratatui::style::Modifier;
use serde_json::Value as J;
use toml::{Table, Value};
use yace_syntax::Scope;

use super::{
    Theme,
    schema::{ALIASES, COLOR_KEYS, parse_color},
};

/// Representative TextMate scopes for each YACE scope, most typical first.
/// A theme rule applies when its selector is a prefix of one of these.
const TM: &[(Scope, &[&str])] = &[
    (Scope::Comment, &["comment.line", "comment.block", "punctuation.definition.comment"]),
    (Scope::String, &["string.quoted.double", "string.quoted", "string"]),
    (Scope::StringSpecial, &["string.regexp", "string.other.symbol"]),
    (Scope::Escape, &["constant.character.escape"]),
    (Scope::Character, &["constant.character", "string.quoted.single"]),
    (Scope::Number, &["constant.numeric"]),
    (Scope::Boolean, &["constant.language.boolean", "constant.language"]),
    (Scope::Constant, &["variable.other.constant", "constant.other", "variable.other.enummember"]),
    (Scope::ConstantBuiltin, &["constant.language.null", "constant.language"]),
    (Scope::Keyword, &["storage.type", "storage.modifier", "keyword.other", "keyword", "storage"]),
    (Scope::KeywordControl, &["keyword.control.flow", "keyword.control.conditional", "keyword.control"]),
    (Scope::Operator, &["keyword.operator"]),
    (Scope::Punctuation, &["punctuation.separator", "punctuation.terminator", "punctuation"]),
    (Scope::PunctuationBracket, &["punctuation.section.block", "meta.brace", "punctuation.section", "punctuation"]),
    (Scope::Function, &["entity.name.function", "support.function", "meta.function-call"]),
    (Scope::FunctionBuiltin, &["support.function.builtin", "support.function", "entity.name.function"]),
    (Scope::FunctionMethod, &["entity.name.function.member", "entity.name.function.method", "entity.name.function"]),
    (
        Scope::Macro,
        &[
            "entity.name.function.macro",
            "entity.name.function.preprocessor",
            "support.function.macro",
            "entity.name.function",
        ],
    ),
    (
        Scope::Type,
        &["entity.name.type.class", "entity.name.type", "entity.name.class", "support.class", "support.type"],
    ),
    (
        Scope::TypeBuiltin,
        &[
            "support.type.primitive",
            "storage.type.primitive",
            "support.type.builtin",
            "support.type",
            "entity.name.type",
        ],
    ),
    (
        Scope::Constructor,
        &["entity.name.function.constructor", "entity.name.type.class", "entity.name.type", "support.class"],
    ),
    (Scope::Variable, &["variable.other.readwrite", "variable.other", "variable"]),
    (Scope::VariableBuiltin, &["variable.language.this", "variable.language", "variable"]),
    (Scope::VariableParameter, &["variable.parameter", "variable"]),
    (
        Scope::Property,
        &[
            "variable.other.property",
            "variable.other.object.property",
            "support.type.property-name",
            "meta.object-literal.key",
            "variable.other.member",
            "variable",
        ],
    ),
    (Scope::Attribute, &["entity.other.attribute-name", "meta.attribute", "meta.decorator"]),
    (Scope::Tag, &["entity.name.tag"]),
    (
        Scope::Namespace,
        &["entity.name.namespace", "entity.name.type.module", "entity.name.type.namespace", "entity.name.type"],
    ),
    (Scope::Label, &["entity.name.label", "entity.name"]),
    (Scope::Heading, &["markup.heading", "entity.name.section"]),
    (Scope::Bold, &["markup.bold"]),
    (Scope::Italic, &["markup.italic"]),
    (Scope::Link, &["markup.underline.link", "string.other.link", "markup.link"]),
    (Scope::Raw, &["markup.inline.raw", "markup.fenced_code", "markup.raw"]),
    (Scope::ListMarker, &["punctuation.definition.list.begin", "markup.list"]),
    (Scope::Embedded, &["meta.embedded", "source"]),
];

/// The resolved style a theme's `tokenColors` give one YACE scope.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct TokenStyle {
    pub foreground: Option<String>,
    /// `bold`, `italic`, `underline` in any combination ("" clears).
    pub font_style: Option<String>,
}

fn selector_matches(selector: &str, scope: &str) -> Option<usize> {
    // Descendant selectors ("meta.x entity.y"): the last part names the
    // token; exclusions ("- x") are ignored.
    let sel = selector.split(" - ").next()?.split_whitespace().last()?;
    let ok = scope == sel || (scope.starts_with(sel) && scope.as_bytes().get(sel.len()) == Some(&b'.'));
    ok.then(|| sel.split('.').count())
}

/// Resolve `tokenColors` (TextMate rules) onto YACE scopes: per property,
/// the most specific matching selector wins; later rules win ties.
pub fn token_styles(rules: &[J]) -> Vec<(Scope, TokenStyle)> {
    let mut out = Vec::new();
    for (scope, reps) in TM {
        let mut best_fg: Option<((usize, usize, usize), String)> = None;
        let mut best_fs: Option<((usize, usize, usize), String)> = None;
        for (rep_rank, rep) in reps.iter().enumerate() {
            for (i, rule) in rules.iter().enumerate() {
                let Some(settings) = rule.get("settings") else { continue };
                let selectors: Vec<String> = match rule.get("scope") {
                    Some(J::String(s)) => s.split(',').map(|s| s.trim().to_string()).collect(),
                    Some(J::Array(a)) => a.iter().filter_map(J::as_str).map(|s| s.trim().to_string()).collect(),
                    _ => continue,
                };
                for sel in &selectors {
                    let Some(spec) = selector_matches(sel, rep) else { continue };
                    // Earlier (more typical) representatives beat later ones.
                    let key = (usize::MAX - rep_rank, spec, i);
                    if let Some(fg) = settings.get("foreground").and_then(J::as_str)
                        && best_fg.as_ref().is_none_or(|(k, _)| key > *k)
                    {
                        best_fg = Some((key, fg.to_string()));
                    }
                    if let Some(fs) = settings.get("fontStyle").and_then(J::as_str)
                        && best_fs.as_ref().is_none_or(|(k, _)| key > *k)
                    {
                        best_fs = Some((key, fs.to_string()));
                    }
                }
            }
        }
        if best_fg.is_some() || best_fs.is_some() {
            out.push((*scope, TokenStyle { foreground: best_fg.map(|b| b.1), font_style: best_fs.map(|b| b.1) }));
        }
    }
    out
}

fn mods_of(font_style: &str) -> Modifier {
    let mut m = Modifier::empty();
    for w in font_style.split_whitespace() {
        match w {
            "bold" => m |= Modifier::BOLD,
            "italic" => m |= Modifier::ITALIC,
            "underline" => m |= Modifier::UNDERLINED,
            _ => {}
        }
    }
    m
}

/// `editor.tokenColorCustomizations.textMateRules` applied to a theme.
pub fn apply_textmate_rules(theme: &mut Theme, rules: &[Value]) {
    let json: Vec<J> = rules.iter().filter_map(|r| serde_json::to_value(r).ok()).collect();
    for (scope, st) in token_styles(&json) {
        let i = scope as usize;
        if let Some(fg) = st.foreground.as_deref().and_then(|c| parse_color(c, theme.bg).ok()) {
            theme.syntax[i] = fg;
        }
        if let Some(fs) = &st.font_style {
            theme.syntax_mods[i] = mods_of(fs);
        }
    }
}

/// A colour theme contributed by an installed VS Code extension.
#[derive(Debug, Clone, PartialEq)]
pub struct VsTheme {
    pub label: String,
    pub path: PathBuf,
    /// `vs-dark`, `vs`, `hc-black`, `hc-light`.
    pub ui_theme: String,
    /// The extension's folder name.
    pub extension: String,
}

/// Extension folders of VS Code and its common forks, plus the themes
/// bundled with VS Code itself.
pub fn extension_dirs(home: &Path) -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = [".vscode", ".vscode-server", ".vscode-oss", ".cursor", ".windsurf"]
        .iter()
        .map(|d| home.join(d).join("extensions"))
        .collect();
    for app in [
        "/usr/share/code/resources/app/extensions",
        "/usr/lib/code/extensions",
        "/snap/code/current/usr/share/code/resources/app/extensions",
        "/opt/visual-studio-code/resources/app/extensions",
    ] {
        dirs.push(PathBuf::from(app));
    }
    dirs
}

/// Every theme contributed by an extension under `dirs`.
pub fn discover(dirs: &[PathBuf]) -> Vec<VsTheme> {
    let mut out = Vec::new();
    for dir in dirs {
        let Ok(rd) = std::fs::read_dir(dir) else { continue };
        let mut exts: Vec<PathBuf> = rd.flatten().map(|e| e.path()).filter(|p| p.is_dir()).collect();
        exts.sort();
        for ext in exts {
            let Ok(text) = std::fs::read_to_string(ext.join("package.json")) else { continue };
            let Ok(pkg) = yace_config::parse_jsonc(&text) else { continue };
            let Some(themes) = pkg.pointer("/contributes/themes").and_then(J::as_array) else { continue };
            let nls = std::fs::read_to_string(ext.join("package.nls.json"))
                .ok()
                .and_then(|t| yace_config::parse_jsonc(&t).ok());
            for t in themes {
                let Some(rel) = t.get("path").and_then(J::as_str) else { continue };
                let mut label = t.get("label").or_else(|| t.get("id")).and_then(J::as_str).unwrap_or(rel).to_string();
                // "%key%" labels come from package.nls.json.
                if let Some(key) = label.strip_prefix('%').and_then(|l| l.strip_suffix('%'))
                    && let Some(v) = nls.as_ref().and_then(|n| n.get(key)).and_then(J::as_str)
                {
                    label = v.to_string();
                }
                out.push(VsTheme {
                    label,
                    path: ext.join(rel),
                    ui_theme: t.get("uiTheme").and_then(J::as_str).unwrap_or("vs-dark").to_string(),
                    extension: ext.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
                });
            }
        }
    }
    out.sort_by_key(|a| a.label.to_lowercase());
    out.dedup_by(|a, b| a.label == b.label && a.path == b.path);
    out
}

/// A theme JSON with its `include` chain merged (base first).
fn load_json(path: &Path, depth: usize) -> Result<J, String> {
    if depth > 8 {
        return Err("`include` chain too deep".into());
    }
    let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let json = yace_config::parse_jsonc(&text).map_err(|e| format!("{}: {e}", path.display()))?;
    let Some(include) = json.get("include").and_then(J::as_str).map(str::to_string) else { return Ok(json) };
    let base_path = path.parent().unwrap_or(Path::new(".")).join(include);
    let mut base = load_json(&base_path, depth + 1)?;
    // Merge: colors by key, tokenColors appended, semanticTokenColors by key.
    for key in ["colors", "semanticTokenColors"] {
        if let Some(J::Object(child)) = json.get(key).cloned() {
            let entry = base.as_object_mut().map(|o| o.entry(key).or_insert_with(|| J::Object(Default::default())));
            if let Some(J::Object(b)) = entry {
                b.extend(child);
            }
        }
    }
    if let Some(J::Array(child)) = json.get("tokenColors").cloned() {
        if let Some(J::Array(b)) = base.get_mut("tokenColors") {
            b.extend(child);
        } else if let Some(o) = base.as_object_mut() {
            o.insert("tokenColors".into(), J::Array(child));
        }
    }
    for key in ["name", "type"] {
        if let Some(v) = json.get(key).cloned()
            && let Some(o) = base.as_object_mut()
        {
            o.insert(key.into(), v);
        }
    }
    Ok(base)
}

/// Convert a VS Code theme file to YACE theme TOML. Returns the file text
/// and notes about what could not be converted.
pub fn convert(path: &Path, label: &str, ui_theme: &str) -> Result<(String, Vec<String>), String> {
    let mut warnings = Vec::new();
    let json = load_json(path, 0)?;
    let light = matches!(ui_theme, "vs" | "hc-light")
        || json.get("type").and_then(J::as_str).is_some_and(|t| t == "light" || t == "hcLight");
    let mut doc = Table::new();
    doc.insert("name".into(), Value::String(label.to_string()));
    doc.insert("type".into(), Value::String(if light { "light" } else { "dark" }.into()));

    let mut colors = Table::new();
    if let Some(J::Object(c)) = json.get("colors") {
        for k in COLOR_KEYS {
            if let Some(v) = c.get(k.id).and_then(J::as_str) {
                colors.insert(k.id.into(), Value::String(v.into()));
            }
        }
        for (alias, canon) in ALIASES {
            if !colors.contains_key(*canon)
                && let Some(v) = c.get(*alias).and_then(J::as_str)
            {
                colors.insert((*canon).into(), Value::String(v.into()));
            }
        }
    }
    let rules: Vec<J> = match json.get("tokenColors") {
        Some(J::Array(a)) => a.clone(),
        Some(J::String(s)) => {
            warnings.push(format!(
                "tokenColors from `{s}` (a .tmTheme file) are not supported; syntax colours use the base theme"
            ));
            Vec::new()
        }
        _ => Vec::new(),
    };
    // The global rule (no scope) sets the default text colour.
    if !colors.contains_key("editor.foreground")
        && let Some(fg) = rules
            .iter()
            .find(|r| r.get("scope").is_none())
            .and_then(|r| r.pointer("/settings/foreground"))
            .and_then(J::as_str)
    {
        colors.insert("editor.foreground".into(), Value::String(fg.into()));
    }
    let mut syntax = Table::new();
    for (scope, st) in token_styles(&rules) {
        let mut t = Table::new();
        if let Some(fg) = st.foreground {
            t.insert("fg".into(), Value::String(fg));
        }
        if let Some(fs) = st.font_style {
            let m = mods_of(&fs);
            t.insert("bold".into(), Value::Boolean(m.contains(Modifier::BOLD)));
            t.insert("italic".into(), Value::Boolean(m.contains(Modifier::ITALIC)));
            t.insert("underline".into(), Value::Boolean(m.contains(Modifier::UNDERLINED)));
        }
        syntax.insert(scope.name().into(), Value::Table(t));
    }
    if let Some(fg) = colors.get("editor.foreground").cloned() {
        for s in [Scope::None, Scope::Operator, Scope::Punctuation, Scope::PunctuationBracket, Scope::Embedded] {
            syntax.entry(s.name()).or_insert_with(|| fg.clone());
        }
    }
    let mut semantic = Table::new();
    if let Some(J::Object(sem)) = json.get("semanticTokenColors") {
        for (k, v) in sem {
            let entry = match v {
                J::String(c) => Some(Value::String(c.clone())),
                J::Object(o) => {
                    let mut t = Table::new();
                    if let Some(fg) = o.get("foreground").and_then(J::as_str) {
                        t.insert("fg".into(), Value::String(fg.into()));
                    }
                    for m in ["bold", "italic", "underline"] {
                        if let Some(b) = o.get(m).and_then(J::as_bool) {
                            t.insert(m.into(), Value::Boolean(b));
                        }
                    }
                    (!t.is_empty()).then_some(Value::Table(t))
                }
                _ => None,
            };
            if let Some(e) = entry {
                semantic.insert(k.clone(), e);
            }
        }
    }
    doc.insert("colors".into(), Value::Table(colors));
    doc.insert("syntax".into(), Value::Table(syntax));
    if !semantic.is_empty() {
        doc.insert("semantic".into(), Value::Table(semantic));
    }
    let body = toml::to_string(&doc).map_err(|e| e.to_string())?;
    let header = format!(
        "# Imported from {} by YACE (colour data only; no extension code runs).\n# Keys missing here come from {}.\n",
        path.display(),
        if light { "YACE Light" } else { "YACE Dark" }
    );
    Ok((header + &body, warnings))
}

/// A file name for an imported theme (`Monokai Pro` → `monokai-pro`).
pub fn slug(label: &str) -> String {
    let mut s = String::new();
    for c in label.chars() {
        if c.is_ascii_alphanumeric() {
            s.push(c.to_ascii_lowercase());
        } else if !s.ends_with('-') && !s.is_empty() {
            s.push('-');
        }
    }
    while s.ends_with('-') {
        s.pop();
    }
    if s.is_empty() { "imported-theme".into() } else { s }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::{Catalog, Overrides, load_theme};
    use ratatui::style::Color;

    const DARK_PLUS_LIKE: &str = r##"{
      // a comment, as VS Code allows
      "name": "Sample Dark",
      "include": "./base.json",
      "colors": { "editor.background": "#101010", "quickInput.background": "#202020", "unknown.key": "#ffffff" },
      "tokenColors": [
        { "settings": { "foreground": "#d4d4d4" } },
        { "scope": "comment", "settings": { "foreground": "#6a9955", "fontStyle": "italic" } },
        { "scope": ["keyword", "storage"], "settings": { "foreground": "#569cd6" } },
        { "scope": "keyword.control", "settings": { "foreground": "#c586c0" } },
        { "scope": "entity.name.function, support.function", "settings": { "foreground": "#dcdcaa" } },
        { "scope": "meta.function-call entity.name.function", "settings": { "fontStyle": "bold" } },
        { "scope": "variable", "settings": { "foreground": "#9cdcfe" } },
        { "scope": "entity.name.tag - meta.x", "settings": { "foreground": "#123456" } },
      ],
      "semanticTokenColors": { "parameter": "#abcdef", "property": { "foreground": "#fedcba", "italic": true } }
    }"##;
    const BASE: &str = r##"{ "colors": { "statusBar.background": "#007acc", "editor.background": "#000000" },
      "tokenColors": [ { "scope": "string", "settings": { "foreground": "#ce9178" } } ] }"##;

    #[test]
    fn textmate_resolution() {
        let rules = yace_config::parse_jsonc(DARK_PLUS_LIKE).unwrap()["tokenColors"].as_array().unwrap().clone();
        let st: std::collections::HashMap<Scope, TokenStyle> = token_styles(&rules).into_iter().collect();
        assert_eq!(st[&Scope::Comment].foreground.as_deref(), Some("#6a9955"));
        assert_eq!(st[&Scope::Comment].font_style.as_deref(), Some("italic"));
        assert_eq!(st[&Scope::Keyword].foreground.as_deref(), Some("#569cd6"));
        assert_eq!(st[&Scope::KeywordControl].foreground.as_deref(), Some("#c586c0"));
        assert_eq!(st[&Scope::Function].foreground.as_deref(), Some("#dcdcaa"));
        assert_eq!(st[&Scope::Function].font_style.as_deref(), Some("bold"));
        assert_eq!(st[&Scope::VariableParameter].foreground.as_deref(), Some("#9cdcfe"));
        assert_eq!(st[&Scope::Tag].foreground.as_deref(), Some("#123456"));
        assert!(!st.contains_key(&Scope::Number));
    }

    #[test]
    fn converts_a_theme_with_includes() {
        let d = tempfile::tempdir().unwrap();
        std::fs::write(d.path().join("sample.json"), DARK_PLUS_LIKE).unwrap();
        std::fs::write(d.path().join("base.json"), BASE).unwrap();
        let (text, warnings) = convert(&d.path().join("sample.json"), "Sample Dark", "vs-dark").unwrap();
        assert!(warnings.is_empty(), "{warnings:?}");
        let themes = d.path().join("themes");
        std::fs::create_dir(&themes).unwrap();
        std::fs::write(themes.join(format!("{}.toml", slug("Sample Dark"))), &text).unwrap();
        let (cat, errors) = Catalog::with_user_dir(&themes);
        assert!(errors.is_empty());
        let (t, w) = load_theme(&cat, "Sample Dark", &Overrides::default()).unwrap();
        assert!(w.is_empty(), "{w:?}\n{text}");
        assert_eq!(t.bg, Color::Rgb(0x10, 0x10, 0x10));
        assert_eq!(t.status_bg, Color::Rgb(0, 0x7a, 0xcc), "from the included base");
        assert_eq!(t.overlay_bg, Color::Rgb(0x20, 0x20, 0x20), "alias");
        assert_eq!(t.syntax[Scope::String as usize], Color::Rgb(0xce, 0x91, 0x78), "base tokenColors");
        assert_eq!(t.syntax[Scope::Comment as usize], Color::Rgb(0x6a, 0x99, 0x55));
        assert!(t.syntax_mods[Scope::Comment as usize].contains(Modifier::ITALIC));
        assert_eq!(t.fg, Color::Rgb(0xd4, 0xd4, 0xd4), "global rule");
        assert_eq!(t.semantic.len(), 2);
        assert_eq!(slug("Monokai Pro (Filter: Spectrum)"), "monokai-pro-filter-spectrum");
    }

    #[test]
    fn discovers_extension_themes() {
        let d = tempfile::tempdir().unwrap();
        let ext = d.path().join("ext").join("someone.cool-theme-1.0.0");
        std::fs::create_dir_all(ext.join("themes")).unwrap();
        std::fs::write(
            ext.join("package.json"),
            r#"{ "contributes": { "themes": [ { "label": "%t%", "uiTheme": "vs", "path": "./themes/t.json" } ] } }"#,
        )
        .unwrap();
        std::fs::write(ext.join("package.nls.json"), r#"{ "t": "Cool Light" }"#).unwrap();
        let found = discover(&[d.path().join("ext"), d.path().join("missing")]);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].label, "Cool Light");
        assert_eq!(found[0].ui_theme, "vs");
        assert!(found[0].path.ends_with("themes/t.json"));
    }
}
