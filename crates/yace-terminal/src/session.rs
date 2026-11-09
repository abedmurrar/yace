//! A terminal session: a shell on a PTY, emulated by `alacritty_terminal`
//! (spec §11.2, §11.3). PTY I/O and parsing run on alacritty's event-loop
//! thread; the UI reads a [`Screen`] snapshot under a short lock.

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex, OnceLock,
        atomic::{AtomicBool, Ordering},
    },
    thread,
};

use alacritty_terminal::{
    Term,
    event::{Event, EventListener, WindowSize},
    event_loop::{EventLoop, EventLoopSender, Msg},
    grid::{Dimensions, Scroll},
    index::{Boundary, Column, Direction, Line, Point, Side},
    selection::{Selection, SelectionType},
    sync::FairMutex,
    term::{
        Config, TermMode,
        cell::Flags,
        search::{RegexIter, RegexSearch},
    },
    tty,
    vte::ansi::{Color, NamedColor, Rgb},
};
use crossbeam_channel::Sender;

pub type SessionId = u32;

/// What a session tells the UI.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Notice {
    /// New output; the UI should redraw (coalesced until `mark_seen`).
    Output(SessionId),
    Title(SessionId, String),
    /// The shell exited (code if it exited normally).
    Exit(SessionId, Option<i32>),
    Bell(SessionId),
}

#[derive(Debug, Clone)]
pub struct SessionConfig {
    pub shell: String,
    pub args: Vec<String>,
    pub env: Vec<(String, String)>,
    pub cwd: PathBuf,
    pub rows: u16,
    pub cols: u16,
    pub scrollback: usize,
    /// Typed into the shell right after it starts.
    pub startup_command: Option<String>,
}

/// The user's login shell, or `/bin/sh`.
pub fn default_shell() -> String {
    std::env::var("SHELL").ok().filter(|s| !s.is_empty() && Path::new(s).exists()).unwrap_or_else(|| "/bin/sh".into())
}

#[derive(Clone)]
struct Listener {
    id: SessionId,
    notify: Sender<Notice>,
    pending: Arc<AtomicBool>,
    writer: Arc<OnceLock<EventLoopSender>>,
    exit: Arc<Mutex<Option<Option<i32>>>>,
    size: Arc<Mutex<WindowSize>>,
}

impl Listener {
    fn write(&self, s: String) {
        if let Some(w) = self.writer.get() {
            let _ = w.send(Msg::Input(s.into_bytes().into()));
        }
    }
}

impl EventListener for Listener {
    fn send_event(&self, event: Event) {
        match event {
            Event::Wakeup => {
                if !self.pending.swap(true, Ordering::AcqRel) {
                    let _ = self.notify.send(Notice::Output(self.id));
                }
            }
            Event::Title(t) => {
                let _ = self.notify.send(Notice::Title(self.id, t));
            }
            Event::Bell => {
                let _ = self.notify.send(Notice::Bell(self.id));
            }
            Event::ChildExit(status) => {
                let code = status.code();
                if let Ok(mut e) = self.exit.lock() {
                    *e = Some(code);
                }
                let _ = self.notify.send(Notice::Exit(self.id, code));
            }
            Event::PtyWrite(text) => self.write(text),
            Event::ColorRequest(index, fmt) => self.write(fmt(palette(index))),
            Event::TextAreaSizeRequest(fmt) => {
                if let Ok(size) = self.size.lock() {
                    self.write(fmt(*size));
                }
            }
            _ => {}
        }
    }
}

