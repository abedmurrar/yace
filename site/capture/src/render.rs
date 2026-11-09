//! Screen snapshots and their SVG rendering.
//!
//! A [`Frame`] is a copy of the emulated screen (characters, colours and
//! text attributes per cell). [`svg_still`] draws one frame; [`svg_animated`]
//! draws a sequence as a single SVG whose frames are shown in turn with a
//! CSS animation (no JavaScript). Rows that repeat across frames are stored
//! once and reused, which keeps recordings small.

use std::{collections::HashMap, fmt::Write as _};

use alacritty_terminal::{
    Term,
    event::VoidListener,
    grid::Dimensions,
    index::{Column, Line},
    term::{cell::Flags, color::Colors},
    vte::ansi::{Color, NamedColor, Rgb},
};

/// Cell size in SVG pixels (14 px monospace, 0.6 em advance).
pub const CW: f32 = 8.4;
pub const CH: f32 = 17.0;
const FONT_SIZE: f32 = 14.0;
const BASELINE: f32 = 12.8;
const PAD: f32 = 8.0;
const FONTS: &str = "ui-monospace,'Cascadia Mono','DejaVu Sans Mono',Menlo,Consolas,'Liberation Mono','Symbols Nerd Font Mono','Symbols Nerd Font',monospace";

/// What the terminal shows where the application set no colour.
#[derive(Clone, Copy)]
pub struct Defaults {
    pub fg: (u8, u8, u8),
    pub bg: (u8, u8, u8),
}

#[derive(Clone, PartialEq, Eq, Hash)]
pub struct Cell {
    pub text: String,
    pub fg: (u8, u8, u8),
    pub bg: (u8, u8, u8),
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub strike: bool,
    pub dim: bool,
    /// 1 normally, 2 for a wide character, 0 for the spacer after it.
    pub width: u8,
}

#[derive(Clone, PartialEq, Eq)]
pub struct Frame {
    pub cols: usize,
    pub rows: Vec<Vec<Cell>>,
    pub cursor: Option<(usize, usize)>,
}

impl Frame {
    pub fn text(&self) -> String {
        let mut s = String::new();
        for row in &self.rows {
            for c in row {
                if c.width > 0 {
                    s.push_str(&c.text);
                }
            }
            s.push('\n');
        }
        s
    }
}

/// The xterm 256-colour palette (the 16 base colours as xterm defaults).
fn xterm(i: u8) -> (u8, u8, u8) {
    const BASE: [(u8, u8, u8); 16] = [
        (0x00, 0x00, 0x00),
        (0xcd, 0x31, 0x31),
        (0x0d, 0xbc, 0x79),
        (0xe5, 0xe5, 0x10),
        (0x24, 0x72, 0xc8),
        (0xbc, 0x3f, 0xbc),
        (0x11, 0xa8, 0xcd),
        (0xe5, 0xe5, 0xe5),
        (0x66, 0x66, 0x66),
        (0xf1, 0x4c, 0x4c),
        (0x23, 0xd1, 0x8b),
        (0xf5, 0xf5, 0x43),
        (0x3b, 0x8e, 0xea),
        (0xd6, 0x70, 0xd6),
        (0x11, 0xa8, 0xcd),
        (0xff, 0xff, 0xff),
    ];
    match i {
        0..=15 => BASE[i as usize],
        16..=231 => {
            let i = i - 16;
            let v = |x: u8| if x == 0 { 0 } else { 55 + x * 40 };
            (v(i / 36), v((i / 6) % 6), v(i % 6))
        }
        _ => {
            let g = 8 + (i - 232) * 10;
            (g, g, g)
        }
    }
}

fn resolve(c: Color, colors: &Colors, d: Defaults, is_fg: bool) -> (u8, u8, u8) {
    let set = |i: usize| colors[i].map(|Rgb { r, g, b }| (r, g, b));
    match c {
        Color::Spec(Rgb { r, g, b }) => (r, g, b),
        Color::Indexed(i) => set(i as usize).unwrap_or_else(|| xterm(i)),
        Color::Named(n) => {
            if let Some(rgb) = set(n as usize) {
                return rgb;
            }
            match n {
                NamedColor::Foreground | NamedColor::DimForeground | NamedColor::BrightForeground => d.fg,
                NamedColor::Background => d.bg,
                NamedColor::Cursor => d.fg,
                other => {
                    let i = other as usize;
                    if i < 16 {
                        xterm(i as u8)
                    } else if (NamedColor::DimBlack as usize..=NamedColor::DimWhite as usize).contains(&i) {
                        xterm((i - NamedColor::DimBlack as usize) as u8)
                    } else if is_fg {
                        d.fg
                    } else {
                        d.bg
                    }
                }
            }
        }
    }
}

