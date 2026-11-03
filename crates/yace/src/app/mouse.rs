//! Mouse support (where the terminal reports it): clicks, double/triple
//! clicks, drag selection, wheel scrolling, tab and split interaction,
//! sidebar resizing.

use std::time::{Duration, Instant};

use ratatui::{
    crossterm::event::{KeyModifiers, MouseButton, MouseEvent, MouseEventKind},
    layout::{Position, Rect},
};
use yace_buffer::{
    Range, Selection,
    movement::{line_range, word_at},
};

use super::{App, Focus};
use crate::{
    editor::{GroupId, SplitHandle},
    overlay::{Overlay, OverlayOutcome},
    ui::{self, overlay_geometry, tab_cells},
};

#[derive(Debug, Clone)]
pub enum Drag {
    Sidebar,
    Panel,
    /// Selecting text in the terminal.
    TermSelect,
    /// Forwarding drags to a terminal application (mouse mode).
    TermReport,
    Split(SplitHandle),
    Select {
        group: GroupId,
        anchor: Range,
    },
}

#[derive(Debug, Clone, Default)]
pub struct MouseState {
    pub drag: Option<Drag>,
    /// Last pointer position (fold markers show on hover).
    pub hover: Option<(u16, u16)>,
    last_click: Option<(Instant, u16, u16, u8)>,
}

const MULTI_CLICK: Duration = Duration::from_millis(450);

fn hit(r: Rect, x: u16, y: u16) -> bool {
    r.contains(Position { x, y })
}