/// xterm-compatible default palette (for OSC 4/10/11 queries).
fn palette(index: usize) -> Rgb {
    const BASE: [(u8, u8, u8); 16] = [
        (0x1e, 0x1f, 0x24),
        (0xe0, 0x6c, 0x75),
        (0x98, 0xc3, 0x79),
        (0xe5, 0xc0, 0x7b),
        (0x61, 0xaf, 0xef),
        (0xc6, 0x78, 0xdd),
        (0x56, 0xb6, 0xc2),
        (0xd4, 0xd6, 0xdc),
        (0x5c, 0x63, 0x70),
        (0xf0, 0x7c, 0x85),
        (0xa8, 0xd3, 0x89),
        (0xf5, 0xd0, 0x8b),
        (0x71, 0xbf, 0xff),
        (0xd6, 0x88, 0xed),
        (0x66, 0xc6, 0xd2),
        (0xff, 0xff, 0xff),
    ];
    let (r, g, b) = match index {
        0..=15 => BASE[index],
        16..=231 => {
            let i = index - 16;
            let v = |x: usize| if x == 0 { 0 } else { (55 + x * 40) as u8 };
            (v(i / 36), v((i / 6) % 6), v(i % 6))
        }
        232..=255 => {
            let g = (8 + (index - 232) * 10) as u8;
            (g, g, g)
        }
        257 => BASE[0],
        _ => BASE[7],
    };
    Rgb { r, g, b }
}

struct Size {
    rows: usize,
    cols: usize,
}

impl Dimensions for Size {
    fn total_lines(&self) -> usize {
        self.rows
    }
    fn screen_lines(&self) -> usize {
        self.rows
    }
    fn columns(&self) -> usize {
        self.cols
    }
}

fn window_size(rows: u16, cols: u16) -> WindowSize {
    WindowSize { num_lines: rows.max(1), num_cols: cols.max(2), cell_width: 1, cell_height: 1 }
}

/// A terminal colour, independent of alacritty's types.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TermColor {
    #[default]
    Default,
    /// ANSI 0–15.
    Named(u8),
    Indexed(u8),
    Rgb(u8, u8, u8),
}

/// A search match in the terminal: first and last cell, inclusive, as
/// (absolute line, column). Absolute lines count from the oldest line in
/// the scrollback, so a match stays put while new output arrives.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TermMatch {
    pub start: (usize, usize),
    pub end: (usize, usize),
}

impl TermMatch {
    fn from<T>(m: &std::ops::RangeInclusive<Point>, t: &Term<T>) -> Self {
        let h = t.grid().history_size() as i32;
        let abs = |p: &Point| ((p.line.0 + h).max(0) as usize, p.column.0);
        Self { start: abs(m.start()), end: abs(m.end()) }
    }
    fn point<T>(t: &Term<T>, (line, col): (usize, usize)) -> Point {
        Point::new(Line(line as i32 - t.grid().history_size() as i32), Column(col))
    }
}

/// Lines searched while holding the terminal lock, at most: output keeps
/// flowing during a long search.
const SEARCH_CHUNK: i32 = 1_000;

/// A compiled terminal search (spec §11: search in the scrollback).
#[derive(Debug, Clone)]
pub struct TermQuery {
    re: RegexSearch,
}

impl TermQuery {
    /// Literal unless `regex`; `whole_word` adds word boundaries. Case
    /// follows `case_sensitive` (not alacritty's smart case).
    pub fn new(query: &str, case_sensitive: bool, whole_word: bool, regex: bool) -> Result<Self, String> {
        if query.is_empty() {
            return Err("empty query".into());
        }
        let body = if regex { query.to_string() } else { regex::escape(query) };
        // The DFA engine only supports ASCII word boundaries.
        let body = if whole_word { format!(r"(?-u:\b)(?:{body})(?-u:\b)") } else { body };
        let flags = if case_sensitive { "(?-i)" } else { "(?i)" };
        RegexSearch::new(&format!("{flags}{body}"))
            .map(|re| Self { re })
            .map_err(|e| e.to_string().lines().last().unwrap_or("invalid regular expression").trim().to_string())
    }
}

/// Cell attributes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct CellStyle {
    pub bold: bool,
    pub dim: bool,
    pub italic: bool,
    pub underline: bool,
    pub inverse: bool,
    pub strike: bool,
    pub hidden: bool,
    pub selected: bool,
    /// Part of a search match / of the current one.
    pub find_match: bool,
    pub find_current: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScreenCell {
    pub ch: char,
    /// Combining characters following `ch`.
    pub extra: Option<Box<str>>,
    /// 2 for a wide character, 0 for its spacer, else 1.
    pub width: u8,
    pub fg: TermColor,
    pub bg: TermColor,
    pub style: CellStyle,
}