/// Copy the visible screen of `term`.
pub fn snapshot(term: &Term<VoidListener>, d: Defaults) -> Frame {
    let rows = term.screen_lines();
    let cols = term.columns();
    let colors = term.colors();
    let grid = term.grid();
    let mut out = Vec::with_capacity(rows);
    for r in 0..rows {
        let line = &grid[Line(r as i32)];
        let mut row = Vec::with_capacity(cols);
        for c in 0..cols {
            let cell = &line[Column(c)];
            let f = cell.flags;
            let mut fg = resolve(cell.fg, colors, d, true);
            let mut bg = resolve(cell.bg, colors, d, false);
            if f.contains(Flags::INVERSE) {
                std::mem::swap(&mut fg, &mut bg);
            }
            if f.contains(Flags::HIDDEN) {
                fg = bg;
            }
            let width = if f.contains(Flags::WIDE_CHAR) {
                2
            } else if f.intersects(Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER) {
                0
            } else {
                1
            };
            let mut text = String::new();
            text.push(if cell.c == '\0' { ' ' } else { cell.c });
            if let Some(z) = cell.zerowidth() {
                text.extend(z.iter());
            }
            row.push(Cell {
                text,
                fg,
                bg,
                bold: f.contains(Flags::BOLD),
                italic: f.contains(Flags::ITALIC),
                underline: f.intersects(Flags::ALL_UNDERLINES),
                strike: f.contains(Flags::STRIKEOUT),
                dim: f.contains(Flags::DIM),
                width,
            });
        }
        out.push(row);
    }
    let cur = term.grid().cursor.point;
    let shown = term.mode().contains(alacritty_terminal::term::TermMode::SHOW_CURSOR);
    let cursor = (shown && cur.line.0 >= 0 && (cur.line.0 as usize) < rows && cur.column.0 < cols)
        .then_some((cur.line.0 as usize, cur.column.0));
    Frame { cols, rows: out, cursor }
}

fn hex((r, g, b): (u8, u8, u8)) -> String {
    format!("#{r:02x}{g:02x}{b:02x}")
}

fn esc(s: &str, out: &mut String) {
    for ch in s.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            c if (c as u32) < 0x20 => out.push(' '),
            c => out.push(c),
        }
    }
}

fn n(v: f32) -> String {
    let s = format!("{v:.1}");
    s.strip_suffix(".0").map(str::to_owned).unwrap_or(s)
}