impl App {
    pub(crate) fn on_mouse(&mut self, m: MouseEvent) {
        let (x, y) = (m.column, m.row);
        self.mouse.hover = Some((x, y));
        if !self.overlays.is_empty() {
            self.mouse_overlay(m);
            return;
        }
        let wl = ui::workbench_layout(self);
        match m.kind {
            MouseEventKind::Down(button) => {
                let count = self.click_count(x, y);
                if let Some(b) = wl.sidebar_border
                    && hit(b, x, y)
                    && button == MouseButton::Left
                {
                    self.mouse.drag = Some(Drag::Sidebar);
                    return;
                }
                if let Some(b) = wl.panel_border
                    && hit(b, x, y)
                {
                    self.mouse.drag = Some(Drag::Panel);
                    return;
                }
                if let Some(strip) = wl.panel_tabs
                    && hit(strip, x, y)
                {
                    self.mouse_panel_tabs(strip, x, button);
                    return;
                }
                if let Some(content) = wl.panel_content
                    && hit(content, x, y)
                {
                    self.mouse_terminal(content, m, count);
                    return;
                }
                if let Some(h) = wl.handles.iter().find(|h| hit(h.rect, x, y)) {
                    self.mouse.drag = Some(Drag::Split(h.clone()));
                    return;
                }
                if let Some(list) = wl.explorer_list
                    && hit(list, x, y)
                {
                    if self.sidebar_view == super::SidebarView::Plugin && self.plugins.active_view.is_some() {
                        self.focus_changed(super::Focus::Explorer);
                        let ui = self.plugin_view_ui();
                        let height = list.height as usize;
                        let scroll = if ui.selected >= height { ui.selected + 1 - height } else { 0 };
                        let row = scroll + (y - list.y) as usize;
                        if row < self.plugin_view_rows().len() {
                            self.open_plugin_view_row(row);
                        }
                    } else if self.sidebar_view == super::SidebarView::Search {
                        self.mouse_search(list, x, y, count);
                    } else {
                        self.mouse_explorer(list, y, count, button);
                    }
                    return;
                }
                if let Some(t) = wl.explorer_title
                    && hit(t, x, y)
                {
                    self.focus_explorer();
                    return;
                }
                // The find bar sits on top of the active group's text.
                if self.find.visible
                    && let Some(g) = wl.groups.iter().find(|g| g.id == self.layout.active)
                    && let Some(fb) = ui::find_bar_layout(self, g)
                    && (hit(fb.row, x, y) || fb.replace_row.is_some_and(|r| hit(r, x, y)))
                {
                    self.mouse_find_bar(&fb, x, y);
                    return;
                }
                for g in &wl.groups {
                    if hit(g.tabs, x, y) {
                        self.mouse_tabs(g.id, g.tabs, x, button, count);
                        return;
                    }
                    if hit(g.content, x, y) && button == MouseButton::Left {
                        self.mouse_editor(g, x, y, count, m.modifiers);
                        return;
                    }
                }
            }
            MouseEventKind::Drag(MouseButton::Left) => match self.mouse.drag.clone() {
                Some(Drag::Sidebar) => {
                    let max = self.area.width.saturating_sub(20).max(10);
                    let w = if self.settings.workbench.side_bar_location == yace_config::SideBarLocation::Right {
                        self.area.right().saturating_sub(x + 1)
                    } else {
                        x.saturating_sub(self.area.x)
                    };
                    self.sidebar_width = w.clamp(10, max);
                }
                Some(Drag::Split(h)) => self.layout.drag_handle(&h, x, y),
                Some(Drag::Panel) => {
                    let bottom = self.area.bottom().saturating_sub(1);
                    self.panel.height =
                        bottom.saturating_sub(y).saturating_sub(2).clamp(3, self.area.height.saturating_sub(8).max(3));
                    self.panel.maximized = false;
                }
                Some(Drag::TermSelect) | Some(Drag::TermReport) => {
                    if let Some(content) = wl.panel_content {
                        self.mouse_terminal(content, m, 1);
                    }
                }
                Some(Drag::Select { group, anchor }) => {
                    if let Some(g) = wl.groups.iter().find(|g| g.id == group)
                        && let Some(pos) = self.position_at(g, x, y)
                    {
                        let sel = if pos >= anchor.from() {
                            Range::new(anchor.from(), pos.max(anchor.to()))
                        } else {
                            Range::new(anchor.to(), pos)
                        };
                        self.set_active_selection(Selection::single(sel));
                    }
                }
                None => {}
            },
            MouseEventKind::Up(_) => {
                if matches!(self.mouse.drag, Some(Drag::TermReport))
                    && let Some(content) = wl.panel_content
                {
                    self.mouse_terminal(content, m, 1);
                }
                self.mouse.drag = None;
            }
            MouseEventKind::ScrollDown | MouseEventKind::ScrollUp => {
                let delta: isize = if m.kind == MouseEventKind::ScrollDown { 3 } else { -3 };
                if let Some(content) = wl.panel_content
                    && hit(content, x, y)
                {
                    self.mouse_terminal(content, m, 1);
                    return;
                }
                if wl.sidebar.is_some_and(|s| hit(s, x, y)) && self.sidebar_view == super::SidebarView::Plugin {
                    let n = self.plugin_view_rows().len();
                    if let Some(key) = self.plugins.active_view.clone() {
                        let ui = self.plugins.view_ui.entry(key).or_default();
                        ui.selected = (ui.selected as isize + delta).clamp(0, n.saturating_sub(1) as isize) as usize;
                    }
                    return;
                }
                if wl.sidebar.is_some_and(|s| hit(s, x, y)) {
                    let max = self
                        .explorer
                        .rows
                        .len()
                        .saturating_sub(wl.explorer_list.map(|l| l.height as usize).unwrap_or(0));
                    self.explorer.scroll = (self.explorer.scroll as isize + delta).clamp(0, max as isize) as usize;
                    // Keep the selection inside the viewport so clamping doesn't undo the scroll.
                    let h = wl.explorer_list.map(|l| l.height as usize).unwrap_or(1).max(1);
                    let s = self.explorer.scroll;
                    self.explorer.selected =
                        self.explorer.selected.clamp(s, s + h - 1).min(self.explorer.rows.len().saturating_sub(1));
                    return;
                }
                if let Some(g) = wl.groups.iter().find(|g| hit(g.rect, x, y)) {
                    let gid = g.id;
                    if m.modifiers.contains(KeyModifiers::SHIFT) {
                        if let Some(view) = self.layout.group_mut(gid).and_then(|gr| gr.active_view_mut()) {
                            view.scroll_col = (view.scroll_col as isize + delta * 2).max(0) as usize;
                        }
                    } else {
                        self.scroll_view(gid, delta);
                    }
                }
            }
            MouseEventKind::ScrollLeft | MouseEventKind::ScrollRight => {
                if let Some(g) = wl.groups.iter().find(|g| hit(g.rect, x, y))
                    && let Some(view) = self.layout.group_mut(g.id).and_then(|gr| gr.active_view_mut())
                {
                    let d: isize = if m.kind == MouseEventKind::ScrollRight { 4 } else { -4 };
                    view.scroll_col = (view.scroll_col as isize + d).max(0) as usize;
                }
            }
            _ => {}
        }
    }