impl Default for ScreenCell {
    fn default() -> Self {
        Self {
            ch: ' ',
            extra: None,
            width: 1,
            fg: TermColor::Default,
            bg: TermColor::Default,
            style: CellStyle::default(),
        }
    }
}

/// Terminal modes the UI needs for input encoding.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Modes {
    pub app_cursor: bool,
    pub bracketed_paste: bool,
    pub mouse_report: bool,
    pub mouse_drag: bool,
    pub mouse_motion: bool,
    pub sgr_mouse: bool,
    pub alt_screen: bool,
    pub alternate_scroll: bool,
    pub show_cursor: bool,
}

impl Modes {
    fn from(m: &TermMode) -> Self {
        Self {
            app_cursor: m.contains(TermMode::APP_CURSOR),
            bracketed_paste: m.contains(TermMode::BRACKETED_PASTE),
            mouse_report: m.contains(TermMode::MOUSE_REPORT_CLICK),
            mouse_drag: m.contains(TermMode::MOUSE_DRAG),
            mouse_motion: m.contains(TermMode::MOUSE_MOTION),
            sgr_mouse: m.contains(TermMode::SGR_MOUSE),
            alt_screen: m.contains(TermMode::ALT_SCREEN),
            alternate_scroll: m.contains(TermMode::ALTERNATE_SCROLL),
            show_cursor: m.contains(TermMode::SHOW_CURSOR),
        }
    }
    pub fn any_mouse(&self) -> bool {
        self.mouse_report || self.mouse_drag || self.mouse_motion
    }
}

/// A rendered snapshot of the visible screen.
#[derive(Debug, Clone, Default)]
pub struct Screen {
    pub rows: usize,
    pub cols: usize,
    pub cells: Vec<ScreenCell>,
    /// Cursor (row, col) when visible on screen.
    pub cursor: Option<(usize, usize)>,
    /// Lines scrolled back from the bottom.
    pub display_offset: usize,
    pub history: usize,
    pub modes: Modes,
}

impl Screen {
    pub fn cell(&self, row: usize, col: usize) -> &ScreenCell {
        &self.cells[row * self.cols + col]
    }
    /// Plain text of one row (trailing spaces trimmed).
    pub fn row_text(&self, row: usize) -> String {
        let mut s = String::new();
        for c in 0..self.cols {
            let cell = self.cell(row, c);
            if cell.width == 0 {
                continue;
            }
            s.push(cell.ch);
            if let Some(e) = &cell.extra {
                s.push_str(e);
            }
        }
        s.trim_end().to_string()
    }
    pub fn text(&self) -> String {
        (0..self.rows).map(|r| self.row_text(r)).collect::<Vec<_>>().join("\n")
    }
}

fn map_color(c: Color, colors: &alacritty_terminal::term::color::Colors) -> TermColor {
    match c {
        Color::Spec(Rgb { r, g, b }) => TermColor::Rgb(r, g, b),
        Color::Indexed(i) => match colors[i as usize] {
            Some(Rgb { r, g, b }) => TermColor::Rgb(r, g, b),
            None if i < 16 => TermColor::Named(i),
            None => TermColor::Indexed(i),
        },
        Color::Named(n) => {
            let idx = n as usize;
            if let Some(Rgb { r, g, b }) = colors[idx] {
                return TermColor::Rgb(r, g, b);
            }
            match n {
                NamedColor::Foreground | NamedColor::Background | NamedColor::Cursor => TermColor::Default,
                NamedColor::BrightForeground => TermColor::Named(15),
                NamedColor::DimForeground => TermColor::Default,
                NamedColor::DimBlack => TermColor::Named(0),
                NamedColor::DimRed => TermColor::Named(1),
                NamedColor::DimGreen => TermColor::Named(2),
                NamedColor::DimYellow => TermColor::Named(3),
                NamedColor::DimBlue => TermColor::Named(4),
                NamedColor::DimMagenta => TermColor::Named(5),
                NamedColor::DimCyan => TermColor::Named(6),
                NamedColor::DimWhite => TermColor::Named(7),
                other => TermColor::Named((other as usize).min(15) as u8),
            }
        }
    }
}

