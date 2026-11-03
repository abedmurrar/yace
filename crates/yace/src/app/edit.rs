//! Editor commands: cursor movement, editing, clipboard, undo — all routed
//! through `yace_buffer::ops` so behaviour is unit-tested at the buffer level.

use yace_buffer::{
    ChangeSet, EditKind, LineEnding, Range, Rope, Selection,
    movement::{self, Direction},
    ops::{self, Edit, EditOptions},
    text::{line_without_ending, visual_col},
};

use super::{App, Severity};
use crate::editor::{DocId, GroupId};

/// Line-comment token by file extension (replaced by language services in M7).
pub fn comment_token(path: Option<&std::path::Path>) -> &'static str {
    let ext = path.and_then(|p| p.extension()).and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
    let name = path.and_then(|p| p.file_name()).and_then(|n| n.to_str()).unwrap_or("");
    match ext.as_str() {
        "py" | "sh" | "bash" | "zsh" | "fish" | "rb" | "pl" | "toml" | "yaml" | "yml" | "r" | "conf" | "ini" | "mk"
        | "cmake" | "nix" | "ps1" | "tf" | "dockerignore" | "gitignore" | "env" => "#",
        "sql" | "lua" | "hs" | "elm" | "ada" => "--",
        "vim" => "\"",
        "lisp" | "clj" | "el" | "scm" => ";",
        "erl" | "tex" | "m" => "%",
        "bat" | "cmd" => "REM",
        _ => match name {
            "Dockerfile" | "Makefile" | "makefile" | "CMakeLists.txt" | ".gitignore" | ".env" | ".bashrc"
            | ".zshrc" | ".gitconfig" => "#",
            _ => "//",
        },
    }
}

/// Human-readable language name by extension (status bar).
pub fn language_name(path: Option<&std::path::Path>) -> &'static str {
    let ext = path.and_then(|p| p.extension()).and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
    let name = path.and_then(|p| p.file_name()).and_then(|n| n.to_str()).unwrap_or("");
    match ext.as_str() {
        "rs" => "Rust",
        "js" | "mjs" | "cjs" => "JavaScript",
        "jsx" => "JavaScript JSX",
        "ts" | "mts" | "cts" => "TypeScript",
        "tsx" => "TypeScript JSX",
        "json" => "JSON",
        "jsonc" | "code-workspace" => "JSON with Comments",
        "yaml" | "yml" => "YAML",
        "toml" => "TOML",
        "html" | "htm" => "HTML",
        "css" => "CSS",
        "scss" => "SCSS",
        "md" | "markdown" => "Markdown",
        "sql" => "SQL",
        "sh" | "bash" | "zsh" => "Shell Script",
        "py" => "Python",
        "go" => "Go",
        "c" | "h" => "C",
        "cpp" | "cc" | "hpp" | "cxx" => "C++",
        "java" => "Java",
        "rb" => "Ruby",
        "lua" => "Lua",
        "txt" => "Plain Text",
        _ => match name {
            "Dockerfile" => "Dockerfile",
            "Makefile" | "makefile" => "Makefile",
            ".gitconfig" | ".gitattributes" | ".gitignore" => "Git",
            _ => "Plain Text",
        },
    }
}

impl App {
    /// The focused view: (group, view index, document).
    pub fn active(&self) -> Option<(GroupId, usize, DocId)> {
        let g = self.layout.active_group();
        g.active_view().map(|v| (g.id, g.active, v.doc))
    }

    pub(crate) fn edit_options(&self, doc: DocId) -> EditOptions {
        let d = self.docs.doc(doc);
        let s = self.settings_for(d.and_then(|d| d.path()));
        EditOptions {
            tab_width: s.editor.tab_size.max(1),
            insert_spaces: s.editor.insert_spaces,
            auto_close: s.editor.auto_closing_brackets,
            eol: d.map(|d| d.eol()).unwrap_or(LineEnding::Lf),
        }
    }

