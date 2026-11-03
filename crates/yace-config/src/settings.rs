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
    pub color_depth: ColorDepth,
    /// Initial sidebar width in cells.
    pub sidebar_width: u16,
}

impl Default for WorkbenchSettings {
    fn default() -> Self {
        Self {
            enable_preview: true,
            restore_workspace: true,
            glyphs: Glyphs::Unicode,
            color_depth: ColorDepth::Auto,
            sidebar_width: 32,
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
            exclude: vec!["**/node_modules".into(), "**/bower_components".into(), "**/*.code-search".into()],
            use_ignore_files: true,
            quick_open_include_ignored: false,
            cache_index: true,
            max_results: 20_000,
            max_indexed_files: 500_000,
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
        ("editor", _) => Scope::Resource,
        ("files", "autoSave" | "autoSaveDelay" | "largeFileThresholdMb") => Scope::Window,
        ("files", _) => Scope::Resource,
        ("keyboard", _) => Scope::Application,
        ("search", "exclude" | "useIgnoreFiles") => Scope::Resource,
        ("workbench", "restoreWorkspace" | "colorDepth" | "glyphs") => Scope::Application,
        _ => Scope::Window,
    }
}