pub struct Session {
    pub id: SessionId,
    term: Arc<FairMutex<Term<Listener>>>,
    sender: EventLoopSender,
    pending: Arc<AtomicBool>,
    exit: Arc<Mutex<Option<Option<i32>>>>,
    size: Arc<Mutex<WindowSize>>,
    pid: u32,
    shutdown: bool,
}

/// Searches a terminal's screen and scrollback from a worker thread, a
/// chunk of lines at a time, so neither the UI nor the shell's output
/// waits for it.
#[derive(Clone)]
pub struct Searcher {
    term: Arc<FairMutex<Term<Listener>>>,
}

impl std::fmt::Debug for Searcher {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Searcher")
    }
}

impl Searcher {
    /// The nearest match older (`up`) or newer than `from` (from the
    /// bottom when `None`), wrapping around. `stop` is checked between
    /// chunks; a stopped search returns `None`.
    pub fn find(
        &self,
        q: &mut TermQuery,
        from: Option<TermMatch>,
        up: bool,
        stop: &dyn Fn() -> bool,
    ) -> Option<TermMatch> {
        // Work in absolute lines; convert under each lock.
        let (total, origin_abs) = {
            let t = self.term.lock();
            let total = t.grid().history_size() + t.screen_lines();
            let origin = match from {
                None => (total - 1, t.columns()), // past the last cell
                Some(m) if up => m.start,
                Some(m) => m.end,
            };
            (total, origin)
        };
        // Scan from the origin to the edge, then wrap from the other edge
        // back to the origin.
        let mut line = origin_abs.0 as i64;
        let mut first = true;
        let mut wrapped = false;
        loop {
            if stop() {
                return None;
            }
            let t = self.term.lock();
            let h = t.grid().history_size() as i64;
            let last = t.last_column();
            let (from_pt, to_pt, next) = if up {
                let end_line = (line - i64::from(SEARCH_CHUNK) + 1).max(0);
                let col = if first { origin_abs.1 } else { last.0 + 1 };
                let a = Point::new(Line((line - h) as i32), Column(col.min(last.0)));
                (a, Point::new(Line((end_line - h) as i32), Column(0)), end_line - 1)
            } else {
                let end_line = (line + i64::from(SEARCH_CHUNK) - 1).min(total as i64 - 1);
                let col = if first { origin_abs.1 } else { 0 };
                let a = Point::new(Line((line - h) as i32), Column(col.min(last.0)));
                (a, Point::new(Line((end_line - h) as i32), last), end_line + 1)
            };
            let skip_origin = first && from.is_some();
            let dir = if up { Direction::Left } else { Direction::Right };
            let hit = RegexIter::new(from_pt, to_pt, dir, &t, &mut q.re).map(|m| TermMatch::from(&m, &t)).find(|m| {
                !(skip_origin && Some(*m) == from) && (from.is_none() || !first || before(m, origin_abs, up))
            });
            if let Some(m) = hit {
                return Some(m);
            }
            drop(t);
            first = false;
            if wrapped && ((up && next < origin_abs.0 as i64) || (!up && next > origin_abs.0 as i64)) {
                return None;
            }
            if next < 0 || next >= total as i64 {
                if wrapped {
                    return None;
                }
                wrapped = true;
                line = if up { total as i64 - 1 } else { 0 };
            } else {
                line = next;
            }
        }
    }

