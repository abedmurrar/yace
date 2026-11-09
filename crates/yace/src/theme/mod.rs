//! The design system (spec §3.4, §16): semantic colour tokens and glyph
//! sets shared by every component. Themes are TOML files (`schema.rs`):
//! built-in ones are embedded, user ones live in `~/.config/yace/themes/`.

mod builtin_gen;
mod schema;
pub mod vscode;

use std::sync::Arc;

use ratatui::style::{Color, Modifier, Style};
pub use schema::{
    BUILTIN, COLOR_KEYS, Catalog, Origin, Overrides, ThemeSource, color_to_string, load_theme, parse_color,
};
use yace_config::{ColorDepth, Glyphs as GlyphSetting};
use yace_syntax::Scope;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum ThemeKind {
    #[default]
    Dark,
    Light,
}

#[derive(Clone, Debug)]
pub struct Theme {
    /// Display name (`name = …` in the file).
    pub name: Arc<str>,
    /// File stem of the theme (`yace-dark`), used by `extends`.
    pub id: Arc<str>,
    pub kind: ThemeKind,
    pub bg: Color,
    pub fg: Color,
    pub muted: Color,
    pub sidebar_bg: Color,
    pub sidebar_fg: Color,
    pub sidebar_title: Color,
    pub tab_active_bg: Color,
    pub tab_active_fg: Color,
    pub tab_inactive_bg: Color,
    pub tab_inactive_fg: Color,
    pub tab_bar_bg: Color,
    pub border: Color,
    pub focus_border: Color,
    pub selection_bg: Color,
    pub selection_inactive_bg: Color,
    pub cursor_line_bg: Color,
    pub line_number: Color,
    pub line_number_active: Color,
    pub cursor_fg: Color,
    pub cursor_bg: Color,
    pub bracket_match_bg: Color,
    pub status_bg: Color,
    pub status_fg: Color,
    pub status_no_folder_bg: Color,
    pub list_active_bg: Color,
    pub list_active_fg: Color,
    pub list_inactive_selection_bg: Color,
    pub list_hover_match: Color,
    pub overlay_bg: Color,
    pub overlay_fg: Color,
    pub overlay_border: Color,
    pub overlay_selected_bg: Color,
    pub input_bg: Color,
    pub error: Color,
    pub warning: Color,
    pub info: Color,
    pub git_modified: Color,
    pub git_added: Color,
    pub git_deleted: Color,
    pub dirty: Color,
    pub missing_root: Color,
    pub scrollbar: Color,
    pub whitespace: Color,
    pub find_match_bg: Color,
    pub find_current_bg: Color,
    pub match_fg: Color,
    pub terminal_bg: Color,
    pub terminal_fg: Color,
    /// ANSI 0–15 for the integrated terminal.
    pub terminal_ansi: [Color; 16],
    /// Syntax colours, indexed by `yace_syntax::Scope`.
    pub syntax: [Color; Scope::COUNT],
    /// Syntax text styles (bold, italic, underline), same index.
    pub syntax_mods: [Modifier; Scope::COUNT],
    /// Semantic token styles (`[semantic]`), kept for when semantic tokens
    /// are rendered; tree-sitter scopes colour the text today.
    pub semantic: Vec<(Box<str>, Color, Modifier)>,
    /// Colour depth the theme was converted for.
    pub depth: Depth,
}

impl Theme {
    /// A theme with every colour unset; the root of `yace-dark`/`yace-light`.
    pub fn blank(kind: ThemeKind) -> Self {
        let r = Color::Reset;
        Self {
            name: Arc::from(""),
            id: Arc::from(""),
            kind,
            bg: r,
            fg: r,
            muted: r,
            sidebar_bg: r,
            sidebar_fg: r,
            sidebar_title: r,
            tab_active_bg: r,
            tab_active_fg: r,
            tab_inactive_bg: r,
            tab_inactive_fg: r,
            tab_bar_bg: r,
            border: r,
            focus_border: r,
            selection_bg: r,
            selection_inactive_bg: r,
            cursor_line_bg: r,
            line_number: r,
            line_number_active: r,
            cursor_fg: r,
            cursor_bg: r,
            bracket_match_bg: r,
            status_bg: r,
            status_fg: r,
            status_no_folder_bg: r,
            list_active_bg: r,
            list_active_fg: r,
            list_inactive_selection_bg: r,
            list_hover_match: r,
            overlay_bg: r,
            overlay_fg: r,
            overlay_border: r,
            overlay_selected_bg: r,
            input_bg: r,
            error: r,
            warning: r,
            info: r,
            git_modified: r,
            git_added: r,
            git_deleted: r,
            dirty: r,
            missing_root: r,
            scrollbar: r,
            whitespace: r,
            find_match_bg: r,
            find_current_bg: r,
            match_fg: r,
            terminal_bg: r,
            terminal_fg: r,
            terminal_ansi: [r; 16],
            syntax: [r; Scope::COUNT],
            syntax_mods: [Modifier::empty(); Scope::COUNT],
            semantic: Vec::new(),
            depth: Depth::TrueColor,
        }
    }

