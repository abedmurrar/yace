//! The language registry (spec §12.2): detection, grammars, queries and
//! editing rules. Grammars are compiled in but only *constructed* when a
//! file of that language is opened; queries are compiled lazily too.

use std::{
    collections::HashMap,
    path::Path,
    sync::{OnceLock, RwLock},
};

use tree_sitter::{Language, Query};

/// Editing behaviour that does not need a parser.
#[derive(Debug, Clone, Copy)]
pub struct EditingRules {
    pub line_comment: Option<&'static str>,
    pub block_comment: Option<(&'static str, &'static str)>,
    /// Bracket pairs for matching and auto-indent.
    pub brackets: &'static [(char, char)],
    /// Auto-closing pairs (brackets + quotes).
    pub auto_close: &'static [(char, char)],
    /// A line ending with one of these (ignoring whitespace) indents the next.
    pub indent_after: &'static [char],
}

const BRACES: &[(char, char)] = &[('(', ')'), ('[', ']'), ('{', '}')];
const C_LIKE_CLOSE: &[(char, char)] = &[('(', ')'), ('[', ']'), ('{', '}'), ('"', '"'), ('\'', '\''), ('`', '`')];
const NO_BACKTICK: &[(char, char)] = &[('(', ')'), ('[', ']'), ('{', '}'), ('"', '"'), ('\'', '\'')];
const MARKUP: &[(char, char)] = &[('<', '>'), ('(', ')'), ('[', ']'), ('{', '}'), ('"', '"'), ('\'', '\'')];

pub struct LanguageConfig {
    pub id: &'static str,
    pub name: &'static str,
    pub extensions: &'static [&'static str],
    pub filenames: &'static [&'static str],
    /// Interpreter names matched against a `#!` line.
    pub shebangs: &'static [&'static str],
    pub rules: EditingRules,
    grammar: Option<fn() -> Language>,
    /// Highlight query sources, base first (later patterns win).
    highlights: &'static [&'static str],
    injections: &'static [&'static str],
    highlight_query: OnceLock<Option<Query>>,
    injection_query: OnceLock<Option<Query>>,
    language: OnceLock<Option<Language>>,
}

impl std::fmt::Debug for LanguageConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "LanguageConfig({})", self.id)
    }
}

/// A grammar loaded at run time (from a grammar package / plugin).
pub struct LoadedGrammar {
    pub language: Language,
    pub highlights: Option<Query>,
    pub injections: Option<Query>,
}

fn external() -> &'static RwLock<HashMap<String, &'static LoadedGrammar>> {
    static EXTERNAL: OnceLock<RwLock<HashMap<String, &'static LoadedGrammar>>> = OnceLock::new();
    EXTERNAL.get_or_init(Default::default)
}

fn external_for(id: &str) -> Option<&'static LoadedGrammar> {
    external().read().unwrap_or_else(|e| e.into_inner()).get(id).copied()
}

/// Register a grammar for language `id` (built-in or plugin-declared).
/// Checks the grammar's ABI version and compiles its queries; a broken
/// highlight query is an error (the grammar would show no colours).
pub fn register_grammar(id: &str, language: Language, highlights: &str, injections: &str) -> Result<(), String> {
    let v = language.abi_version();
    if !(tree_sitter::MIN_COMPATIBLE_LANGUAGE_VERSION..=tree_sitter::LANGUAGE_VERSION).contains(&v) {
        return Err(format!(
            "grammar ABI {v} is not supported (this YACE supports {}–{})",
            tree_sitter::MIN_COMPATIBLE_LANGUAGE_VERSION,
            tree_sitter::LANGUAGE_VERSION
        ));
    }
    let compile = |src: &str, what: &str| -> Result<Option<Query>, String> {
        if src.trim().is_empty() {
            return Ok(None);
        }
        Query::new(&language, src).map(Some).map_err(|e| format!("{what} query: {e}"))
    };
    let highlights = compile(highlights, "highlights")?;
    let injections = compile(injections, "injections")?;
    let g: &'static LoadedGrammar = Box::leak(Box::new(LoadedGrammar { language, highlights, injections }));
    external().write().unwrap_or_else(|e| e.into_inner()).insert(id.to_string(), g);
    Ok(())
}