    fn click_count(&mut self, x: u16, y: u16) -> u8 {
        let now = Instant::now();
        let count = match self.mouse.last_click {
            Some((t, lx, ly, c)) if now.duration_since(t) < MULTI_CLICK && lx == x && ly == y => (c % 3) + 1,
            _ => 1,
        };
        self.mouse.last_click = Some((now, x, y, count));
        count
    }

    fn mouse_explorer(&mut self, list: Rect, y: u16, count: u8, button: MouseButton) {
        self.focus_explorer();
        let idx = self.explorer.scroll + (y - list.y) as usize;
        if idx >= self.explorer.rows.len() {
            return;
        }
        self.explorer.selected = idx;
        if button != MouseButton::Left {
            return;
        }
        let row = self.explorer.rows[idx].clone();
        if row.is_dir_like() {
            if count == 1 {
                self.explorer.toggle(idx);
            }
        } else if row.is_file() {
            if count >= 2 {
                self.open_file(&row.path, false);
            } else {
                self.open_file(&row.path, true);
                self.focus = Focus::Explorer;
            }
        }
    }

    fn mouse_panel_tabs(&mut self, strip: Rect, x: u16, button: MouseButton) {
        let (cells, plus) = ui::panel_tab_cells(self, strip);
        if plus.is_some_and(|p| x >= p.x && x < p.x + p.width) {
            self.new_terminal_default();
            return;
        }
        if let Some((i, _, _, close_x)) = cells.into_iter().find(|(_, cx, w, _)| x >= *cx && x < cx + w) {
            self.panel.active = i;
            if button == MouseButton::Middle || x == close_x {
                self.kill_terminal_requested();
            } else {
                self.ensure_active_started();
                self.focus_changed(Focus::Terminal);
            }
        }
    }

    /// Clicks, drags and wheel in the terminal area: selection and
    /// scrollback, or SGR/X10 reports when the application wants mouse input.
    fn mouse_terminal(&mut self, content: Rect, m: MouseEvent, count: u8) {
        use yace_terminal::{MouseButton as TB, MouseKind as TK, encode_key, encode_mouse};
        if !matches!(m.kind, MouseEventKind::ScrollUp | MouseEventKind::ScrollDown | MouseEventKind::Moved) {
            self.focus_changed(Focus::Terminal);
        }
        let row = m.row.saturating_sub(content.y) as usize;
        let col = m.column.saturating_sub(content.x) as usize;
        let mut mods = yace_keymap::Mods::NONE;
        if m.modifiers.contains(KeyModifiers::SHIFT) {
            mods = mods | yace_keymap::Mods::SHIFT;
        }
        if m.modifiers.contains(KeyModifiers::ALT) {
            mods = mods | yace_keymap::Mods::ALT;
        }
        if m.modifiers.contains(KeyModifiers::CONTROL) {
            mods = mods | yace_keymap::Mods::CTRL;
        }
        let Some(s) = self.panel.active_tab().and_then(|t| t.session.as_ref()) else { return };
        let modes = s.modes();
        // Shift bypasses mouse reporting (select text in a TUI app).
        let report = modes.any_mouse() && !m.modifiers.contains(KeyModifiers::SHIFT);
        let button = |b: MouseButton| match b {
            MouseButton::Left => TB::Left,
            MouseButton::Middle => TB::Middle,
            MouseButton::Right => TB::Right,
        };
        match m.kind {
            MouseEventKind::Down(b) => {
                if report {
                    if let Some(bytes) = encode_mouse(button(b), TK::Press, row, col, mods, modes) {
                        s.write(&bytes);
                    }
                    self.mouse.drag = Some(Drag::TermReport);
                } else if b == MouseButton::Left {
                    s.select_clear();
                    s.select_start(row, col, count >= 2);
                    self.mouse.drag = Some(Drag::TermSelect);
                } else if b == MouseButton::Middle {
                    self.terminal_paste();
                }
            }
            MouseEventKind::Drag(b) => {
                if report {
                    if let Some(bytes) = encode_mouse(button(b), TK::Drag, row, col, mods, modes) {
                        s.write(&bytes);
                    }
                } else {
                    s.select_update(row, col);
                }
            }
            MouseEventKind::Up(b) => {
                if report && let Some(bytes) = encode_mouse(button(b), TK::Release, row, col, mods, modes) {
                    s.write(&bytes);
                }
            }
            MouseEventKind::ScrollUp | MouseEventKind::ScrollDown => {
                let up = m.kind == MouseEventKind::ScrollUp;
                if report {
                    let wb = if up { TB::WheelUp } else { TB::WheelDown };
                    if let Some(bytes) = encode_mouse(wb, TK::Press, row, col, mods, modes) {
                        s.write(&bytes);
                    }
                } else if modes.alt_screen && modes.alternate_scroll {
                    // Full-screen apps without mouse mode: send arrow keys.
                    let key = yace_keymap::KeyPress::new(
                        if up { yace_keymap::KeyCode::Up } else { yace_keymap::KeyCode::Down },
                        yace_keymap::Mods::NONE,
                    );
                    if let Some(bytes) = encode_key(key, None, modes) {
                        for _ in 0..3 {
                            s.write(&bytes);
                        }
                    }
                } else {
                    s.scroll(if up { 3 } else { -3 });
                }
            }
            _ => {}
        }
    }

