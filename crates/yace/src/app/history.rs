//! Navigation history (spec §10.3): Back / Forward across files and roots.
//! A location is recorded *before* every jump (open file, Quick Open,
//! go to line, search result, document start/end, large cursor moves).

use std::path::PathBuf;

use super::App;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Location {
    pub path: PathBuf,
    pub line: usize,
    pub col: usize,
}

#[derive(Debug, Default)]
pub struct NavHistory {
    back: Vec<Location>,
    forward: Vec<Location>,
}

const LIMIT: usize = 50;

impl NavHistory {
    pub fn can_go_back(&self) -> bool {
        !self.back.is_empty()
    }
    pub fn can_go_forward(&self) -> bool {
        !self.forward.is_empty()
    }
    fn push(&mut self, loc: Location) {
        // Collapse repeated jumps from the same line.
        if self.back.last().is_some_and(|l| l.path == loc.path && l.line == loc.line) {
            return;
        }
        self.back.push(loc);
        if self.back.len() > LIMIT {
            self.back.remove(0);
        }
        self.forward.clear();
    }
}

impl App {
    /// The primary cursor location of the active editor.
    pub(crate) fn current_location(&self) -> Option<Location> {
        let (g, v, d) = self.active()?;
        let doc = self.docs.doc(d)?;
        let path = doc.path()?.to_path_buf();
        let rope = doc.rope();
        let head = self.layout.group(g)?.views[v].sel.primary().head.min(rope.len_chars());
        let line = rope.char_to_line(head);
        Some(Location { path, line, col: head - rope.line_to_char(line) })
    }

    /// Remember where we are before jumping elsewhere.
    pub(crate) fn record_jump(&mut self) {
        if let Some(loc) = self.current_location() {
            self.nav.push(loc);
        }
    }

    fn go_to_location(&mut self, loc: &Location) {
        self.open_file(&loc.path, false);
        self.set_cursor_line_col(loc.line, loc.col);
    }

    pub(crate) fn navigate_back(&mut self) {
        let Some(loc) = self.nav.back.pop() else { return };
        if let Some(cur) = self.current_location() {
            self.nav.forward.push(cur);
        }
        self.go_to_location(&loc);
    }

    pub(crate) fn navigate_forward(&mut self) {
        let Some(loc) = self.nav.forward.pop() else { return };
        if let Some(cur) = self.current_location() {
            self.nav.back.push(cur);
        }
        self.go_to_location(&loc);
    }
}