/// Whether a grammar for `id` was registered at run time.
pub fn has_external_grammar(id: &str) -> bool {
    external_for(id).is_some()
}

impl LanguageConfig {
    /// The tree-sitter grammar (constructed on first use): built in, or
    /// registered at run time.
    pub fn language(&self) -> Option<&Language> {
        if self.grammar.is_some() {
            return self.language.get_or_init(|| self.grammar.map(|g| g())).as_ref();
        }
        external_for(self.id).map(|g| &g.language)
    }

    fn build(&self, sources: &[&str]) -> Option<Query> {
        let lang = self.language()?;
        let src = sources.concat();
        if src.trim().is_empty() {
            return None;
        }
        match Query::new(lang, &src) {
            Ok(q) => Some(q),
            Err(e) => {
                // A broken query must never take the editor down; it only
                // costs the colours.
                eprintln_once(self.id, &e.to_string());
                None
            }
        }
    }

    pub fn highlight_query(&self) -> Option<&Query> {
        if self.grammar.is_none() {
            return external_for(self.id).and_then(|g| g.highlights.as_ref());
        }
        self.highlight_query.get_or_init(|| self.build(self.highlights)).as_ref()
    }

    pub fn injection_query(&self) -> Option<&Query> {
        if self.grammar.is_none() {
            return external_for(self.id).and_then(|g| g.injections.as_ref());
        }
        self.injection_query.get_or_init(|| self.build(self.injections)).as_ref()
    }

    pub fn has_grammar(&self) -> bool {
        self.grammar.is_some() || external_for(self.id).is_some()
    }

    /// Built-in languages whose grammar ships in the `yace-grammars`
    /// package rather than in the binary (hybrid grammars, decision D29).
    pub fn grammar_is_packaged(&self) -> bool {
        PACKAGED.contains(&self.id)
    }
}

/// Languages whose grammars are in the `yace-grammars` package.
pub const PACKAGED: &[&str] = &["c", "cpp", "go", "java", "ruby", "scss", "sql", "dockerfile"];

/// A language declared by a plugin (`[[languages]]` in its manifest).
#[derive(Debug, Clone, Default)]
pub struct LanguageDef {
    pub id: String,
    pub name: String,
    pub extensions: Vec<String>,
    pub filenames: Vec<String>,
    pub shebangs: Vec<String>,
    pub line_comment: Option<String>,
    pub block_comment: Option<(String, String)>,
    pub brackets: Vec<(char, char)>,
}

fn extra() -> &'static RwLock<Vec<&'static LanguageConfig>> {
    static EXTRA: OnceLock<RwLock<Vec<&'static LanguageConfig>>> = OnceLock::new();
    EXTRA.get_or_init(Default::default)
}

fn leak_str(s: &str) -> &'static str {
    Box::leak(s.to_string().into_boxed_str())
}

fn leak_strs(v: &[String]) -> &'static [&'static str] {
    Box::leak(v.iter().map(|s| leak_str(s)).collect::<Vec<_>>().into_boxed_slice())
}

/// Add a language (once per id; built-in ids are ignored). Its grammar, if
/// any, comes from `register_grammar`.
pub fn register_language(def: &LanguageDef) -> bool {
    if by_id(&def.id).is_some() {
        return false;
    }
    let brackets: &'static [(char, char)] =
        if def.brackets.is_empty() { BRACES } else { Box::leak(def.brackets.clone().into_boxed_slice()) };
    let mut auto: Vec<(char, char)> = brackets.to_vec();
    auto.extend([('"', '"'), ('\'', '\'')]);
    let cfg = LanguageConfig {
        id: leak_str(&def.id),
        name: leak_str(&def.name),
        extensions: leak_strs(&def.extensions),
        filenames: leak_strs(&def.filenames),
        shebangs: leak_strs(&def.shebangs),
        rules: EditingRules {
            line_comment: def.line_comment.as_deref().map(leak_str),
            block_comment: def.block_comment.as_ref().map(|(a, b)| (leak_str(a), leak_str(b))),
            brackets,
            auto_close: Box::leak(auto.into_boxed_slice()),
            indent_after: Box::leak(brackets.iter().map(|b| b.0).collect::<Vec<_>>().into_boxed_slice()),
        },
        grammar: None,
        highlights: &[],
        injections: &[],
        highlight_query: OnceLock::new(),
        injection_query: OnceLock::new(),
        language: OnceLock::new(),
    };
    extra().write().unwrap_or_else(|e| e.into_inner()).push(Box::leak(Box::new(cfg)));
    true
}