    fn rope_and_sel(&self) -> Option<(GroupId, usize, DocId, Rope, Selection)> {
        let (g, v, d) = self.active()?;
        let rope = self.docs.doc(d)?.rope().clone();
        let sel = self.layout.group(g)?.views[v].sel.clone();
        Some((g, v, d, rope, sel))
    }

    /// Apply an edit to the active view's document and keep every other
    /// view of that document in sync.
    pub(crate) fn apply_edit(&mut self, edit: Edit) {
        let Some((g, v, d)) = self.active() else { return };
        self.apply_edit_at(g, v, d, edit);
    }

    pub(crate) fn apply_edit_at(&mut self, g: GroupId, v: usize, d: DocId, edit: Edit) {
        if edit.changes.is_empty() {
            self.set_view_selection(g, v, edit.selection);
            return;
        }
        let Some(before) = self.layout.group(g).map(|gr| gr.views[v].sel.clone()) else { return };
        let Some(doc) = self.docs.doc_mut(d) else { return };
        let applied = doc.apply(edit.changes, &before, edit.selection, edit.kind);
        if applied.changes.changes().iter().any(|c| c.insert.contains('\r') || c.insert.contains('\n')) {
            doc.refresh_line_ending();
        }
        if let Some(gr) = self.layout.group_mut(g) {
            gr.views[v].sel = applied.selection;
            gr.views[v].preview = false;
        }
        self.layout.sync_views(d, &applied.changes, Some((g, v)));
        self.reveal_cursor = true;
        self.on_document_edited();
    }

    /// Apply changes not tied to a view (save-time transforms); every view
    /// of the document is mapped.
    pub(crate) fn apply_doc_changes(&mut self, d: DocId, cs: ChangeSet) {
        if cs.is_empty() {
            return;
        }
        let sel = self
            .layout
            .views_of(d)
            .first()
            .and_then(|(g, v)| self.layout.group(*g).map(|gr| gr.views[*v].sel.clone()))
            .unwrap_or_default();
        let after = sel.map(&cs);
        let Some(doc) = self.docs.doc_mut(d) else { return };
        let applied = doc.apply(cs, &sel, after, EditKind::Other);
        self.layout.sync_views(d, &applied.changes, None);
    }

    fn set_view_selection(&mut self, g: GroupId, v: usize, sel: Selection) {
        if let Some(gr) = self.layout.group_mut(g)
            && let Some(view) = gr.views.get_mut(v)
        {
            view.sel = sel;
        }
        self.reveal_cursor = true;
    }

    /// Apply `f` to every range of the active view (movement).
    pub(crate) fn move_cursors(&mut self, f: impl Fn(&Rope, Range) -> Range) {
        let Some((g, v, d, rope, sel)) = self.rope_and_sel() else { return };
        let new = sel.transform(|r| f(&rope, r));
        if let Some(doc) = self.docs.doc_mut(d) {
            doc.seal_history();
        }
        self.set_view_selection(g, v, new);
    }

    pub(crate) fn tab_width_active(&self) -> usize {
        self.active().map(|(_, _, d)| self.edit_options(d).tab_width).unwrap_or(4)
    }

