//! The settings schema and its defaults. Every key is documented in
//! `docs/configuration.md`.

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
pub struct Settings {
    pub editor: EditorSettings,
    pub files: FilesSettings,
    pub explorer: ExplorerSettings,
    pub workbench: WorkbenchSettings,
    pub keyboard: KeyboardSettings,
    pub search: SearchSettings,
    pub terminal: TerminalSettings,
    pub lsp: LspSettings,
    pub git: GitSettings,
    pub plugins: PluginsSettings,
    /// Each plugin's own settings: `[plugin."dev.example.hello"]`, keys as
    /// declared in its manifest (validated when the plugin reads them).
    pub plugin: std::collections::BTreeMap<String, toml::Table>,
}

/// Git integration (spec §15).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
pub struct GitSettings {
    pub enabled: bool,
    /// The git program (user settings only).
    pub path: String,
    /// Colour changed files in the Explorer.
    pub decorations_enabled: bool,
    /// Refresh status on file and repository changes and focus.
    pub auto_refresh: bool,
    /// Change bars in the editor gutter: `all`, `gutter` or `none`.
    pub diff_decorations: DiffDecorations,
}

impl Default for GitSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            path: "git".into(),
            decorations_enabled: true,
            auto_refresh: true,
            diff_decorations: DiffDecorations::All,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DiffDecorations {
    All,
    Gutter,
    None,
}

/// Plugin enablement (spec §18.11). User scope; per-workspace overrides
/// live in the workspace's session state, never in the workspace file.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
pub struct PluginsSettings {
    /// Plugin ids that are not loaded.
    pub disabled: Vec<String>,
}

/// Language servers (spec §13). User scope only: a workspace must never
/// choose which programs run (§4.12).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
pub struct LspSettings {
    pub enabled: bool,
    /// Server names never started (built-in or configured).
    pub disabled: Vec<String>,
    /// Stop a server this long after its last document closed (0 = never).
    pub idle_shutdown_secs: u64,
    /// Requests without an answer after this long are dropped.
    pub request_timeout_ms: u64,
    /// Extra servers, or overrides of built-in ones (same `name`).
    pub servers: Vec<LspServer>,
}