/// Built-in languages, then plugin languages.
fn all_languages() -> Vec<&'static LanguageConfig> {
    let mut v: Vec<&'static LanguageConfig> = LANGUAGES.iter().collect();
    v.extend(extra().read().unwrap_or_else(|e| e.into_inner()).iter().copied());
    v
}

fn eprintln_once(_id: &str, _err: &str) {
    #[cfg(debug_assertions)]
    eprintln!("yace-syntax: query for {_id} failed: {_err}");
}

macro_rules! lang {
    ($id:literal, $name:literal, ext: $ext:expr, files: $files:expr, shebang: $sb:expr, rules: $rules:expr, grammar: $g:expr, hl: $hl:expr, inj: $inj:expr) => {
        LanguageConfig {
            id: $id,
            name: $name,
            extensions: $ext,
            filenames: $files,
            shebangs: $sb,
            rules: $rules,
            grammar: $g,
            highlights: $hl,
            injections: $inj,
            highlight_query: OnceLock::new(),
            injection_query: OnceLock::new(),
            language: OnceLock::new(),
        }
    };
}

/// A grammar compiled in only with a cargo feature.
macro_rules! g {
    ($feat:literal, $f:expr) => {{
        #[cfg(feature = "all-grammars")]
        let g: Option<fn() -> Language> = Some($f);
        #[cfg(not(feature = "all-grammars"))]
        let g: Option<fn() -> Language> = None;
        g
    }};
}

/// Queries of a feature-gated grammar.
macro_rules! q {
    ($($e:expr),*) => {{
        #[cfg(feature = "all-grammars")]
        let q: &'static [&'static str] = &[$($e),*];
        #[cfg(not(feature = "all-grammars"))]
        let q: &'static [&'static str] = &[];
        q
    }};
}

const fn rules(
    line: Option<&'static str>,
    block: Option<(&'static str, &'static str)>,
    auto_close: &'static [(char, char)],
    indent_after: &'static [char],
) -> EditingRules {
    EditingRules { line_comment: line, block_comment: block, brackets: BRACES, auto_close, indent_after }
}

const C_RULES: EditingRules = rules(Some("//"), Some(("/*", "*/")), C_LIKE_CLOSE, &['{', '[', '(']);
const HASH_RULES: EditingRules = rules(Some("#"), None, NO_BACKTICK, &['{', '[', '(']);

fn ts() -> Language {
    tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into()
}
fn tsx() -> Language {
    tree_sitter_typescript::LANGUAGE_TSX.into()
}

