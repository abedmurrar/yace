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
    text::{char_at_visual_col, line_without_ending},
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
    Split(SplitHandle),
    Select { group: GroupId, anchor: Range },
}

#[derive(Debug, Clone, Default)]
pub struct MouseState {
    pub drag: Option<Drag>,
    last_click: Option<(Instant, u16, u16, u8)>,
}

const MULTI_CLICK: Duration = Duration::from_millis(450);

fn hit(r: Rect, x: u16, y: u16) -> bool {
    r.contains(Position { x, y })
}

impl App {
    pub(crate) fn on_mouse(&mut self, m: MouseEvent) {
        let (x, y) = (m.column, m.row);
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
                if let Some(h) = wl.handles.iter().find(|h| hit(h.rect, x, y)) {
                    self.mouse.drag = Some(Drag::Split(h.clone()));
                    return;
                }
                if let Some(list) = wl.explorer_list
                    && hit(list, x, y)
                {
                    self.mouse_explorer(list, y, count, button);
                    return;
                }
                if let Some(t) = wl.explorer_title
                    && hit(t, x, y)
                {
                    self.focus_explorer();
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
                    self.sidebar_width = x.saturating_sub(self.area.x).clamp(10, max);
                }
                Some(Drag::Split(h)) => self.layout.drag_handle(&h, x, y),
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
            MouseEventKind::Up(_) => self.mouse.drag = None,
            MouseEventKind::ScrollDown | MouseEventKind::ScrollUp => {
                let delta: isize = if m.kind == MouseEventKind::ScrollDown { 3 } else { -3 };
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
                    if let Some(group) = self.layout.group(gid)
                        && let Some(v) = group.active_view()
                    {
                        let lines = self.docs.doc(v.doc).map(|d| d.rope().len_lines()).unwrap_or(1);
                        if let Some(view) = self.layout.group_mut(gid).and_then(|gr| gr.active_view_mut()) {
                            if m.modifiers.contains(KeyModifiers::SHIFT) {
                                view.scroll_col = (view.scroll_col as isize + delta * 2).max(0) as usize;
                            } else {
                                view.scroll_line = (view.scroll_line as isize + delta)
                                    .clamp(0, lines.saturating_sub(1) as isize)
                                    as usize;
                            }
                        }
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
    fn position_at(&self, g: &ui::GroupLayout, x: u16, y: u16) -> Option<usize> {
        let group = self.layout.group(g.id)?;
        let view = group.active_view()?;
        let doc = self.docs.doc(view.doc)?;
        let rope = doc.rope();
        let line = (view.scroll_line + y.saturating_sub(g.content.y) as usize).min(rope.len_lines().saturating_sub(1));
        let col = view.scroll_col + x.saturating_sub(g.text_x) as usize;
        let tw = self.settings_for(doc.path()).editor.tab_size.max(1);
        Some(rope.line_to_char(line) + char_at_visual_col(line_without_ending(rope, line), col, tw))
    }

    fn mouse_editor(&mut self, g: &ui::GroupLayout, x: u16, y: u16, count: u8, mods: KeyModifiers) {
        self.layout.active = g.id;
        self.focus_editor();
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
        self.mouse.drag = Some(Drag::Select {
            group: g.id,
            anchor: if mods.contains(KeyModifiers::SHIFT) { Range::point(prev_anchor) } else { range },
        });
        if let Some(doc) = self.docs.doc_mut(d) {
            doc.seal_history();
        }
        self.set_active_selection(sel);
    }

    fn set_active_selection(&mut self, sel: Selection) {
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
                if let Some(Overlay::Picker(p)) = self.overlays.last_mut() {
                    p.move_by(if m.kind == MouseEventKind::ScrollDown { 1 } else { -1 });
                }
            }
            _ => {}
        }
    }
}