    fn mouse_find_bar(&mut self, fb: &ui::FindBarLayout, x: u16, y: u16) {
        if hit(fb.close, x, y) {
            self.find_close();
            return;
        }
        if let Some((c, _)) = fb.toggles.iter().find(|(_, r)| hit(*r, x, y)) {
            self.find_toggle(*c);
            return;
        }
        self.find.field = if fb.replace_row.is_some_and(|r| hit(r, x, y)) {
            super::FindField::Replace
        } else {
            super::FindField::Query
        };
        self.focus_changed(Focus::Find);
    }

    fn mouse_search(&mut self, list: Rect, x: u16, y: u16, count: u8) {
        self.focus_changed(Focus::Explorer);
        let lay = ui::search_layout(self, list);
        if let Some((f, _)) = lay.fields.iter().find(|(_, r)| hit(*r, x, y)) {
            self.search.field = *f;
            return;
        }
        if hit(lay.results, x, y) {
            let scroll = ui::search_scroll(self, lay.results.height as usize);
            let row = scroll + (y - lay.results.y) as usize;
            if row < self.search.rows().len() {
                self.search_click_row(row, count >= 2);
            }
        }
    }

    fn mouse_tabs(&mut self, gid: GroupId, tabs: Rect, x: u16, button: MouseButton, count: u8) {
        let Some(group) = self.layout.group(gid) else { return };
        let cells = tab_cells(self, group, tabs);
        let Some(cell) = cells.iter().find(|c| x >= c.x && x < c.x + c.width) else { return };
        let (view, close_x) = (cell.view, cell.close_x);
        let doc = group.views[view].doc;
        self.layout.active = gid;
        if button == MouseButton::Middle || (button == MouseButton::Left && x == close_x) {
            self.close_views(gid, vec![doc]);
            return;
        }
        if let Some(g) = self.layout.group_mut(gid) {
            g.activate(view);
            if count >= 2 {
                g.views[view].preview = false;
            }
        }
        self.focus_editor();
    }

    /// Char index under a screen position in a group's text area.
    /// The display row under screen row `y` of a group.
    fn row_at(&self, g: &ui::GroupLayout, y: u16) -> Option<(crate::display::Row, bool)> {
        let group = self.layout.group(g.id)?;
        let geo = self.view_geo(g.id, group.active)?;
        let view = group.active_view()?;
        let map = geo.map();
        let dy = y.saturating_sub(g.content.y) as usize;
        let rows = map.rows(view.top(), dy + 1);
        let row = *rows.last()?;
        Some((row, rows.len() <= dy))
    }

    fn position_at(&self, g: &ui::GroupLayout, x: u16, y: u16) -> Option<usize> {
        let group = self.layout.group(g.id)?;
        let geo = self.view_geo(g.id, group.active)?;
        let view = group.active_view()?;
        let map = geo.map();
        let (row, past_end) = self.row_at(g, y)?;
        if past_end {
            return Some(geo.rope.len_chars());
        }
        let scroll_col = if map.wraps() { 0 } else { view.scroll_col };
        Some(map.char_at(row, scroll_col + x.saturating_sub(g.text_x) as usize))
    }