/// Every built-in language. Order matters only for display.
pub static LANGUAGES: std::sync::LazyLock<Vec<LanguageConfig>> = std::sync::LazyLock::new(|| {
    vec![
        lang!("rust", "Rust", ext: &["rs"], files: &[], shebang: &[], rules: C_RULES,
            grammar: Some(|| tree_sitter_rust::LANGUAGE.into()),
            hl: &[tree_sitter_rust::HIGHLIGHTS_QUERY], inj: &[tree_sitter_rust::INJECTIONS_QUERY]),
        lang!("typescript", "TypeScript", ext: &["ts", "mts", "cts"], files: &[], shebang: &["deno", "ts-node"], rules: C_RULES,
            grammar: Some(ts),
            hl: &[tree_sitter_javascript::HIGHLIGHT_QUERY, tree_sitter_typescript::HIGHLIGHTS_QUERY],
            inj: &[tree_sitter_javascript::INJECTIONS_QUERY]),
        lang!("typescriptreact", "TypeScript JSX", ext: &["tsx"], files: &[], shebang: &[], rules: C_RULES,
            grammar: Some(tsx),
            hl: &[tree_sitter_javascript::HIGHLIGHT_QUERY, tree_sitter_javascript::JSX_HIGHLIGHT_QUERY, tree_sitter_typescript::HIGHLIGHTS_QUERY],
            inj: &[tree_sitter_javascript::INJECTIONS_QUERY]),
        lang!("javascript", "JavaScript", ext: &["js", "mjs", "cjs"], files: &[], shebang: &["node", "bun"], rules: C_RULES,
            grammar: Some(|| tree_sitter_javascript::LANGUAGE.into()),
            hl: &[tree_sitter_javascript::HIGHLIGHT_QUERY, tree_sitter_javascript::JSX_HIGHLIGHT_QUERY],
            inj: &[tree_sitter_javascript::INJECTIONS_QUERY]),
        lang!("javascriptreact", "JavaScript JSX", ext: &["jsx"], files: &[], shebang: &[], rules: C_RULES,
            grammar: Some(|| tree_sitter_javascript::LANGUAGE.into()),
            hl: &[tree_sitter_javascript::HIGHLIGHT_QUERY, tree_sitter_javascript::JSX_HIGHLIGHT_QUERY],
            inj: &[tree_sitter_javascript::INJECTIONS_QUERY]),
        lang!("json", "JSON", ext: &["json", "jsonl", "geojson", "webmanifest"], files: &[".prettierrc", ".babelrc"], shebang: &[],
            rules: rules(None, None, NO_BACKTICK, &['{', '[']),
            grammar: Some(|| tree_sitter_json::LANGUAGE.into()), hl: &[tree_sitter_json::HIGHLIGHTS_QUERY], inj: &[]),
        lang!("jsonc", "JSON with Comments", ext: &["jsonc", "code-workspace"],
            files: &["tsconfig.json", "jsconfig.json", "settings.json", "launch.json", "tasks.json", "devcontainer.json", ".eslintrc.json"],
            shebang: &[], rules: rules(Some("//"), Some(("/*", "*/")), NO_BACKTICK, &['{', '[']),
            grammar: Some(|| tree_sitter_json::LANGUAGE.into()), hl: &[tree_sitter_json::HIGHLIGHTS_QUERY], inj: &[]),
        lang!("yaml", "YAML", ext: &["yaml", "yml"], files: &[".clang-format"], shebang: &[],
            rules: rules(Some("#"), None, NO_BACKTICK, &[':', '[', '{']),
            grammar: Some(|| tree_sitter_yaml::LANGUAGE.into()), hl: &[tree_sitter_yaml::HIGHLIGHTS_QUERY], inj: &[]),
        lang!("toml", "TOML", ext: &["toml"], files: &["Cargo.lock", "Pipfile"], shebang: &[], rules: HASH_RULES,
            grammar: Some(|| tree_sitter_toml_ng::LANGUAGE.into()), hl: &[tree_sitter_toml_ng::HIGHLIGHTS_QUERY], inj: &[]),
        lang!("html", "HTML", ext: &["html", "htm", "xhtml"], files: &[], shebang: &[],
            rules: EditingRules { line_comment: None, block_comment: Some(("<!--", "-->")), brackets: BRACES, auto_close: MARKUP, indent_after: &['>', '{'] },
            grammar: Some(|| tree_sitter_html::LANGUAGE.into()), hl: &[tree_sitter_html::HIGHLIGHTS_QUERY], inj: &[tree_sitter_html::INJECTIONS_QUERY]),
        lang!("css", "CSS", ext: &["css"], files: &[], shebang: &[],
            rules: rules(None, Some(("/*", "*/")), NO_BACKTICK, &['{', '(']),
            grammar: Some(|| tree_sitter_css::LANGUAGE.into()), hl: &[tree_sitter_css::HIGHLIGHTS_QUERY], inj: &[]),
        lang!("scss", "SCSS", ext: &["scss"], files: &[], shebang: &[],
            rules: rules(Some("//"), Some(("/*", "*/")), NO_BACKTICK, &['{', '(']),
            grammar: g!("all-grammars", tree_sitter_scss::language), hl: q!(tree_sitter_css::HIGHLIGHTS_QUERY, tree_sitter_scss::HIGHLIGHTS_QUERY), inj: &[]),
        lang!("markdown", "Markdown", ext: &["md", "markdown", "mdx"], files: &["README", "CHANGELOG"], shebang: &[],
            rules: EditingRules { line_comment: None, block_comment: Some(("<!--", "-->")), brackets: BRACES, auto_close: &[('(', ')'), ('[', ']'), ('{', '}'), ('`', '`')], indent_after: &[] },
            grammar: Some(|| tree_sitter_md::LANGUAGE.into()),
            hl: &[tree_sitter_md::HIGHLIGHT_QUERY_BLOCK], inj: &[tree_sitter_md::INJECTION_QUERY_BLOCK]),
        lang!("markdown_inline", "Markdown (inline)", ext: &[], files: &[], shebang: &[], rules: rules(None, None, &[], &[]),
            grammar: Some(|| tree_sitter_md::INLINE_LANGUAGE.into()),
            hl: &[tree_sitter_md::HIGHLIGHT_QUERY_INLINE], inj: &[tree_sitter_md::INJECTION_QUERY_INLINE]),
        lang!("python", "Python", ext: &["py", "pyi", "pyw"], files: &["SConstruct", "SConscript"], shebang: &["python", "python3", "python2"],
            rules: rules(Some("#"), None, NO_BACKTICK, &[':', '{', '[', '(']),
            grammar: Some(|| tree_sitter_python::LANGUAGE.into()), hl: &[tree_sitter_python::HIGHLIGHTS_QUERY], inj: &[]),
        lang!("go", "Go", ext: &["go"], files: &[], shebang: &[], rules: C_RULES,
            grammar: g!("all-grammars", || tree_sitter_go::LANGUAGE.into()), hl: q!(tree_sitter_go::HIGHLIGHTS_QUERY), inj: &[]),
        lang!("shellscript", "Shell Script", ext: &["sh", "bash", "zsh", "ksh", "bats", "envrc"],
            files: &[".bashrc", ".bash_profile", ".zshrc", ".profile", ".bash_aliases", ".zprofile", "PKGBUILD"],
            shebang: &["sh", "bash", "zsh", "ksh", "dash"],
            rules: rules(Some("#"), None, C_LIKE_CLOSE, &['{', '(', '[']),
            grammar: Some(|| tree_sitter_bash::LANGUAGE.into()), hl: &[tree_sitter_bash::HIGHLIGHT_QUERY], inj: &[]),
        lang!("sql", "SQL", ext: &["sql", "psql", "pgsql"], files: &[], shebang: &[],
            rules: rules(Some("--"), Some(("/*", "*/")), NO_BACKTICK, &['(']),
            grammar: g!("all-grammars", || tree_sitter_sequel::LANGUAGE.into()), hl: q!(tree_sitter_sequel::HIGHLIGHTS_QUERY), inj: &[]),
        lang!("dockerfile", "Dockerfile", ext: &["dockerfile", "containerfile"], files: &["Dockerfile", "Containerfile"], shebang: &[], rules: HASH_RULES,
            grammar: g!("all-grammars", || tree_sitter_containerfile::LANGUAGE.into()),
            hl: q!(tree_sitter_containerfile::HIGHLIGHTS_QUERY), inj: q!(tree_sitter_containerfile::INJECTIONS_QUERY)),
        lang!("c", "C", ext: &["c", "h"], files: &[], shebang: &[], rules: C_RULES,
            grammar: g!("all-grammars", || tree_sitter_c::LANGUAGE.into()), hl: q!(tree_sitter_c::HIGHLIGHT_QUERY), inj: &[]),
        lang!("cpp", "C++", ext: &["cpp", "cc", "cxx", "hpp", "hh", "hxx", "c++", "ipp"], files: &[], shebang: &[], rules: C_RULES,
            grammar: g!("all-grammars", || tree_sitter_cpp::LANGUAGE.into()), hl: q!(tree_sitter_c::HIGHLIGHT_QUERY, tree_sitter_cpp::HIGHLIGHT_QUERY), inj: &[]),
        lang!("java", "Java", ext: &["java"], files: &[], shebang: &[], rules: C_RULES,
            grammar: g!("all-grammars", || tree_sitter_java::LANGUAGE.into()), hl: q!(tree_sitter_java::HIGHLIGHTS_QUERY), inj: &[]),
        lang!("ruby", "Ruby", ext: &["rb", "rake", "gemspec", "ru"], files: &["Gemfile", "Rakefile", "Guardfile", "Vagrantfile"], shebang: &["ruby"],
            rules: rules(Some("#"), Some(("=begin", "=end")), NO_BACKTICK, &['{', '[', '(', '|']),
            grammar: g!("all-grammars", || tree_sitter_ruby::LANGUAGE.into()), hl: q!(tree_sitter_ruby::HIGHLIGHTS_QUERY), inj: &[]),
        // Languages without a built-in grammar: editing rules only.
        lang!("makefile", "Makefile", ext: &["mk", "mak"], files: &["Makefile", "makefile", "GNUmakefile"], shebang: &["make"], rules: HASH_RULES,
            grammar: None, hl: &[], inj: &[]),
        lang!("ignore", "Ignore", ext: &["gitignore", "dockerignore", "npmignore", "prettierignore", "eslintignore"],
            files: &[".gitignore", ".dockerignore", ".npmignore", ".prettierignore", ".eslintignore"], shebang: &[], rules: HASH_RULES,
            grammar: None, hl: &[], inj: &[]),
        lang!("properties", "Properties", ext: &["env", "ini", "cfg", "conf", "properties"],
            files: &[".env", ".env.local", ".env.example", ".editorconfig", ".npmrc", ".gitconfig", ".gitattributes"], shebang: &[],
            rules: HASH_RULES, grammar: None, hl: &[], inj: &[]),
        lang!("lua", "Lua", ext: &["lua"], files: &[], shebang: &["lua"],
            rules: rules(Some("--"), Some(("--[[", "]]")), NO_BACKTICK, &['{', '(']), grammar: None, hl: &[], inj: &[]),
        lang!("diff", "Diff", ext: &["diff", "patch", "rej"], files: &[], shebang: &[],
            rules: rules(Some("#"), None, &[], &[]), grammar: None, hl: &[], inj: &[]),
        lang!("plaintext", "Plain Text", ext: &["txt", "text", "log"], files: &["LICENSE", "COPYING"], shebang: &[],
            rules: rules(None, None, &[('(', ')'), ('[', ']'), ('{', '}'), ('"', '"')], &[]), grammar: None, hl: &[], inj: &[]),
    ]
});