    pub(crate) fn cmd_move(&mut self, kind: Move, extend: bool) {
        let tw = self.tab_width_active();
        let page = self.active_view_height().saturating_sub(1).max(1);
        self.move_cursors(|rope, r| match kind {
            Move::Left => movement::move_grapheme(rope, r, Direction::Backward, extend),
            Move::Right => movement::move_grapheme(rope, r, Direction::Forward, extend),
            Move::Up => movement::move_vertical(rope, r, Direction::Backward, 1, extend, tw),
            Move::Down => movement::move_vertical(rope, r, Direction::Forward, 1, extend, tw),
            Move::WordLeft => movement::move_word(rope, r, Direction::Backward, extend),
            Move::WordRight => movement::move_word(rope, r, Direction::Forward, extend),
            Move::Home => movement::line_home(rope, r, extend),
            Move::End => movement::line_end(rope, r, extend),
            Move::Top => movement::doc_start(r, extend),
            Move::Bottom => movement::doc_end(rope, r, extend),
            Move::PageUp => movement::move_vertical(rope, r, Direction::Backward, page, extend, tw),
            Move::PageDown => movement::move_vertical(rope, r, Direction::Forward, page, extend, tw),
        });
        if matches!(kind, Move::PageUp | Move::PageDown) {
            // Keep the cursor at the same screen row, like VS Code.
            let delta = page as isize * if kind == Move::PageUp { -1 } else { 1 };
            if let Some((g, v, d)) = self.active() {
                let lines = self.docs.doc(d).map(|x| x.rope().len_lines()).unwrap_or(1);
                if let Some(view) = self.layout.group_mut(g).map(|gr| &mut gr.views[v]) {
                    view.scroll_line =
                        (view.scroll_line as isize + delta).clamp(0, lines.saturating_sub(1) as isize) as usize;
                }
            }
        }
    }

    pub(crate) fn type_char(&mut self, c: char) {
        let Some((_, _, d, rope, sel)) = self.rope_and_sel() else { return };
        let opts = self.edit_options(d);
        let edit = ops::type_char(&rope, &sel, c, &opts);
        self.apply_edit(edit);
    }

    /// Insert text at the cursors (used by tests and bracketed paste).
    pub fn type_text_for_tests(&mut self, text: &str) {
        self.insert_text(text);
    }

    pub(crate) fn insert_text(&mut self, text: &str) {
        let Some((_, _, d, rope, sel)) = self.rope_and_sel() else { return };
        let opts = self.edit_options(d);
        self.apply_edit(ops::insert_text(&rope, &sel, text, &opts));
    }

    pub(crate) fn run_op(&mut self, op: Op) {
        let Some((_, _, d, rope, sel)) = self.rope_and_sel() else { return };
        let opts = self.edit_options(d);
        let path = self.docs.doc(d).and_then(|x| x.path().map(|p| p.to_path_buf()));
        let edit = match op {
            Op::Newline => ops::insert_newline(&rope, &sel, &opts),
            Op::Backspace => ops::backspace(&rope, &sel, &opts),
            Op::Delete => ops::delete_forward(&rope, &sel),
            Op::DeleteWordLeft => ops::delete_word(&rope, &sel, Direction::Backward),
            Op::DeleteWordRight => ops::delete_word(&rope, &sel, Direction::Forward),
            Op::Tab => ops::indent(&rope, &sel, &opts),
            Op::Outdent => ops::outdent(&rope, &sel, &opts),
            Op::IndentLines => ops::indent_lines(&rope, &sel, &opts),
            Op::LineBelow => ops::insert_line(&rope, &sel, Direction::Forward, &opts),
            Op::LineAbove => ops::insert_line(&rope, &sel, Direction::Backward, &opts),
            Op::MoveLinesUp => ops::move_lines(&rope, &sel, Direction::Backward),
            Op::MoveLinesDown => ops::move_lines(&rope, &sel, Direction::Forward),
            Op::CopyLinesUp => ops::copy_lines(&rope, &sel, Direction::Backward, &opts),
            Op::CopyLinesDown => ops::copy_lines(&rope, &sel, Direction::Forward, &opts),
            Op::DeleteLines => ops::delete_lines(&rope, &sel),
            Op::ToggleComment => ops::toggle_line_comment(&rope, &sel, comment_token(path.as_deref())),
            Op::TrimWhitespace => {
                let cs = ops::trim_trailing_whitespace(&rope);
                let s = sel.map(&cs);
                Edit { changes: cs, selection: s, kind: EditKind::Other }
            }
        };
        self.apply_edit(edit);
    }