    fn mouse_editor(&mut self, g: &ui::GroupLayout, x: u16, y: u16, count: u8, mods: KeyModifiers) {
        self.layout.active = g.id;
        self.focus_editor();
        // The fold marker column of the gutter toggles folds.
        if x == g.content.x + g.gutter.saturating_sub(2)
            && x < g.text_x
            && let Some((row, false)) = self.row_at(g, y)
            && row.sub == 0
            && self.toggle_fold_line(g.id, row.line)
        {
            return;
        }
        let Some(pos) = self.position_at(g, x, y) else { return };
        let Some((_, v, d)) = self.active() else { return };
        let Some(rope) = self.docs.doc(d).map(|x| x.rope().clone()) else { return };
        let current = self.layout.active_group().views[v].sel.clone();
        let range = match count {
            2 => word_at(&rope, pos),
            3 => line_range(&rope, rope.char_to_line(pos)),
            _ => Range::point(pos),
        };
        let prev_anchor = current.primary().anchor;
        let sel = if mods.contains(KeyModifiers::ALT) {
            current.push(range)
        } else if mods.contains(KeyModifiers::SHIFT) {
            Selection::single(Range::new(prev_anchor, pos))
        } else {
            Selection::single(range)
        };
        if mods.contains(KeyModifiers::CONTROL) && count == 1 {
            // Ctrl+Click: go to definition (VS Code).
            if let Some(doc) = self.docs.doc_mut(d) {
                doc.seal_history();
            }
            self.set_active_selection(Selection::point(pos));
            self.lsp_goto("textDocument/definition", "Definition");
            return;
        }
        self.mouse.drag = Some(Drag::Select {
            group: g.id,
            anchor: if mods.contains(KeyModifiers::SHIFT) { Range::point(prev_anchor) } else { range },
        });
        if let Some(doc) = self.docs.doc_mut(d) {
            doc.seal_history();
        }
        self.set_active_selection(sel);
    }

    pub(crate) fn set_active_selection(&mut self, sel: Selection) {
        if let Some(view) = self.layout.active_group_mut().active_view_mut() {
            view.sel = sel;
        }
        self.reveal_cursor = true;
    }

    fn mouse_overlay(&mut self, m: MouseEvent) {
        let Some(top) = self.overlays.last() else { return };
        let geom = overlay_geometry(self.area, top);
        let (x, y) = (m.column, m.row);
        match m.kind {
            MouseEventKind::Down(MouseButton::Left) => {
                if !hit(geom.outer, x, y) {
                    if !matches!(top, Overlay::Confirm(_)) {
                        self.on_overlay_outcome(OverlayOutcome::Cancel);
                    }
                    return;
                }
                if let Some((_, i)) = geom.buttons.iter().find(|(r, _)| hit(*r, x, y))
                    && let Some(Overlay::Confirm(c)) = self.overlays.last()
                {
                    let action = c.buttons[*i].1.clone();
                    self.on_overlay_outcome(OverlayOutcome::Confirm(action));
                    return;
                }
                if let Some(list) = geom.list
                    && hit(list, x, y)
                    && let Some(Overlay::Keybindings(e)) = self.overlays.last_mut()
                {
                    let height = list.height as usize;
                    let scroll = if e.selected >= height { e.selected + 1 - height } else { 0 };
                    let idx = scroll + (y - list.y) as usize;
                    if idx < e.filtered.len() {
                        e.selected = idx;
                    }
                    return;
                }
                if let Some(list) = geom.list
                    && hit(list, x, y)
                    && let Some(Overlay::Picker(p)) = self.overlays.last_mut()
                {
                    let height = list.height as usize;
                    let scroll = if p.selected >= height { p.selected + 1 - height } else { 0 };
                    let idx = scroll + (y - list.y) as usize;
                    if idx < p.filtered.len() {
                        p.selected = idx;
                        let data = p.items[p.filtered[idx]].data;
                        self.on_overlay_outcome(OverlayOutcome::Pick(data));
                    }
                }
            }
            MouseEventKind::ScrollDown | MouseEventKind::ScrollUp => {
                let down = m.kind == MouseEventKind::ScrollDown;
                match self.overlays.last_mut() {
                    Some(Overlay::Picker(p)) => p.move_by(if down { 1 } else { -1 }),
                    Some(Overlay::Keybindings(e)) => {
                        let n = e.filtered.len();
                        e.selected =
                            if down { (e.selected + 3).min(n.saturating_sub(1)) } else { e.selected.saturating_sub(3) };
                    }
                    _ => {}
                }
            }
            _ => {}
        }
    }
}