pub fn by_id(id: &str) -> Option<&'static LanguageConfig> {
    LANGUAGES
        .iter()
        .find(|l| l.id == id)
        .or_else(|| extra().read().unwrap_or_else(|e| e.into_inner()).iter().copied().find(|l| l.id == id))
}

pub fn plain_text() -> &'static LanguageConfig {
    by_id("plaintext").expect("plaintext language")
}

/// Map an injection language name (from a query) to a registry entry.
pub fn by_injection_name(name: &str) -> Option<&'static LanguageConfig> {
    let n = name.trim().to_ascii_lowercase();
    let alias = match n.as_str() {
        "js" | "javascript" | "node" | "mjs" | "cjs" => "javascript",
        "jsx" => "javascriptreact",
        "ts" | "typescript" => "typescript",
        "tsx" => "typescriptreact",
        "rs" | "rust" => "rust",
        "py" | "python" | "python3" => "python",
        "sh" | "bash" | "shell" | "zsh" | "console" | "shellscript" => "shellscript",
        "yml" | "yaml" => "yaml",
        "md" | "markdown" => "markdown",
        "c++" | "cpp" | "cxx" => "cpp",
        "golang" | "go" => "go",
        "rb" | "ruby" => "ruby",
        "docker" | "dockerfile" | "containerfile" => "dockerfile",
        "postgres" | "postgresql" | "sql" => "sql",
        other => other,
    };
    by_id(alias).filter(|l| l.has_grammar())
}