    /// How many matches there are (at most `cap`), and the 1-based number
    /// of `current` counted from the top. `None` when stopped.
    pub fn count(
        &self,
        q: &mut TermQuery,
        current: Option<TermMatch>,
        cap: usize,
        stop: &dyn Fn() -> bool,
    ) -> Option<(usize, Option<usize>)> {
        let total = {
            let t = self.term.lock();
            t.grid().history_size() + t.screen_lines()
        };
        let mut n = 0;
        let mut at = None;
        let mut line = 0usize;
        let mut col = 0usize;
        while line < total && n < cap {
            if stop() {
                return None;
            }
            let t = self.term.lock();
            let h = t.grid().history_size() as i64;
            let end_line = (line + SEARCH_CHUNK as usize).min(total) - 1;
            let a = Point::new(Line((line as i64 - h) as i32), Column(col.min(t.last_column().0)));
            let b = Point::new(Line((end_line as i64 - h) as i32), t.last_column());
            let mut next = (end_line + 1, 0);
            for m in RegexIter::new(a, b, Direction::Right, &t, &mut q.re) {
                let tm = TermMatch::from(&m, &t);
                n += 1;
                if current == Some(tm) {
                    at = Some(n);
                }
                if tm.end.0 > end_line {
                    next = (tm.end.0, tm.end.1 + 1); // continue after a match that ran on
                }
                if n >= cap {
                    break;
                }
            }
            (line, col) = next;
        }
        Some((n, at))
    }
}

/// `m` lies before `origin` in the search direction.
fn before(m: &TermMatch, origin: (usize, usize), up: bool) -> bool {
    if up { m.start < origin } else { m.start > origin }
}

impl std::fmt::Debug for Session {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Session").field("id", &self.id).field("pid", &self.pid).finish()
    }
}

impl Session {
    pub fn spawn(cfg: &SessionConfig, id: SessionId, notify: Sender<Notice>) -> std::io::Result<Self> {
        let mut env: HashMap<String, String> = cfg.env.iter().cloned().collect();
        env.entry("TERM".into()).or_insert_with(|| "xterm-256color".into());
        env.entry("COLORTERM".into()).or_insert_with(|| "truecolor".into());
        env.insert("TERM_PROGRAM".into(), "yace".into());
        let opts = tty::Options {
            shell: Some(tty::Shell::new(cfg.shell.clone(), cfg.args.clone())),
            working_directory: Some(cfg.cwd.clone()),
            drain_on_exit: true,
            env,
        };
        let ws = window_size(cfg.rows, cfg.cols);
        let pty = tty::new(&opts, ws, u64::from(id))?;
        let pid = pty.child().id();
        let pending = Arc::new(AtomicBool::new(false));
        let writer = Arc::new(OnceLock::new());
        let exit = Arc::new(Mutex::new(None));
        let size = Arc::new(Mutex::new(ws));
        let listener = Listener {
            id,
            notify,
            pending: Arc::clone(&pending),
            writer: Arc::clone(&writer),
            exit: Arc::clone(&exit),
            size: Arc::clone(&size),
        };
        let config = Config { scrolling_history: cfg.scrollback, ..Config::default() };
        let dims = Size { rows: usize::from(ws.num_lines), cols: usize::from(ws.num_cols) };
        let term = Arc::new(FairMutex::new(Term::new(config, &dims, listener.clone())));
        let event_loop = EventLoop::new(Arc::clone(&term), listener, pty, true, false)?;
        let sender = event_loop.channel();
        let _ = writer.set(sender.clone());
        event_loop.spawn();
        let s = Self { id, term, sender, pending, exit, size, pid, shutdown: false };
        if let Some(cmd) = cfg.startup_command.as_deref().filter(|c| !c.is_empty()) {
            s.write(format!("{cmd}\r").as_bytes());
        }
        Ok(s)
    }

    pub fn pid(&self) -> u32 {
        self.pid
    }

    /// Send bytes to the shell (keys, paste).
    pub fn write(&self, bytes: &[u8]) {
        if !bytes.is_empty() {
            let _ = self.sender.send(Msg::Input(bytes.to_vec().into()));
        }
    }

    /// Allow the next output notification (call after rendering).
    pub fn mark_seen(&self) {
        self.pending.store(false, Ordering::Release);
    }

    pub fn resize(&self, rows: u16, cols: u16) {
        let ws = window_size(rows, cols);
        if let Ok(mut s) = self.size.lock() {
            if s.num_lines == ws.num_lines && s.num_cols == ws.num_cols {
                return;
            }
            *s = ws;
        }
        self.term.lock().resize(Size { rows: usize::from(ws.num_lines), cols: usize::from(ws.num_cols) });
        let _ = self.sender.send(Msg::Resize(ws));
    }