    pub(crate) fn set_selection_with(&mut self, f: impl Fn(&Rope, &Selection) -> Selection) {
        let Some((g, v, d, rope, sel)) = self.rope_and_sel() else { return };
        let new = f(&rope, &sel);
        if let Some(doc) = self.docs.doc_mut(d) {
            doc.seal_history();
        }
        self.set_view_selection(g, v, new);
    }

    pub(crate) fn add_cursor(&mut self, dir: Direction) {
        let tw = self.tab_width_active();
        self.set_selection_with(|rope, sel| ops::add_cursor_vertical(rope, sel, dir, tw));
    }

    pub(crate) fn collapse_cursors(&mut self, keep_selection: bool) {
        self.set_selection_with(|_, sel| {
            let p = sel.clone().into_primary();
            if keep_selection { p } else { Selection::point(p.primary().head) }
        });
    }

    pub(crate) fn undo(&mut self, redo: bool) {
        let Some((g, v, d)) = self.active() else { return };
        let Some(doc) = self.docs.doc_mut(d) else { return };
        let result = if redo { doc.redo() } else { doc.undo() };
        let Some((changes, sel)) = result else { return };
        doc.refresh_line_ending();
        for cs in &changes {
            self.layout.sync_views(d, cs, Some((g, v)));
        }
        self.set_view_selection(g, v, sel);
        self.on_document_edited();
    }

    pub(crate) fn copy(&mut self, cut: bool) {
        let Some((_, _, d, rope, sel)) = self.rope_and_sel() else { return };
        let eol = self.edit_options(d).eol;
        let clip = if cut {
            let (clip, edit) = ops::cut(&rope, &sel, eol);
            self.apply_edit(edit);
            clip
        } else {
            ops::copy(&rope, &sel, eol)
        };
        if let Some(bytes) = self.clipboard.set(clip) {
            self.emit(super::Effect::Write(bytes));
        }
    }

    pub(crate) fn paste(&mut self) {
        let Some((_, _, d, rope, sel)) = self.rope_and_sel() else { return };
        let clip = self.clipboard.get();
        if clip.text.is_empty() {
            return;
        }
        let opts = self.edit_options(d);
        self.apply_edit(ops::paste(&rope, &sel, &clip, &opts));
    }

    pub(crate) fn jump_to_bracket(&mut self) {
        self.move_cursors(|rope, r| match ops::matching_bracket(rope, r.head) {
            Some((_, other)) => Range::point(other),
            None => r,
        });
    }

    pub(crate) fn change_eol(&mut self, to: LineEnding) {
        let Some((_, _, d, rope, _)) = self.rope_and_sel() else { return };
        let cs = ops::convert_line_endings(&rope, to);
        self.apply_doc_changes(d, cs);
        if let Some(doc) = self.docs.doc_mut(d) {
            doc.line_ending = yace_buffer::LineEndingInfo { dominant: to, mixed: false };
        }
    }

    pub(crate) fn goto_line(&mut self, text: &str) -> Result<(), String> {
        let t = text.trim().trim_start_matches(':');
        let mut parts = t.splitn(2, [':', ',']);
        let line: usize =
            parts.next().and_then(|l| l.trim().parse().ok()).ok_or("Type a line number, e.g. 42 or 42:8")?;
        let col: usize = parts.next().and_then(|c| c.trim().parse().ok()).unwrap_or(1);
        self.set_cursor_line_col(line.saturating_sub(1), col.saturating_sub(1));
        self.focus_editor();
        Ok(())
    }

    /// Place the primary cursor at a 0-based (line, char column), clamped.
    /// Move the primary cursor to a 0-based (line, column), clamped. If the
    /// document is still loading, the position is applied when it arrives.
    pub fn goto_line_col(&mut self, line: usize, col: usize) {
        self.set_cursor_line_col(line, col);
    }