/// One row drawn at y = 0: background runs, then text runs.
fn row_svg(row: &[Cell], d: Defaults) -> String {
    let mut s = String::new();
    // Backgrounds.
    let mut c = 0;
    while c < row.len() {
        let bg = row[c].bg;
        let start = c;
        while c < row.len() && row[c].bg == bg {
            c += 1;
        }
        if bg != d.bg {
            let _ = write!(
                s,
                r#"<rect x="{}" width="{}" height="{}" fill="{}"/>"#,
                n(start as f32 * CW),
                n((c - start) as f32 * CW),
                n(CH),
                hex(bg)
            );
        }
    }
    // Text: runs of the same style; a wide character is its own run.
    let key = |x: &Cell| (x.fg, x.bold, x.italic, x.underline, x.strike, x.dim);
    let mut c = 0;
    while c < row.len() {
        let cell = &row[c];
        if cell.width == 0 {
            c += 1;
            continue;
        }
        let start = c;
        let mut text = String::new();
        let mut cells = 0usize;
        if cell.width == 2 {
            text.push_str(&cell.text);
            cells = 2;
            c += 1;
        } else {
            while c < row.len() && row[c].width == 1 && key(&row[c]) == key(cell) {
                text.push_str(&row[c].text);
                cells += 1;
                c += 1;
            }
        }
        let trimmed = text.trim_end();
        if trimmed.is_empty() && !cell.underline && !cell.strike {
            continue;
        }
        let (text, cells) = if cell.underline || cell.strike {
            (text.as_str(), cells)
        } else {
            (trimmed, trimmed.chars().count().clamp(1, cells))
        };
        let _ = write!(s, r#"<text x="{}" y="{}""#, n(start as f32 * CW), n(BASELINE));
        if cell.fg != d.fg {
            let _ = write!(s, r#" fill="{}""#, hex(cell.fg));
        }
        if cell.bold {
            s.push_str(r#" font-weight="bold""#);
        }
        if cell.italic {
            s.push_str(r#" font-style="italic""#);
        }
        match (cell.underline, cell.strike) {
            (true, true) => s.push_str(r#" text-decoration="underline line-through""#),
            (true, false) => s.push_str(r#" text-decoration="underline""#),
            (false, true) => s.push_str(r#" text-decoration="line-through""#),
            _ => {}
        }
        if cell.dim {
            s.push_str(r#" opacity="0.6""#);
        }
        if cells > 1 {
            let _ = write!(s, r#" textLength="{}""#, n(cells as f32 * CW));
        }
        s.push('>');
        esc(text, &mut s);
        s.push_str("</text>");
    }
    s
}

struct Rows {
    ids: HashMap<String, usize>,
    defs: String,
}

impl Rows {
    fn new() -> Self {
        Rows { ids: HashMap::new(), defs: String::new() }
    }
    /// The `<use>` elements for a frame's rows.
    fn frame(&mut self, f: &Frame, d: Defaults) -> String {
        let mut s = String::new();
        for (r, row) in f.rows.iter().enumerate() {
            let svg = row_svg(row, d);
            if svg.is_empty() {
                continue;
            }
            let next = self.ids.len();
            let id = *self.ids.entry(svg.clone()).or_insert_with(|| {
                let _ = write!(self.defs, r#"<g id="r{next}">{svg}</g>"#);
                next
            });
            let _ = write!(s, r##"<use href="#r{id}" y="{}"/>"##, n(r as f32 * CH));
        }
        if let Some((r, c)) = f.cursor {
            let fg = f.rows[r][c].fg;
            let _ = write!(
                s,
                r#"<rect x="{}" y="{}" width="2" height="{}" fill="{}"/>"#,
                n(c as f32 * CW),
                n(r as f32 * CH),
                n(CH),
                hex(fg)
            );
        }
        s
    }
}

fn header(w: f32, h: f32, d: Defaults, title: &str, extra_style: &str) -> String {
    let mut t = String::new();
    esc(title, &mut t);
    format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="{w}" height="{h}" viewBox="0 0 {w} {h}" role="img" aria-label="{t}" font-family="{FONTS}" font-size="{FONT_SIZE}" fill="{fg}" xml:space="preserve"><title>{t}</title><style>text{{white-space:pre}}{extra_style}</style><rect width="{w}" height="{h}" rx="6" fill="{bg}"/>"#,
        w = n(w),
        h = n(h),
        fg = hex(d.fg),
        bg = hex(d.bg),
    )
}

fn size(f: &Frame) -> (f32, f32) {
    (f.cols as f32 * CW + 2.0 * PAD, f.rows.len() as f32 * CH + 2.0 * PAD)
}

/// A still screenshot.
pub fn svg_still(f: &Frame, d: Defaults, title: &str) -> String {
    let (w, h) = size(f);
    let mut rows = Rows::new();
    let body = rows.frame(f, d);
    format!(
        r#"{}<defs>{}</defs><g transform="translate({PAD} {PAD})">{body}</g></svg>
"#,
        header(w, h, d, title, ""),
        rows.defs
    )
}

/// A recording: frames with their durations in seconds, looping.
pub fn svg_animated(frames: &[(Frame, f32)], d: Defaults, title: &str) -> String {
    let (w, h) = size(&frames[0].0);
    let fw = frames[0].0.cols as f32 * CW;
    let total: f32 = frames.iter().map(|(_, t)| t).sum();
    let mut rows = Rows::new();
    let mut strip = String::new();
    let mut keys = String::new();
    let mut at = 0.0f32;
    for (i, (f, dur)) in frames.iter().enumerate() {
        let body = rows.frame(f, d);
        let _ = write!(strip, r#"<g transform="translate({} 0)">{body}</g>"#, n(i as f32 * fw));
        let _ = write!(keys, "{:.3}%{{transform:translateX({}px)}}", at / total * 100.0, n(-(i as f32) * fw));
        at += dur;
    }
    let _ = write!(keys, "100%{{transform:translateX({}px)}}", n(-((frames.len() - 1) as f32) * fw));
    let style = format!(
        "@keyframes play{{{keys}}}.strip{{animation:play {total:.2}s steps(1,end) infinite}}@media (prefers-reduced-motion:reduce){{.strip{{animation:none;transform:translateX({}px)}}}}",
        n(-((frames.len() - 1) as f32) * fw)
    );
    format!(
        r#"{}<defs>{}<clipPath id="screen"><rect width="{}" height="{}"/></clipPath></defs><g transform="translate({PAD} {PAD})"><g clip-path="url(#screen)"><g class="strip">{strip}</g></g></g></svg>
"#,
        header(w, h, d, title, &style),
        rows.defs,
        n(fw),
        n(frames[0].0.rows.len() as f32 * CH),
    )
}
