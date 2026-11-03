//! The design system (spec §3.4, §16): semantic colour tokens and glyph
//! sets shared by every component. M1–M3 ship a built-in "YACE Dark";
//! loading themes from TOML arrives in M6 without changing widgets.

use ratatui::style::{Color, Modifier, Style};
use yace_config::{ColorDepth, Glyphs as GlyphSetting};

#[derive(Clone, Debug)]
pub struct Theme {
    pub name: &'static str,
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
}

impl Theme {
    pub fn yace_dark() -> Self {
        Self {
            name: "YACE Dark",
            bg: Color::Rgb(0x1e, 0x1f, 0x24),
            fg: Color::Rgb(0xd4, 0xd6, 0xdc),
            muted: Color::Rgb(0x80, 0x86, 0x92),
            sidebar_bg: Color::Rgb(0x19, 0x1a, 0x1f),
            sidebar_fg: Color::Rgb(0xc4, 0xc7, 0xcf),
            sidebar_title: Color::Rgb(0x9a, 0xa0, 0xac),
            tab_active_bg: Color::Rgb(0x1e, 0x1f, 0x24),
            tab_active_fg: Color::Rgb(0xf2, 0xf3, 0xf5),
            tab_inactive_bg: Color::Rgb(0x25, 0x26, 0x2c),
            tab_inactive_fg: Color::Rgb(0x8c, 0x91, 0x9c),
            tab_bar_bg: Color::Rgb(0x25, 0x26, 0x2c),
            border: Color::Rgb(0x34, 0x36, 0x3e),
            focus_border: Color::Rgb(0x4d, 0x8d, 0xf5),
            selection_bg: Color::Rgb(0x26, 0x4f, 0x78),
            selection_inactive_bg: Color::Rgb(0x3a, 0x3d, 0x41),
            cursor_line_bg: Color::Rgb(0x26, 0x28, 0x2e),
            line_number: Color::Rgb(0x5a, 0x5f, 0x6b),
            line_number_active: Color::Rgb(0xc6, 0xc9, 0xd1),
            cursor_fg: Color::Rgb(0x1e, 0x1f, 0x24),
            cursor_bg: Color::Rgb(0xae, 0xaf, 0xad),
            bracket_match_bg: Color::Rgb(0x3b, 0x51, 0x4d),
            status_bg: Color::Rgb(0x1f, 0x4f, 0x8f),
            status_fg: Color::Rgb(0xf2, 0xf3, 0xf5),
            status_no_folder_bg: Color::Rgb(0x6c, 0x3c, 0x8c),
            list_active_bg: Color::Rgb(0x04, 0x39, 0x5e),
            list_active_fg: Color::Rgb(0xf2, 0xf3, 0xf5),
            list_inactive_selection_bg: Color::Rgb(0x37, 0x37, 0x3d),
            list_hover_match: Color::Rgb(0x3d, 0xa9, 0xfc),
            overlay_bg: Color::Rgb(0x25, 0x26, 0x2c),
            overlay_fg: Color::Rgb(0xd4, 0xd6, 0xdc),
            overlay_border: Color::Rgb(0x4d, 0x8d, 0xf5),
            overlay_selected_bg: Color::Rgb(0x04, 0x39, 0x5e),
            input_bg: Color::Rgb(0x31, 0x33, 0x3b),
            error: Color::Rgb(0xf1, 0x4c, 0x4c),
            warning: Color::Rgb(0xcc, 0xa7, 0x00),
            info: Color::Rgb(0x3d, 0xa9, 0xfc),
            git_modified: Color::Rgb(0xe2, 0xc0, 0x8d),
            git_added: Color::Rgb(0x81, 0xb8, 0x8b),
            git_deleted: Color::Rgb(0xc7, 0x4e, 0x39),
            dirty: Color::Rgb(0xe2, 0xc0, 0x8d),
            missing_root: Color::Rgb(0xc7, 0x4e, 0x39),
            scrollbar: Color::Rgb(0x42, 0x45, 0x4f),
            whitespace: Color::Rgb(0x45, 0x48, 0x52),
            find_match_bg: Color::Rgb(0x62, 0x33, 0x15),
            find_current_bg: Color::Rgb(0x9e, 0x6a, 0x03),
            match_fg: Color::Rgb(0x3d, 0xa9, 0xfc),
        }
    }

    /// Downgrade every token for terminals with fewer colours.
    pub fn for_depth(mut self, depth: Depth) -> Self {
        if depth == Depth::TrueColor {
            return self;
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
            match_fg
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
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Depth {
    TrueColor,
    Ansi256,
    Ansi16,
    Mono,
}

/// Detect colour depth from the environment (spec §16.5).
pub fn detect_depth(setting: ColorDepth) -> Depth {
    match setting {
        ColorDepth::Truecolor => return Depth::TrueColor,
        ColorDepth::Ansi256 => return Depth::Ansi256,
        ColorDepth::Ansi16 => return Depth::Ansi16,
        ColorDepth::Mono => return Depth::Mono,
        ColorDepth::Auto => {}
    }
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

fn downgrade(c: Color, depth: Depth) -> Color {
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
            },
            GlyphSetting::NerdFont => Self {
                expanded: "",
                collapsed: "",
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
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