impl Default for LspSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            disabled: Vec::new(),
            idle_shutdown_secs: 300,
            request_timeout_ms: 10_000,
            servers: Vec::new(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
pub struct LspServer {
    pub name: String,
    pub command: String,
    pub args: Vec<String>,
    /// Language ids (`rust`, `typescript`, `typescriptreact`, …).
    pub languages: Vec<String>,
    pub env: std::collections::BTreeMap<String, String>,
    pub initialization_options: Option<toml::Value>,
    /// Answers the server's `workspace/configuration` requests.
    pub settings: Option<toml::Value>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum LineNumbers {
    #[default]
    On,
    Off,
    Relative,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
pub struct EditorSettings {
    /// Spaces per indentation level / tab stop width.
    pub tab_size: usize,
    /// Insert spaces when pressing Tab.
    pub insert_spaces: bool,
    /// Auto-close brackets and quotes.
    pub auto_closing_brackets: bool,
    pub line_numbers: LineNumbers,
    /// Highlight the line containing the primary cursor.
    pub render_line_highlight: bool,
    /// Lines kept visible above/below the cursor when scrolling.
    pub cursor_surrounding_lines: usize,
    /// Allow scrolling so the last line can reach the top of the view.
    pub scroll_beyond_last_line: bool,
    /// Highlight the bracket matching the one at the cursor.
    pub match_brackets: bool,
    /// Soft wrapping of long lines (VS Code `editor.wordWrap`).
    pub word_wrap: WordWrap,
    /// Column for `wordWrapColumn` / `bounded`.
    pub word_wrap_column: usize,
    /// Code folding.
    pub folding: bool,
    /// When fold markers show in the gutter.
    pub show_folding_controls: FoldingControls,
    /// Syntax colour overrides (VS Code shape: `comments`, `strings`,
    /// `keywords`, … or YACE scope names; `"[Theme Name]"` sections apply to
    /// one theme only). See `docs/themes.md`.
    pub token_color_customizations: toml::Table,
    /// Format with the language server before saving.
    pub format_on_save: bool,
    /// The language server that formats (`"biome"`), when it serves the
    /// file; otherwise the file's main server.
    pub default_formatter: Option<String>,
    /// Ask for completions while typing (not only on trigger characters).
    pub quick_suggestions: bool,
    /// Milliseconds of typing pause before quick suggestions are requested.
    pub quick_suggestions_delay: u64,
    /// Ask for completions on the server's trigger characters (`.`, `::`, …).
    pub suggest_on_trigger_characters: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum WordWrap {
    #[default]
    Off,
    /// At the viewport width.
    On,
    /// At `wordWrapColumn`.
    WordWrapColumn,
    /// At the smaller of the viewport width and `wordWrapColumn`.
    Bounded,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum FoldingControls {
    Always,
    /// Folded lines always; foldable ones while the mouse is over the gutter.
    #[default]
    Mouseover,
    Never,
}

impl Default for EditorSettings {
    fn default() -> Self {
        Self {
            tab_size: 4,
            insert_spaces: true,
            auto_closing_brackets: true,
            line_numbers: LineNumbers::On,
            render_line_highlight: true,
            cursor_surrounding_lines: 3,
            scroll_beyond_last_line: true,
            match_brackets: true,
            word_wrap: WordWrap::Off,
            word_wrap_column: 80,
            folding: true,
            show_folding_controls: FoldingControls::Mouseover,
            token_color_customizations: toml::Table::new(),
            format_on_save: false,
            default_formatter: None,
            quick_suggestions: true,
            quick_suggestions_delay: 50,
            suggest_on_trigger_characters: true,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum AutoSave {
    #[default]
    Off,
    AfterDelay,
    OnFocusChange,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum Eol {
    /// Use the file's dominant ending; LF for new files.
    #[default]
    Auto,
    Lf,
    Crlf,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
pub struct FilesSettings {
    /// Glob patterns hidden from the Explorer (and later Quick Open/search).
    pub exclude: Vec<String>,
    pub auto_save: AutoSave,
    /// Delay for `autoSave = "afterDelay"`, in milliseconds.
    pub auto_save_delay: u64,
    pub insert_final_newline: bool,
    pub trim_trailing_whitespace: bool,
    /// Line ending for new files.
    pub eol: Eol,
    /// Files larger than this (MiB) open in large-file mode after a prompt.
    pub large_file_threshold_mb: u64,
    /// Glob (or file name) → language id, as VS Code's `files.associations`.
    pub associations: std::collections::BTreeMap<String, String>,
}

impl Default for FilesSettings {
    fn default() -> Self {
        Self {
            exclude: vec![
                "**/.git".into(),
                "**/.svn".into(),
                "**/.hg".into(),
                "**/CVS".into(),
                "**/.DS_Store".into(),
                "**/Thumbs.db".into(),
            ],
            auto_save: AutoSave::Off,
            auto_save_delay: 1000,
            insert_final_newline: false,
            trim_trailing_whitespace: false,
            eol: Eol::Auto,
            large_file_threshold_mb: 50,
            associations: Default::default(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
pub struct ExplorerSettings {
    /// Show dotfiles (names starting with `.`). Excluded globs stay hidden.
    pub show_hidden: bool,
    /// Ask before moving files to the trash.
    pub confirm_delete: bool,
}

impl Default for ExplorerSettings {
    fn default() -> Self {
        Self { show_hidden: true, confirm_delete: true }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum Glyphs {
    #[default]
    Unicode,
    NerdFont,
    Ascii,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum ColorDepth {
    #[default]
    Auto,
    Truecolor,
    #[serde(rename = "256")]
    Ansi256,
    #[serde(rename = "16")]
    Ansi16,
    Mono,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
pub struct WorkbenchSettings {
    /// Single-click opens files in a reusable preview tab.
    pub enable_preview: bool,
    /// `yace` with no arguments reopens the most recent workspace.
    pub restore_workspace: bool,
    pub glyphs: Glyphs,
    /// File-type icons: devicons with `glyphs = "nerdFont"`, a coloured
    /// dot with `"unicode"`.
    pub file_icons: bool,
    pub color_depth: ColorDepth,
    /// Initial sidebar width in cells.
    pub sidebar_width: u16,
    /// Theme name or id (`docs/themes.md`).
    pub color_theme: String,
    /// UI colour overrides by VS Code colour id; `"[Theme Name]"` sections
    /// apply to one theme only.
    pub color_customizations: toml::Table,
    pub side_bar_location: SideBarLocation,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum SideBarLocation {
    #[default]
    Left,
    Right,
}

impl Default for WorkbenchSettings {
    fn default() -> Self {
        Self {
            enable_preview: true,
            restore_workspace: true,
            glyphs: Glyphs::Unicode,
            file_icons: true,
            color_depth: ColorDepth::Auto,
            sidebar_width: 32,
            color_theme: "YACE Dark".into(),
            color_customizations: toml::Table::new(),
            side_bar_location: SideBarLocation::Left,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
pub struct KeyboardSettings {
    /// How long a chord prefix (e.g. Ctrl+K) waits for its second key.
    pub chord_timeout_ms: u64,
    /// Request the Kitty keyboard protocol when the terminal supports it.
    pub enhanced_protocol: bool,
}

impl Default for KeyboardSettings {
    fn default() -> Self {
        Self { chord_timeout_ms: 3000, enhanced_protocol: true }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
pub struct SearchSettings {
    /// Globs excluded from Quick Open and search (in addition to
    /// `files.exclude`). Additive across layers; `"!glob"` removes.
    pub exclude: Vec<String>,
    /// Honour `.gitignore`, `.ignore` and global git excludes.
    pub use_ignore_files: bool,
    /// Quick Open also lists git-ignored files.
    pub quick_open_include_ignored: bool,
    /// Cache file names on disk for an instant Quick Open on next launch.
    pub cache_index: bool,
    /// Maximum matches collected by a workspace search.
    pub max_results: usize,
    /// Maximum files indexed per root for Quick Open.
    pub max_indexed_files: usize,
}

impl Default for SearchSettings {
    fn default() -> Self {
        Self {
            exclude: vec![
                "**/node_modules".into(),
                "**/bower_components".into(),
                "**/*.code-search".into(),
                // Claude Code worktrees: full copies of the repository.
                "**/.claude/worktrees".into(),
            ],
            use_ignore_files: true,
            quick_open_include_ignored: false,
            cache_index: true,
            max_results: 20_000,
            max_indexed_files: 500_000,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum TerminalCwd {
    /// The workspace folder of the active file (else the first folder).
    #[default]
    ActiveRoot,
    /// The directory of the active file.
    ActiveFileDir,
    /// The first workspace folder.
    FirstRoot,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum ConfirmKill {
    /// Ask only when a command is running in the terminal.
    #[default]
    WhenRunning,
    Always,
    Never,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
pub struct TerminalProfile {
    pub name: String,
    /// Empty = the default shell.
    pub shell: String,
    pub args: Vec<String>,
    pub env: std::collections::BTreeMap<String, String>,
    /// Typed into the shell after it starts (e.g. `"claude"`).
    pub startup_command: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
pub struct TerminalSettings {
    /// Shell executable; empty = `$SHELL` (else `/bin/sh`).
    pub shell: String,
    pub args: Vec<String>,
    /// Extra environment variables for new terminals.
    pub env: std::collections::BTreeMap<String, String>,
    /// Lines of scrollback per terminal (bounded memory).
    pub scrollback: usize,
    /// Height of the bottom panel in rows.
    pub panel_height: u16,
    /// Where new terminals start.
    pub cwd: TerminalCwd,
    /// Extra command ids that run even while a terminal has focus.
    pub commands_to_skip_shell: Vec<String>,
    pub confirm_kill: ConfirmKill,
    pub profiles: Vec<TerminalProfile>,
    /// Profile used by "New Terminal" (empty = the settings above).
    pub default_profile: String,
}

impl Default for TerminalSettings {
    fn default() -> Self {
        Self {
            shell: String::new(),
            args: Vec::new(),
            env: Default::default(),
            scrollback: 10_000,
            panel_height: 12,
            cwd: TerminalCwd::ActiveRoot,
            commands_to_skip_shell: Vec::new(),
            confirm_kill: ConfirmKill::WhenRunning,
            profiles: Vec::new(),
            default_profile: String::new(),
        }
    }
}

/// Where a setting may be configured (spec §17.1).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Scope {
    /// User settings only (application-wide behaviour).
    Application,
    /// User or workspace.
    Window,
    /// User, workspace or per-root (repository) settings.
    Resource,
}

/// Scope of a `section.key` setting.
pub fn scope_of(section: &str, key: &str) -> Scope {
    match (section, key) {
        ("editor", "tokenColorCustomizations") => Scope::Window,
        ("editor", _) => Scope::Resource,
        ("files", "autoSave" | "autoSaveDelay" | "largeFileThresholdMb") => Scope::Window,
        ("files", _) => Scope::Resource,
        ("keyboard", _) => Scope::Application,
        // A workspace must never choose which program runs (spec §4.12).
        ("terminal", _) => Scope::Application,
        ("lsp", _) => Scope::Application,
        // Which native code runs is the user's decision only (§18.9).
        ("plugins", _) => Scope::Application,
        // Which program runs is the user's decision only (§4.12).
        ("git", "path") => Scope::Application,
        ("plugin", _) => Scope::Window,
        ("search", "exclude" | "useIgnoreFiles") => Scope::Resource,
        ("workbench", "restoreWorkspace" | "colorDepth" | "glyphs") => Scope::Application,
        _ => Scope::Window,
    }
}