    pub(crate) fn set_cursor_line_col(&mut self, line: usize, col: usize) {
        if let Some((_, _, d)) = self.active()
            && self.docs.is_loading(d)
        {
            if let Some(e) = self.docs.get_mut(d) {
                e.pending_cursor = Some((line, col));
            }
            return;
        }
        self.set_selection_with(|rope, _| {
            let line = line.min(rope.len_lines().saturating_sub(1));
            let len = line_without_ending(rope, line).len_chars();
            Selection::point(rope.line_to_char(line) + col.min(len))
        });
        self.center_active_cursor();
    }

    pub(crate) fn scroll_lines(&mut self, delta: isize) {
        let Some((g, v, d)) = self.active() else { return };
        let lines = self.docs.doc(d).map(|x| x.rope().len_lines()).unwrap_or(1);
        if let Some(view) = self.layout.group_mut(g).map(|gr| &mut gr.views[v]) {
            view.scroll_line = (view.scroll_line as isize + delta).clamp(0, lines.saturating_sub(1) as isize) as usize;
        }
    }

    /// Height (rows) of the active group's text area.
    pub fn active_view_height(&self) -> usize {
        let wl = crate::ui::workbench_layout(self);
        wl.groups.iter().find(|g| g.id == self.layout.active).map(|g| g.content.height as usize).unwrap_or(20)
    }

    pub(crate) fn center_active_cursor(&mut self) {
        let h = self.active_view_height();
        let Some((g, v, d)) = self.active() else { return };
        let Some(rope) = self.docs.doc(d).map(|x| x.rope().clone()) else { return };
        if let Some(view) = self.layout.group_mut(g).map(|gr| &mut gr.views[v]) {
            let line = rope.char_to_line(view.sel.primary().head.min(rope.len_chars()));
            view.scroll_line = line.saturating_sub(h / 2);
        }
        self.reveal_cursor = true;
    }

    /// Keep the primary cursor visible (with `cursorSurroundingLines`).
    pub(crate) fn scroll_active_to_cursor(&mut self) {
        let wl = crate::ui::workbench_layout(self);
        let gid = self.layout.active;
        let Some(gl) = wl.groups.iter().find(|g| g.id == gid) else { return };
        let (height, width) = (gl.content.height as usize, gl.text_width as usize);
        if height == 0 || width == 0 {
            return;
        }
        let Some((g, v, d)) = self.active() else { return };
        let margin = self.settings.editor.cursor_surrounding_lines.min(height.saturating_sub(1) / 2);
        let tw = self.edit_options(d).tab_width;
        let Some(rope) = self.docs.doc(d).map(|x| x.rope().clone()) else { return };
        let Some(view) = self.layout.group_mut(g).map(|gr| &mut gr.views[v]) else { return };
        let head = view.sel.primary().head.min(rope.len_chars());
        let line = rope.char_to_line(head);
        if line < view.scroll_line + margin {
            view.scroll_line = line.saturating_sub(margin);
        } else if line + margin >= view.scroll_line + height {
            view.scroll_line = (line + margin + 1).saturating_sub(height);
        }
        let col = visual_col(line_without_ending(&rope, line), head - rope.line_to_char(line), tw);
        if col < view.scroll_col {
            view.scroll_col = col.saturating_sub(4);
        } else if col >= view.scroll_col + width {
            view.scroll_col = col + 1 - width + 4.min(width / 4);
        }
    }

    pub(crate) fn not_available(&mut self, what: &str, milestone: &str) {
        self.notify(Severity::Info, format!("{what} is not available yet (planned for milestone {milestone})."));
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Move {
    Left,
    Right,
    Up,
    Down,
    WordLeft,
    WordRight,
    Home,
    End,
    Top,
    Bottom,
    PageUp,
    PageDown,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Op {
    Newline,
    Backspace,
    Delete,
    DeleteWordLeft,
    DeleteWordRight,
    Tab,
    Outdent,
    IndentLines,
    LineBelow,
    LineAbove,
    MoveLinesUp,
    MoveLinesDown,
    CopyLinesUp,
    CopyLinesDown,
    DeleteLines,
    ToggleComment,
    TrimWhitespace,
}