    pub fn size(&self) -> (u16, u16) {
        self.size.lock().map(|s| (s.num_lines, s.num_cols)).unwrap_or((24, 80))
    }

    /// Scroll the view through history (positive = up).
    pub fn scroll(&self, lines: i32) {
        self.term.lock().scroll_display(Scroll::Delta(lines));
    }

    pub fn scroll_to_bottom(&self) {
        self.term.lock().scroll_display(Scroll::Bottom);
    }

    /// Clear the screen and scrollback (Terminal: Clear).
    pub fn clear(&self) {
        let mut t = self.term.lock();
        t.grid_mut().clear_history();
        // Ask the shell to redraw its prompt on a clean screen.
        drop(t);
        self.write(b"\x0c");
    }

    pub fn modes(&self) -> Modes {
        Modes::from(self.term.lock().mode())
    }

    /// Exit status once the shell ended (`Some(None)` = killed by a signal).
    pub fn exit_status(&self) -> Option<Option<i32>> {
        self.exit.lock().ok().and_then(|e| *e)
    }

    pub fn is_running(&self) -> bool {
        self.exit_status().is_none() && !self.shutdown
    }

    /// True when a foreground job other than the shell itself is running.
    pub fn has_running_job(&self) -> bool {
        if !self.is_running() {
            return false;
        }
        let Ok(stat) = std::fs::read_to_string(format!("/proc/{}/stat", self.pid)) else { return false };
        let Some(rest) = stat.rsplit_once(')').map(|(_, r)| r) else { return false };
        let fields: Vec<&str> = rest.split_whitespace().collect();
        // state ppid pgrp session tty_nr tpgid …
        let tpgid: i64 = fields.get(5).and_then(|f| f.parse().ok()).unwrap_or(-1);
        tpgid > 0 && tpgid != i64::from(self.pid)
    }

    /// The shell's current working directory (Linux).
    pub fn current_dir(&self) -> Option<PathBuf> {
        std::fs::read_link(format!("/proc/{}/cwd", self.pid)).ok()
    }

    // --- selection -----------------------------------------------------------

    fn point(&self, t: &Term<Listener>, row: usize, col: usize) -> Point {
        let offset = t.grid().display_offset() as i32;
        let col = col.min(t.columns().saturating_sub(1));
        Point::new(Line(row as i32 - offset), Column(col))
    }

    pub fn select_start(&self, row: usize, col: usize, semantic: bool) {
        let mut t = self.term.lock();
        let p = self.point(&t, row, col);
        let ty = if semantic { SelectionType::Semantic } else { SelectionType::Simple };
        t.selection = Some(Selection::new(ty, p, Side::Left));
    }

    pub fn select_update(&self, row: usize, col: usize) {
        let mut t = self.term.lock();
        let p = self.point(&t, row, col);
        if let Some(sel) = t.selection.as_mut() {
            sel.update(p, Side::Right);
        }
    }

    pub fn select_clear(&self) {
        self.term.lock().selection = None;
    }

    pub fn selection_text(&self) -> Option<String> {
        self.term.lock().selection_to_string().filter(|s| !s.is_empty())
    }

    pub fn has_selection(&self) -> bool {
        self.term.lock().selection.as_ref().is_some_and(|s| !s.is_empty())
    }

    // --- search --------------------------------------------------------------

    /// A handle for searching this terminal from another thread.
    pub fn searcher(&self) -> Searcher {
        Searcher { term: Arc::clone(&self.term) }
    }

    /// Scroll to a match and select it (so copying takes it).
    pub fn show_match(&self, m: TermMatch) {
        let mut t = self.term.lock();
        let (a, b) = (TermMatch::point(&t, m.start), TermMatch::point(&t, m.end));
        if a.line.0 < -(t.grid().history_size() as i32) {
            return; // scrolled out of the history meanwhile
        }
        t.scroll_to_point(a);
        let mut sel = Selection::new(SelectionType::Simple, a, Side::Left);
        sel.update(b, Side::Right);
        t.selection = Some(sel);
    }