    /// The default theme (embedded `themes/yace-dark.toml`).
    pub fn yace_dark() -> Self {
        Self::builtin("yace-dark")
    }

    /// A built-in theme by id; panics only if an embedded file is broken
    /// (covered by tests).
    pub fn builtin(id: &str) -> Self {
        let cat = Catalog::builtin();
        match load_theme(&cat, id, &Overrides::default()) {
            Ok((t, _)) => t,
            Err(e) => panic!("built-in theme {id}: {e}"),
        }
    }

    /// Downgrade every token for terminals with fewer colours.
    pub fn for_depth(mut self, depth: Depth) -> Self {
        self.depth = depth;
        if depth == Depth::TrueColor {
            return self;
        }
        for c in &mut self.terminal_ansi {
            *c = if depth == Depth::Ansi16 || depth == Depth::Mono { *c } else { downgrade(*c, depth) };
        }
        if depth == Depth::Ansi16 {
            // Use the terminal's own 16 colours.
            const N: [Color; 16] = [
                Color::Black,
                Color::Red,
                Color::Green,
                Color::Yellow,
                Color::Blue,
                Color::Magenta,
                Color::Cyan,
                Color::Gray,
                Color::DarkGray,
                Color::LightRed,
                Color::LightGreen,
                Color::LightYellow,
                Color::LightBlue,
                Color::LightMagenta,
                Color::LightCyan,
                Color::White,
            ];
            self.terminal_ansi = N;
        } else if depth == Depth::Mono {
            self.terminal_ansi = [Color::Reset; 16];
        }
        for c in &mut self.syntax {
            *c = downgrade(*c, depth);
        }
        for (_, c, _) in &mut self.semantic {
            *c = downgrade(*c, depth);
        }
        macro_rules! conv {
            ($($f:ident),*) => { $( self.$f = downgrade(self.$f, depth); )* };
        }
        conv!(
            bg,
            fg,
            muted,
            sidebar_bg,
            sidebar_fg,
            sidebar_title,
            tab_active_bg,
            tab_active_fg,
            tab_inactive_bg,
            tab_inactive_fg,
            tab_bar_bg,
            border,
            focus_border,
            selection_bg,
            selection_inactive_bg,
            cursor_line_bg,
            line_number,
            line_number_active,
            cursor_fg,
            cursor_bg,
            bracket_match_bg,
            status_bg,
            status_fg,
            status_no_folder_bg,
            list_active_bg,
            list_active_fg,
            list_inactive_selection_bg,
            list_hover_match,
            overlay_bg,
            overlay_fg,
            overlay_border,
            overlay_selected_bg,
            input_bg,
            error,
            warning,
            info,
            git_modified,
            git_added,
            git_deleted,
            dirty,
            missing_root,
            scrollbar,
            whitespace,
            find_match_bg,
            find_current_bg,
            match_fg,
            terminal_bg,
            terminal_fg
        );
        self
    }

    pub fn base(&self) -> Style {
        Style::new().fg(self.fg).bg(self.bg)
    }
    pub fn sidebar(&self) -> Style {
        Style::new().fg(self.sidebar_fg).bg(self.sidebar_bg)
    }
    pub fn title(&self, focused: bool) -> Style {
        let s = Style::new().fg(if focused { self.focus_border } else { self.sidebar_title }).bg(self.sidebar_bg);
        s.add_modifier(Modifier::BOLD)
    }

    // State styles. With no colours (monochrome, `NO_COLOR`), state is shown
    // with text attributes instead so selection, matches and focus stay
    // visible (spec §16.5).