/// Glob → language id associations (VS Code `files.associations`).
#[derive(Debug, Default, Clone)]
pub struct Associations {
    set: Option<globset::GlobSet>,
    ids: Vec<String>,
}

impl Associations {
    pub fn new(pairs: &[(String, String)]) -> Self {
        let mut b = globset::GlobSetBuilder::new();
        let mut ids = Vec::new();
        for (glob, id) in pairs {
            let g = if glob.contains('/') { glob.clone() } else { format!("**/{glob}") };
            if let Ok(gl) = globset::Glob::new(&g) {
                b.add(gl);
                ids.push(id.clone());
            }
        }
        Self { set: b.build().ok(), ids }
    }
}

/// Detect the language of a file (spec §12.2): user associations → file
/// name → extension → `#!` line → plain text.
pub fn detect(path: Option<&Path>, first_line: &str, assoc: &Associations) -> &'static LanguageConfig {
    if let Some(p) = path {
        if let Some(set) = &assoc.set
            && let Some(i) = set.matches(p).into_iter().next()
            && let Some(l) = by_id(&assoc.ids[i])
        {
            return l;
        }
        let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
        let langs = all_languages();
        if let Some(l) = langs.iter().copied().find(|l| l.filenames.contains(&name)) {
            return l;
        }
        // Dockerfile.dev, Makefile.inc, …
        if let Some(l) = langs.iter().copied().find(|l| {
            l.filenames
                .iter()
                .any(|f| name.starts_with(&format!("{f}.")) && f.chars().next().is_some_and(char::is_uppercase))
        }) {
            return l;
        }
        if let Some(ext) = p.extension().and_then(|e| e.to_str()) {
            let ext = ext.to_ascii_lowercase();
            if let Some(l) = langs.iter().copied().find(|l| l.extensions.contains(&ext.as_str())) {
                return l;
            }
        }
    }
    if (first_line.starts_with("diff --git ") || first_line.starts_with("--- a/"))
        && let Some(l) = by_id("diff")
    {
        return l;
    }
    if let Some(rest) = first_line.strip_prefix("#!") {
        let mut words = rest.split_whitespace();
        let first = words.next().unwrap_or("");
        let prog = if first.ends_with("/env") { words.find(|w| !w.starts_with('-')).unwrap_or("") } else { first };
        let prog = prog.rsplit('/').next().unwrap_or(prog);
        let base = prog.trim_end_matches(|c: char| c.is_ascii_digit() || c == '.');
        if let Some(l) = all_languages().into_iter().find(|l| l.shebangs.contains(&prog) || l.shebangs.contains(&base))
        {
            return l;
        }
    }
    plain_text()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detection() {
        let a = Associations::default();
        let d = |p: &str, first: &str| detect(Some(Path::new(p)), first, &a).id;
        assert_eq!(d("src/main.rs", ""), "rust");
        assert_eq!(d("app/page.tsx", ""), "typescriptreact");
        assert_eq!(d("a/b/index.ts", ""), "typescript");
        assert_eq!(d("Dockerfile", ""), "dockerfile");
        assert_eq!(d("Dockerfile.dev", ""), "dockerfile");
        assert_eq!(d("tsconfig.json", ""), "jsonc");
        assert_eq!(d("ws.code-workspace", ""), "jsonc");
        assert_eq!(d("package.json", ""), "json");
        assert_eq!(d("run", "#!/usr/bin/env python3"), "python");
        assert_eq!(d("deploy", "#!/bin/bash -e"), "shellscript");
        assert_eq!(d("notes", ""), "plaintext");
        assert_eq!(d(".env", ""), "properties");
        let assoc = Associations::new(&[("*.mdc".into(), "markdown".into()), ("Justfile".into(), "makefile".into())]);
        assert_eq!(detect(Some(Path::new("x/rules.mdc")), "", &assoc).id, "markdown");
        assert_eq!(detect(Some(Path::new("Justfile")), "", &assoc).id, "makefile");
    }

    #[test]
    fn every_builtin_query_compiles() {
        for l in LANGUAGES.iter().filter(|l| l.has_grammar()) {
            assert!(l.highlight_query().is_some(), "highlight query for {} failed to compile", l.id);
            if !l.injections.is_empty() {
                assert!(l.injection_query().is_some(), "injection query for {} failed to compile", l.id);
            }
        }
    }

    #[test]
    fn injection_aliases() {
        assert_eq!(by_injection_name("ts").unwrap().id, "typescript");
        assert_eq!(by_injection_name("Bash").unwrap().id, "shellscript");
        assert!(by_injection_name("klingon").is_none());
        assert!(by_injection_name("makefile").is_none(), "no grammar");
    }
}