    /// [`Session::screen`] with the matches of `q` on screen highlighted
    /// (and `current` marked). Only the visible lines are searched.
    pub fn screen_with_matches(&self, out: &mut Screen, q: &mut TermQuery, current: Option<TermMatch>) {
        self.screen(out);
        let t = self.term.lock();
        let offset = t.grid().display_offset() as i32;
        let top = Point::new(Line(-offset), Column(0));
        let bottom = Point::new(Line(t.screen_lines() as i32 - 1 - offset), t.last_column());
        let cols = out.cols;
        for m in RegexIter::new(top, bottom, Direction::Right, &t, &mut q.re) {
            let is_current = current.is_some_and(|c| TermMatch::from(&m, &t) == c);
            let mut p = *m.start();
            loop {
                let row = p.line.0 + offset;
                if row >= 0 && (row as usize) < out.rows && p.column.0 < cols {
                    let st = &mut out.cells[row as usize * cols + p.column.0].style;
                    st.find_match = true;
                    st.find_current |= is_current;
                }
                if p >= *m.end() {
                    break;
                }
                p = p.add(&*t, Boundary::None, 1);
            }
        }
    }

    /// Snapshot the visible screen.
    pub fn screen(&self, out: &mut Screen) {
        let t = self.term.lock();
        let content = t.renderable_content();
        let rows = t.screen_lines();
        let cols = t.columns();
        out.rows = rows;
        out.cols = cols;
        out.cells.clear();
        out.cells.resize(rows * cols, ScreenCell::default());
        out.display_offset = content.display_offset;
        out.history = t.grid().history_size();
        out.modes = Modes::from(t.mode());
        let sel = content.selection;
        let offset = content.display_offset as i32;
        for ind in content.display_iter {
            let row = ind.point.line.0 + offset;
            if row < 0 || row as usize >= rows {
                continue;
            }
            let (row, col) = (row as usize, ind.point.column.0);
            if col >= cols {
                continue;
            }
            let cell = ind.cell;
            let f = cell.flags;
            let width = if f.contains(Flags::WIDE_CHAR) {
                2
            } else if f.intersects(Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER) {
                0
            } else {
                1
            };
            let selected = sel.is_some_and(|s| s.contains(ind.point));
            out.cells[row * cols + col] = ScreenCell {
                ch: cell.c,
                extra: cell.zerowidth().map(|z| z.iter().collect::<String>().into_boxed_str()),
                width,
                fg: map_color(cell.fg, content.colors),
                bg: map_color(cell.bg, content.colors),
                style: CellStyle {
                    bold: f.contains(Flags::BOLD),
                    dim: f.contains(Flags::DIM),
                    italic: f.contains(Flags::ITALIC),
                    underline: f.intersects(Flags::ALL_UNDERLINES),
                    inverse: f.contains(Flags::INVERSE),
                    strike: f.contains(Flags::STRIKEOUT),
                    hidden: f.contains(Flags::HIDDEN),
                    selected,
                    find_match: false,
                    find_current: false,
                },
            };
        }
        let c = content.cursor.point;
        let crow = c.line.0 + offset;
        out.cursor = (out.modes.show_cursor && crow >= 0 && (crow as usize) < rows && c.column.0 < cols)
            .then_some((crow as usize, c.column.0));
    }

    /// Stop the shell: the event loop shuts down and the PTY is dropped,
    /// which sends SIGHUP to the shell's process group. Reaping happens on
    /// a background thread so the UI never blocks.
    pub fn kill(&mut self) {
        if self.shutdown {
            return;
        }
        self.shutdown = true;
        let _ = self.sender.send(Msg::Shutdown);
        let pid = self.pid;
        thread::Builder::new()
            .name("yace-term-reap".into())
            .spawn(move || {
                // If the shell ignores SIGHUP, make sure it is gone.
                thread::sleep(std::time::Duration::from_secs(2));
                if std::path::Path::new(&format!("/proc/{pid}")).exists() {
                    let _ = std::process::Command::new("kill").arg("-KILL").arg(pid.to_string()).status();
                }
            })
            .ok();
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        self.kill();
    }
}