    pub fn is_mono(&self) -> bool {
        self.depth == Depth::Mono
    }
    /// Selected text (patched over the text style).
    pub fn selection(&self, focused: bool) -> Style {
        if self.is_mono() {
            return Style::new().add_modifier(Modifier::REVERSED);
        }
        Style::new().bg(if focused { self.selection_bg } else { self.selection_inactive_bg })
    }
    /// A find match (patched over the text style).
    pub fn find_match(&self, current: bool) -> Style {
        if self.is_mono() {
            return if current {
                Style::new().add_modifier(Modifier::REVERSED | Modifier::BOLD)
            } else {
                Style::new().add_modifier(Modifier::UNDERLINED)
            };
        }
        Style::new().bg(if current { self.find_current_bg } else { self.find_match_bg })
    }
    /// A drawn (non-terminal) cursor cell.
    pub fn cursor(&self) -> Style {
        if self.is_mono() {
            return Style::new().add_modifier(Modifier::REVERSED);
        }
        Style::new().fg(self.cursor_fg).bg(self.cursor_bg)
    }
    /// The selected row of a list (Explorer, pickers).
    pub fn list_selected(&self, focused: bool) -> Style {
        if self.is_mono() {
            return if focused {
                Style::new().add_modifier(Modifier::REVERSED)
            } else {
                Style::new().add_modifier(Modifier::UNDERLINED)
            };
        }
        if focused {
            Style::new().fg(self.list_active_fg).bg(self.list_active_bg)
        } else {
            Style::new().bg(self.list_inactive_selection_bg)
        }
    }
    /// The selected row of an overlay list.
    pub fn overlay_selected(&self) -> Style {
        if self.is_mono() {
            return Style::new().add_modifier(Modifier::REVERSED);
        }
        Style::new().fg(self.list_active_fg).bg(self.overlay_selected_bg)
    }
    /// A tab in the tab bar.
    pub fn tab(&self, active: bool) -> Style {
        if self.is_mono() {
            return if active {
                Style::new().add_modifier(Modifier::BOLD | Modifier::UNDERLINED)
            } else {
                Style::new()
            };
        }
        if active {
            Style::new().fg(self.tab_active_fg).bg(self.tab_active_bg)
        } else {
            Style::new().fg(self.tab_inactive_fg).bg(self.tab_inactive_bg)
        }
    }
    /// The status bar.
    pub fn status(&self, no_folder: bool) -> Style {
        if self.is_mono() {
            return Style::new().add_modifier(Modifier::REVERSED);
        }
        Style::new().fg(self.status_fg).bg(if no_folder { self.status_no_folder_bg } else { self.status_bg })
    }
    /// Text of a syntax scope.
    pub fn syntax_style(&self, scope: Option<Scope>) -> Style {
        let Some(s) = scope else { return Style::new().fg(self.fg) };
        Style::new().fg(self.syntax[s as usize]).add_modifier(self.syntax_mods[s as usize])
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Depth {
    TrueColor,
    Ansi256,
    Ansi16,
    Mono,
}

/// The colour depth to draw with: the `workbench.colorDepth` setting, or
/// for `"auto"` what the terminal reports (`terminal`, from
/// [`terminal_depth`]).
pub fn detect_depth(setting: ColorDepth, terminal: Depth) -> Depth {
    match setting {
        ColorDepth::Truecolor => Depth::TrueColor,
        ColorDepth::Ansi256 => Depth::Ansi256,
        ColorDepth::Ansi16 => Depth::Ansi16,
        ColorDepth::Mono => Depth::Mono,
        ColorDepth::Auto => terminal,
    }
}

/// The colour depth the terminal reports through the environment
/// (`NO_COLOR`, `COLORTERM`, `TERM`; spec §16.5). Read once at startup.
pub fn terminal_depth() -> Depth {
    if std::env::var_os("NO_COLOR").is_some_and(|v| !v.is_empty()) {
        return Depth::Mono;
    }
    let colorterm = std::env::var("COLORTERM").unwrap_or_default().to_ascii_lowercase();
    if colorterm.contains("truecolor") || colorterm.contains("24bit") {
        return Depth::TrueColor;
    }
    let term = std::env::var("TERM").unwrap_or_default();
    if term.contains("direct") || term.contains("kitty") || term.contains("alacritty") || term.contains("wezterm") {
        return Depth::TrueColor;
    }
    if term.contains("256") {
        return Depth::Ansi256;
    }
    if term == "dumb" {
        return Depth::Mono;
    }
    Depth::Ansi16
}

/// Convert a colour for the terminal's colour depth.
pub fn downgrade(c: Color, depth: Depth) -> Color {
    let Color::Rgb(r, g, b) = c else { return c };
    match depth {
        Depth::TrueColor => c,
        Depth::Ansi256 => Color::Indexed(rgb_to_256(r, g, b)),
        Depth::Ansi16 => rgb_to_16(r, g, b),
        Depth::Mono => Color::Reset,
    }
}

fn rgb_to_256(r: u8, g: u8, b: u8) -> u8 {
    // Grey ramp when the colour is nearly grey.
    let (ri, gi, bi) = (i32::from(r), i32::from(g), i32::from(b));
    if (ri - gi).abs() < 10 && (gi - bi).abs() < 10 {
        let avg = (ri + gi + bi) / 3;
        if avg < 8 {
            return 16;
        }
        if avg > 248 {
            return 231;
        }
        return (232 + (avg - 8) * 24 / 240) as u8;
    }
    let q = |v: u8| -> u8 {
        if v < 48 {
            0
        } else if v < 115 {
            1
        } else {
            ((v - 35) / 40).min(5)
        }
    };
    16 + 36 * q(r) + 6 * q(g) + q(b)
}

fn rgb_to_16(r: u8, g: u8, b: u8) -> Color {
    let bright = u16::from(r) + u16::from(g) + u16::from(b) > 382;
    let (rb, gb, bb) = (r > 100, g > 100, b > 100);
    match (rb, gb, bb, bright) {
        (false, false, false, _) => {
            if u16::from(r) + u16::from(g) + u16::from(b) > 120 {
                Color::DarkGray
            } else {
                Color::Black
            }
        }
        (true, false, false, false) => Color::Red,
        (true, false, false, true) => Color::LightRed,
        (false, true, false, false) => Color::Green,
        (false, true, false, true) => Color::LightGreen,
        (false, false, true, false) => Color::Blue,
        (false, false, true, true) => Color::LightBlue,
        (true, true, false, false) => Color::Yellow,
        (true, true, false, true) => Color::LightYellow,
        (true, false, true, false) => Color::Magenta,
        (true, false, true, true) => Color::LightMagenta,
        (false, true, true, false) => Color::Cyan,
        (false, true, true, true) => Color::LightCyan,
        (true, true, true, false) => Color::Gray,
        (true, true, true, true) => Color::White,
    }
}

/// Characters used for icons and decorations (spec §8.6).
#[derive(Clone, Debug)]
pub struct Glyphs {
    pub expanded: &'static str,
    pub collapsed: &'static str,
    pub folder: &'static str,
    pub folder_open: &'static str,
    pub file: &'static str,
    pub symlink: &'static str,
    pub broken: &'static str,
    pub dirty: &'static str,
    pub close: &'static str,
    pub missing: &'static str,
    pub vbar: &'static str,
    pub hbar: &'static str,
    pub scroll_thumb: &'static str,
    pub ellipsis: &'static str,
    pub tab_char: &'static str,
    /// Problem counts in the status bar.
    pub error: &'static str,
    pub warning: &'static str,
}

impl Glyphs {
    pub fn from_setting(s: GlyphSetting) -> Self {
        match s {
            GlyphSetting::Unicode => Self {
                expanded: "▾",
                collapsed: "▸",
                folder: "",
                folder_open: "",
                file: "",
                symlink: "↪",
                broken: "⚠",
                dirty: "●",
                close: "×",
                missing: "⚠",
                vbar: "│",
                hbar: "─",
                scroll_thumb: "┃",
                ellipsis: "…",
                tab_char: " ",
                error: "⊗",
                warning: "⚠",
            },
            GlyphSetting::NerdFont => Self {
                expanded: "\u{f078}",
                collapsed: "\u{f054}",
                folder: "\u{f07b} ",
                folder_open: "\u{f07c} ",
                file: "\u{f15b} ",
                symlink: "\u{f0c1}",
                broken: "\u{f071}",
                dirty: "●",
                close: "\u{f00d}",
                missing: "\u{f071}",
                vbar: "│",
                hbar: "─",
                scroll_thumb: "┃",
                ellipsis: "…",
                tab_char: " ",
                error: "\u{f057}",
                warning: "\u{f071}",
            },
            GlyphSetting::Ascii => Self {
                expanded: "v",
                collapsed: ">",
                folder: "",
                folder_open: "",
                file: "",
                symlink: "->",
                broken: "!",
                dirty: "*",
                close: "x",
                missing: "!",
                vbar: "|",
                hbar: "-",
                scroll_thumb: "#",
                ellipsis: "~",
                tab_char: " ",
                error: "E",
                warning: "W",
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Markers must take one cell, or rows shift (an empty chevron did).
    #[test]
    fn glyph_markers_take_one_cell() {
        use unicode_width::UnicodeWidthStr;
        for s in [GlyphSetting::Unicode, GlyphSetting::NerdFont, GlyphSetting::Ascii] {
            let g = Glyphs::from_setting(s);
            for (name, v) in
                [("expanded", g.expanded), ("collapsed", g.collapsed), ("dirty", g.dirty), ("close", g.close)]
            {
                assert_eq!(UnicodeWidthStr::width(v), 1, "{s:?} {name}");
            }
        }
    }

    #[test]
    fn downgrades() {
        assert!(matches!(downgrade(Color::Rgb(0x1e, 0x1f, 0x24), Depth::Ansi256), Color::Indexed(_)));
        assert_eq!(downgrade(Color::Rgb(255, 0, 0), Depth::Ansi16), Color::Red);
        assert_eq!(downgrade(Color::Rgb(255, 255, 255), Depth::Ansi16), Color::White);
        assert_eq!(downgrade(Color::Rgb(1, 2, 3), Depth::Mono), Color::Reset);
        let t = Theme::yace_dark().for_depth(Depth::Ansi16);
        assert!(!matches!(t.bg, Color::Rgb(..)));
    }
}
