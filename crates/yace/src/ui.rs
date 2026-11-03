//! Rendering. Layout geometry is computed by pure functions shared with
//! mouse hit-testing, so what is drawn is exactly what is clickable.
//! Only visible lines are rendered (spec §22.3).

use std::collections::HashMap;

use ratatui::{
    Frame,
    buffer::Buffer,
    layout::{Position, Rect},
    style::{Modifier, Style},
};
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;
use yace_buffer::{
    DiskState,
    ops::matching_bracket,
    text::{grapheme_width, line_without_ending},
};
use yace_config::LineNumbers;

use crate::{
    app::{App, Focus, Severity, SidebarView},
    editor::{Group, GroupId, SplitHandle},
    explorer::RowKind,
    overlay::Overlay,
};

#[derive(Debug, Clone)]
pub struct GroupLayout {
    pub id: GroupId,
    pub rect: Rect,
    pub tabs: Rect,
    /// Text area including the gutter.
    pub content: Rect,
    pub gutter: u16,
    pub text_x: u16,
    pub text_width: u16,
}

#[derive(Debug, Clone)]
pub struct WorkbenchLayout {
    pub sidebar: Option<Rect>,
    pub sidebar_border: Option<Rect>,
    pub explorer_title: Option<Rect>,
    pub explorer_list: Option<Rect>,
    pub editor: Rect,
    pub groups: Vec<GroupLayout>,
    pub handles: Vec<SplitHandle>,
    pub status: Rect,
    /// Bottom panel: drag border, tab strip, terminal content.
    pub panel_border: Option<Rect>,
    pub panel_tabs: Option<Rect>,
    pub panel_content: Option<Rect>,
}

fn digits(n: usize) -> u16 {
    (n.max(1).ilog10() + 1) as u16
}

pub fn workbench_layout(app: &App) -> WorkbenchLayout {
    let area = app.area;
    let status = Rect { x: area.x, y: area.bottom().saturating_sub(1), width: area.width, height: area.height.min(1) };
    let main = Rect { height: area.height.saturating_sub(1), ..area };
    let narrow = main.width < 50;
    let show_sidebar = app.sidebar_visible && (!narrow || app.focus == Focus::Explorer || app.active().is_none());
    let (sidebar, sidebar_border, editor) = if show_sidebar && main.width >= 12 {
        let w = if narrow {
            main.width.saturating_sub(1)
        } else {
            app.sidebar_width.min(main.width.saturating_sub(21)).max(10)
        };
        let sb = Rect { width: w, ..main };
        let border = Rect { x: main.x + w, width: 1, ..main };
        let ed = Rect { x: main.x + w + 1, width: main.width.saturating_sub(w + 1), ..main };
        (Some(sb), Some(border), ed)
    } else {
        (None, None, main)
    };
    // Bottom panel below the editor groups (the sidebar stays full height).
    let (editor, panel_border, panel_tabs, panel_content) = if app.panel.visible && editor.height >= 6 {
        let total = if app.panel.maximized {
            editor.height
        } else {
            (app.panel.height.max(3) + 2).min(editor.height.saturating_sub(4)).max(4)
        };
        let border = Rect { y: editor.bottom() - total, height: 1, ..editor };
        let tabs = Rect { y: border.y + 1, height: 1, ..editor };
        let content = Rect { y: tabs.y + 1, height: total.saturating_sub(2), ..editor };
        (Rect { height: editor.height - total, ..editor }, Some(border), Some(tabs), Some(content))
    } else {
        (editor, None, None, None)
    };
    let explorer_title = sidebar.map(|s| Rect { height: s.height.min(1), ..s });
    let explorer_list = sidebar.map(|s| Rect { y: s.y + 1, height: s.height.saturating_sub(1), ..s });
    let (rects, handles) = app.layout.rects(editor);
    let groups = rects
        .into_iter()
        .map(|(id, rect)| {
            let tabs = Rect { height: rect.height.min(1), ..rect };
            let content = Rect { y: rect.y + tabs.height, height: rect.height.saturating_sub(tabs.height), ..rect };
            let lines = app
                .layout
                .group(id)
                .and_then(|g| g.active_view())
                .and_then(|v| app.docs.doc(v.doc))
                .map(|d| d.rope().len_lines())
                .unwrap_or(1);
            let gutter = match app.settings.editor.line_numbers {
                LineNumbers::Off => 1,
                _ => digits(lines).max(3) + 2,
            }
            .min(content.width / 3);
            let text_x = content.x + gutter;
            let text_width = content.width.saturating_sub(gutter + 1);
            GroupLayout { id, rect, tabs, content, gutter, text_x, text_width }
        })
        .collect();
    WorkbenchLayout {
        sidebar,
        sidebar_border,
        explorer_title,
        explorer_list,
        editor,
        groups,
        handles,
        status,
        panel_border,
        panel_tabs,
        panel_content,
    }
}

pub fn explorer_list_height(app: &App) -> u16 {
    workbench_layout(app).explorer_list.map(|r| r.height).unwrap_or(0)
}

/// A tab's horizontal extent in the tab bar.
#[derive(Debug, Clone)]
pub struct TabCell {
    pub view: usize,
    pub x: u16,
    pub width: u16,
    pub label: String,
    pub close_x: u16,
}

/// Labels for tabs, disambiguated when file names collide (spec §6.7).
pub fn tab_label(app: &App, view_doc: crate::editor::DocId, names: &HashMap<String, usize>) -> String {
    let name = app.docs.name(view_doc);
    if names.get(&name).copied().unwrap_or(0) > 1
        && let Some(path) = app.docs.doc(view_doc).and_then(|d| d.path())
    {
        if let Some((root, rel)) = app.workspace.relative(path) {
            let parent = rel.parent().map(|p| p.display().to_string()).filter(|p| !p.is_empty());
            return match parent {
                Some(p) => format!("{name} — {} / {p}", root.display_name()),
                None => format!("{name} — {}", root.display_name()),
            };
        }
        if let Some(parent) = path.parent().and_then(|p| p.file_name()) {
            return format!("{name} — {}", parent.to_string_lossy());
        }
    }
    name
}

pub fn name_counts(app: &App) -> HashMap<String, usize> {
    let mut m = HashMap::new();
    for (id, _) in app.docs.iter() {
        *m.entry(app.docs.name(id)).or_insert(0) += 1;
    }
    m
}

pub fn tab_cells(app: &App, group: &Group, area: Rect) -> Vec<TabCell> {
    if group.views.is_empty() || area.width == 0 {
        return Vec::new();
    }
    let names = name_counts(app);
    let mut widths: Vec<(usize, String, u16)> = group
        .views
        .iter()
        .enumerate()
        .map(|(i, v)| {
            let label = tab_label(app, v.doc, &names);
            let w = (UnicodeWidthStr::width(label.as_str()) as u16 + 4).min(area.width.max(8));
            (i, label, w)
        })
        .collect();
    // Scroll so the active tab is visible.
    let mut start = group.tab_scroll.min(widths.len().saturating_sub(1));
    if group.active < start {
        start = group.active;
    }
    loop {
        let used: u16 = widths[start..=group.active.min(widths.len().saturating_sub(1))].iter().map(|w| w.2).sum();
        if used <= area.width || start >= group.active {
            break;
        }
        start += 1;
    }
    let mut x = area.x;
    let mut out = Vec::new();
    for (i, label, w) in widths.drain(start..) {
        if x >= area.right() {
            break;
        }
        let w = w.min(area.right() - x);
        out.push(TabCell { view: i, x, width: w, label, close_x: x + w.saturating_sub(2) });
        x += w;
    }
    out
}

// ---------------------------------------------------------------------------

pub fn draw(app: &App, f: &mut Frame) {
    let area = f.area();
    let theme = &app.theme;
    let buf = f.buffer_mut();
    fill(buf, area, theme.base());
    let wl = workbench_layout(app);
    draw_sidebar(app, buf, &wl);
    for g in &wl.groups {
        draw_group(app, buf, g);
    }
    for h in &wl.handles {
        let glyph = match h.axis {
            yace_workspace::session::SplitAxis::Horizontal => app.glyphs.vbar,
            yace_workspace::session::SplitAxis::Vertical => app.glyphs.hbar,
        };
        fill_with(buf, h.rect, glyph, Style::new().fg(theme.border).bg(theme.bg));
    }
    draw_panel(app, buf, &wl);
    draw_status(app, buf, wl.status);
    let mut cursor = None;
    if app.overlays.is_empty() {
        cursor = match app.focus {
            Focus::Editor => editor_cursor(app, &wl),
            Focus::Find => wl.groups.iter().find(|g| g.id == app.layout.active).and_then(|g| find_bar_cursor(app, g)),
            Focus::Explorer if app.sidebar_view == SidebarView::Search => {
                wl.explorer_list.and_then(|l| search_view_cursor(app, l))
            }
            Focus::Explorer => None,
            Focus::Terminal => terminal_cursor(app, &wl),
        };
    }
    if let Some(o) = app.overlays.last() {
        cursor = draw_overlay(app, buf, area, o);
    }
    if let Some((x, y)) = cursor {
        f.set_cursor_position(Position { x, y });
    }
}

fn fill(buf: &mut Buffer, r: Rect, style: Style) {
    fill_with(buf, r, " ", style);
}

fn fill_with(buf: &mut Buffer, r: Rect, s: &str, style: Style) {
    let r = r.intersection(buf.area);
    for y in r.top()..r.bottom() {
        for x in r.left()..r.right() {
            if let Some(c) = buf.cell_mut((x, y)) {
                c.set_symbol(s);
                c.set_style(style);
            }
        }
    }
}

/// Write `text` clipped to `max` cells; returns cells written.
fn put(buf: &mut Buffer, x: u16, y: u16, text: &str, max: u16, style: Style) -> u16 {
    if max == 0 || y >= buf.area.bottom() {
        return 0;
    }
    let (end_x, _) = buf.set_stringn(x, y, text, max as usize, style);
    end_x.saturating_sub(x)
}

fn ellipsize(s: &str, max: usize, ell: &str) -> String {
    if UnicodeWidthStr::width(s) <= max {
        return s.to_string();
    }
    let mut out = String::new();
    let mut w = 0;
    for g in s.graphemes(true) {
        let gw = UnicodeWidthStr::width(g);
        if w + gw + 1 > max {
            break;
        }
        out.push_str(g);
        w += gw;
    }
    out.push_str(ell);
    out
}

fn draw_sidebar(app: &App, buf: &mut Buffer, wl: &WorkbenchLayout) {
    let (Some(sb), Some(title), Some(list)) = (wl.sidebar, wl.explorer_title, wl.explorer_list) else { return };
    let t = &app.theme;
    let focused = app.focus == Focus::Explorer && app.overlays.is_empty();
    fill(buf, sb, t.sidebar());
    if app.sidebar_view == SidebarView::Search {
        put(buf, title.x, title.y, " SEARCH", title.width, t.title(focused));
        if let Some(b) = wl.sidebar_border {
            fill_with(
                buf,
                b,
                app.glyphs.vbar,
                Style::new().fg(if focused { t.focus_border } else { t.border }).bg(t.bg),
            );
        }
        draw_search_view(app, buf, list, focused);
        return;
    }
    let mut head = String::from(" EXPLORER");
    if !app.workspace.roots().is_empty() {
        head.push_str(": ");
        head.push_str(&app.workspace.name().to_uppercase());
    }
    if !app.explorer.filter.is_empty() {
        head = format!(" FILTER: {}", app.explorer.filter);
    }
    put(
        buf,
        title.x,
        title.y,
        &ellipsize(&head, title.width as usize, app.glyphs.ellipsis),
        title.width,
        t.title(focused),
    );
    if let Some(b) = wl.sidebar_border {
        fill_with(buf, b, app.glyphs.vbar, Style::new().fg(if focused { t.focus_border } else { t.border }).bg(t.bg));
    }
    if app.workspace.roots().is_empty() {
        let lines = [
            "No folder opened.",
            "",
            "Ctrl+K Ctrl+O  Open Folder",
            "Ctrl+R         Open Recent",
            "F1             Command Palette",
            "  → Workspace: Add Folder",
        ];
        for (i, l) in lines.iter().enumerate() {
            put(
                buf,
                list.x + 1,
                list.y + 1 + i as u16,
                l,
                list.width.saturating_sub(1),
                Style::new().fg(t.muted).bg(t.sidebar_bg),
            );
        }
        return;
    }
    let ex = &app.explorer;
    let height = list.height as usize;
    for (row_i, row) in ex.rows.iter().enumerate().skip(ex.scroll).take(height) {
        let y = list.y + (row_i - ex.scroll) as u16;
        let selected = row_i == ex.selected;
        let mut style = t.sidebar();
        if selected {
            style = if focused {
                Style::new().fg(t.list_active_fg).bg(t.list_active_bg)
            } else {
                Style::new().fg(t.sidebar_fg).bg(t.list_inactive_selection_bg)
            };
            fill(buf, Rect { x: list.x, y, width: list.width, height: 1 }, style);
        }
        let indent = (row.depth as u16) * 2;
        let mut x = list.x + 1 + indent.min(list.width.saturating_sub(4));
        let avail = |x: u16| list.right().saturating_sub(x);
        let g = &app.glyphs;
        match &row.kind {
            RowKind::Root { missing, duplicate } => {
                let marker = if row.expanded { g.expanded } else { g.collapsed };
                x += put(buf, x, y, marker, avail(x), style);
                x += put(buf, x, y, " ", avail(x), style);
                let mut s = style.add_modifier(Modifier::BOLD);
                let mut label = row.label.to_uppercase();
                if let Some(m) = missing {
                    s = s.fg(t.missing_root);
                    label = format!("{label} {} ({m})", g.missing);
                } else if *duplicate {
                    s = s.fg(t.muted);
                    label.push_str(" (duplicate)");
                }
                put(buf, x, y, &ellipsize(&label, avail(x) as usize, g.ellipsis), avail(x), s);
            }
            RowKind::Note { error } => {
                let s = style.fg(if *error { t.error } else { t.muted }).add_modifier(Modifier::ITALIC);
                put(buf, x + 2, y, &ellipsize(&row.label, avail(x + 2) as usize, g.ellipsis), avail(x + 2), s);
            }
            kind => {
                let (marker, icon) = match kind {
                    RowKind::Dir | RowKind::SymlinkDir => (
                        if row.expanded { g.expanded } else { g.collapsed },
                        if row.expanded { g.folder_open } else { g.folder },
                    ),
                    _ => (" ", g.file),
                };
                x += put(buf, x, y, marker, avail(x), style);
                x += put(buf, x, y, " ", avail(x), style);
                x += put(buf, x, y, icon, avail(x), style);
                let dirty =
                    app.docs.find_by_path(&row.path).and_then(|d| app.docs.doc(d)).is_some_and(|d| d.is_dirty());
                let mut s = style;
                if matches!(kind, RowKind::Broken) {
                    s = s.fg(t.error).add_modifier(Modifier::CROSSED_OUT);
                }
                let mut label = row.label.clone();
                if matches!(kind, RowKind::SymlinkDir | RowKind::SymlinkFile) {
                    label = format!("{label} {}", g.symlink);
                }
                let room = avail(x).saturating_sub(if dirty { 2 } else { 0 });
                x += put(buf, x, y, &ellipsize(&label, room as usize, g.ellipsis), room, s);
                if dirty {
                    put(buf, x + 1, y, g.dirty, avail(x + 1), style.fg(t.dirty));
                }
            }
        }
    }
    // Scrollbar.
    if ex.rows.len() > height && height > 0 {
        let bar_x = list.right().saturating_sub(1);
        let thumb_h = ((height * height) / ex.rows.len()).max(1);
        let thumb_y = (ex.scroll * height) / ex.rows.len();
        for i in 0..thumb_h.min(height) {
            put(
                buf,
                bar_x,
                list.y + (thumb_y + i).min(height - 1) as u16,
                app.glyphs.scroll_thumb,
                1,
                Style::new().fg(t.scrollbar).bg(t.sidebar_bg),
            );
        }
    }
}

fn draw_group(app: &App, buf: &mut Buffer, gl: &GroupLayout) {
    let t = &app.theme;
    let Some(group) = app.layout.group(gl.id) else { return };
    let group_focused = app.focus == Focus::Editor && app.layout.active == gl.id && app.overlays.is_empty();
    fill(buf, gl.tabs, Style::new().bg(t.tab_bar_bg));
    for cell in tab_cells(app, group, gl.tabs) {
        let v = &group.views[cell.view];
        let active = cell.view == group.active;
        let mut style = if active {
            Style::new().fg(t.tab_active_fg).bg(t.tab_active_bg)
        } else {
            Style::new().fg(t.tab_inactive_fg).bg(t.tab_inactive_bg)
        };
        if v.preview {
            style = style.add_modifier(Modifier::ITALIC);
        }
        fill(buf, Rect { x: cell.x, y: gl.tabs.y, width: cell.width, height: 1 }, style);
        let label = ellipsize(&cell.label, cell.width.saturating_sub(4) as usize, app.glyphs.ellipsis);
        let doc = app.docs.doc(v.doc);
        let label_style = match doc.map(|d| d.disk_state) {
            Some(DiskState::Deleted) => style.add_modifier(Modifier::CROSSED_OUT),
            _ => style,
        };
        put(buf, cell.x + 1, gl.tabs.y, &label, cell.width.saturating_sub(3), label_style);
        let dirty = doc.is_some_and(|d| d.is_dirty());
        let mark = if dirty {
            app.glyphs.dirty
        } else if active {
            app.glyphs.close
        } else {
            " "
        };
        let ms = if dirty { style.fg(t.dirty) } else { style };
        put(buf, cell.close_x, gl.tabs.y, mark, 1, ms);
        if active && group_focused {
            // Focus indicator: underline the active tab.
            for x in cell.x..cell.x + cell.width {
                if let Some(c) = buf.cell_mut((x, gl.tabs.y)) {
                    c.set_style(Style::new().add_modifier(Modifier::UNDERLINED).underline_color(t.focus_border));
                }
            }
        }
    }
    let Some(view) = group.active_view() else {
        draw_empty_group(app, buf, gl.content, gl.id == app.layout.active);
        return;
    };
    if app.docs.is_loading(view.doc) {
        fill(buf, gl.content, Style::new().bg(t.bg));
        let msg = format!("Loading {}…", app.docs.name(view.doc));
        let y = gl.content.y + gl.content.height / 3;
        let x = gl.content.x + gl.content.width.saturating_sub(UnicodeWidthStr::width(msg.as_str()) as u16) / 2;
        put(buf, x, y, &msg, gl.content.width, Style::new().fg(t.muted).bg(t.bg));
        return;
    }
    let Some(doc) = app.docs.doc(view.doc) else { return };
    let rope = doc.rope();
    let settings = app.settings_for(doc.path());
    let tw = settings.editor.tab_size.max(1);
    let height = gl.content.height as usize;
    let primary = view.sel.primary();
    let head = primary.head.min(rope.len_chars());
    let cursor_line = rope.char_to_line(head);
    let bracket = if settings.editor.match_brackets && !doc.large_file { matching_bracket(rope, head) } else { None };
    let ranges: Vec<(usize, usize)> = view.sel.ranges().iter().map(|r| (r.from(), r.to())).collect();
    let heads: Vec<usize> = view.sel.ranges().iter().map(|r| r.head).collect();
    let sel_bg = if group_focused { t.selection_bg } else { t.selection_inactive_bg };
    let len_lines = rope.len_lines();
    // Display rows (folds + word wrap) for the viewport only.
    let map = crate::display::DisplayMap::new(rope, &view.folds, app.wrap_width(view.doc, gl.text_width as usize), tw);
    let rows = map.rows(view.top(), height);
    let scroll_col = if map.wraps() { 0 } else { view.scroll_col };
    let first_line = rows.first().map(|r| r.line).unwrap_or(0);
    let last_line = rows.last().map(|r| r.line).unwrap_or(0);
    // Find matches intersecting the viewport (spec §22.3: visible lines only).
    let find_active = app.find.visible && app.find.doc == Some(view.doc) && gl.id == app.layout.active;
    let vis_start = rope.line_to_char(first_line);
    let vis_end = if last_line + 1 < len_lines { rope.line_to_char(last_line + 1) } else { rope.len_chars() };
    let find_matches: &[(usize, usize)] = if find_active {
        let m = &app.find.matches;
        let a = m.partition_point(|(_, e)| *e <= vis_start);
        let b = m.partition_point(|(st, _)| *st < vis_end);
        &m[a..b.max(a)]
    } else {
        &[]
    };
    let current_match = app.find.current.and_then(|i| app.find.matches.get(i)).copied();
    // Syntax spans for the visible lines only, per contiguous run (folds
    // skip hidden text), cached until the tree changes.
    let mut spans: Vec<yace_syntax::Span> = Vec::new();
    if !doc.large_file {
        let mut runs: Vec<(usize, usize)> = Vec::new();
        for r in &rows {
            match runs.last_mut() {
                Some((_, e)) if r.line <= *e + 1 => *e = r.line,
                _ => runs.push((r.line, r.line)),
            }
        }
        for (a, b) in runs {
            let sb = rope.line_to_byte(a);
            let eb = if b + 1 < len_lines { rope.line_to_byte(b + 1) } else { rope.len_bytes() };
            if let Some(sp) = app.syntax.spans(view.doc, rope, sb..eb) {
                spans.extend_from_slice(&sp);
            }
        }
    }
    let mut si = 0usize;
    // Fold markers: folded headers always; foldable lines per setting.
    let show_foldable = settings.editor.folding
        && match settings.editor.show_folding_controls {
            yace_config::FoldingControls::Always => true,
            yace_config::FoldingControls::Never => false,
            yace_config::FoldingControls::Mouseover => app
                .mouse
                .hover
                .is_some_and(|(x, y)| x >= gl.content.x && x < gl.text_x && gl.content.contains((x, y).into())),
        };
    let marker_x = gl.content.x + gl.gutter.saturating_sub(2);
    for ri in 0..height {
        let y = gl.content.y + ri as u16;
        let Some(&drow) = rows.get(ri) else {
            fill(buf, Rect { x: gl.content.x, y, width: gl.content.width, height: 1 }, Style::new().bg(t.bg));
            continue;
        };
        let line = drow.line;
        let is_cursor_line = line == cursor_line;
        let line_bg = if is_cursor_line && settings.editor.render_line_highlight && !view.sel.has_non_empty() {
            t.cursor_line_bg
        } else {
            t.bg
        };
        fill(buf, Rect { x: gl.content.x, y, width: gl.content.width, height: 1 }, Style::new().bg(t.bg));
        fill(buf, Rect { x: gl.text_x, y, width: gl.text_width, height: 1 }, Style::new().bg(line_bg));
        // Gutter: number on the first row of a line, fold marker after it.
        if drow.sub == 0 {
            if settings.editor.line_numbers != LineNumbers::Off && gl.gutter > 2 {
                let n = match settings.editor.line_numbers {
                    LineNumbers::Relative if !is_cursor_line => line.abs_diff(cursor_line),
                    _ => line + 1,
                };
                let s = format!("{n:>w$} ", w = (gl.gutter - 2) as usize);
                let style = Style::new().fg(if is_cursor_line { t.line_number_active } else { t.line_number }).bg(t.bg);
                put(buf, gl.content.x, y, &s, gl.gutter, style);
            }
            let folded = map.is_folded_header(line);
            if folded || show_foldable && app.fold_range(view.doc, rope, line, tw).is_some() {
                let glyph = if folded { app.glyphs.collapsed } else { app.glyphs.expanded };
                put(buf, marker_x, y, glyph, 1, Style::new().fg(if folded { t.fg } else { t.line_number }).bg(t.bg));
            }
        }
        let line_start = rope.line_to_char(line);
        let content = line_without_ending(rope, line);
        let segs = map.segments(line);
        let seg = segs[drow.sub.min(segs.len() - 1)];
        let last_seg = drow.sub + 1 >= segs.len();
        let limit =
            if map.wraps() { seg.end } else { content.len_chars().min((scroll_col + gl.text_width as usize) * 2 + 16) };
        let text: std::borrow::Cow<str> = content.slice(seg.start..limit.max(seg.start)).into();
        let line_end = line_start + content.len_chars();
        let active_ranges: Vec<(usize, usize)> =
            ranges.iter().copied().filter(|(a, b)| *a <= line_end && *b >= line_start && a != b).collect();
        let line_heads: Vec<usize> = heads.iter().copied().filter(|h| *h >= line_start && *h <= line_end).collect();
        let mut col = seg.indent;
        let mut ci = line_start + seg.start;
        let mut bi = rope.char_to_byte(ci);
        let max_col = scroll_col + gl.text_width as usize;
        for g in text.graphemes(true) {
            if col >= max_col {
                break;
            }
            let w = grapheme_width(g, col, tw);
            let n = g.chars().count();
            while si < spans.len() && spans[si].end <= bi {
                si += 1;
            }
            let scope = spans.get(si).filter(|s| s.start <= bi).map(|s| s.scope);
            bi += g.len();
            if col + w > scroll_col {
                let selected = active_ranges.iter().any(|(a, b)| ci >= *a && ci < *b);
                let mut style = syntax_style(t, scope).bg(line_bg);
                if let Some(m) = find_matches.iter().find(|(a, b)| ci >= *a && ci < *b) {
                    style = style.bg(if Some(*m) == current_match { t.find_current_bg } else { t.find_match_bg });
                }
                if selected {
                    style = style.bg(sel_bg);
                }
                if bracket.is_some_and(|(a, b)| ci == a || ci == b) {
                    style = style.bg(t.bracket_match_bg);
                }
                if line_heads.contains(&ci) && ci != head && group_focused {
                    style = Style::new().fg(t.cursor_fg).bg(t.cursor_bg);
                }
                let (sym, style) = if g == "\t" {
                    (" ".repeat(w), style)
                } else if g.chars().next().is_some_and(|c| c.is_control()) {
                    ("·".to_string(), style.fg(t.whitespace))
                } else {
                    (g.to_string(), style)
                };
                let x0 = gl.text_x as usize + col.saturating_sub(scroll_col);
                let visible_w = (col + w).min(max_col) - col.max(scroll_col);
                if col < scroll_col || visible_w < w {
                    // Partially visible wide char: blank it.
                    fill(buf, Rect { x: x0 as u16, y, width: visible_w as u16, height: 1 }, style);
                } else {
                    put(buf, x0 as u16, y, &sym, w as u16, style);
                }
            }
            col += w;
            ci += n;
        }
        if !last_seg {
            continue;
        }
        // Selected line break / secondary cursor at end of line.
        let end_x = gl.text_x as usize + col.saturating_sub(scroll_col);
        if col >= scroll_col && col < max_col {
            let nl_selected =
                active_ranges.iter().any(|(a, b)| line_end >= *a && line_end < *b && line + 1 < len_lines);
            if line_heads.contains(&line_end) && line_end != head && group_focused {
                put(buf, end_x as u16, y, " ", 1, Style::new().bg(t.cursor_bg));
            } else if nl_selected {
                put(buf, end_x as u16, y, " ", 1, Style::new().bg(sel_bg));
            }
        }
        // Folded region placeholder.
        if map.is_folded_header(line) && col + 1 >= scroll_col && col + 4 <= max_col {
            let x = gl.text_x as usize + (col + 1).saturating_sub(scroll_col);
            let text = format!(" {} ", app.glyphs.ellipsis);
            put(buf, x as u16, y, &text, 3, Style::new().fg(t.muted).bg(t.selection_inactive_bg));
        }
    }
    // Vertical scrollbar (visible lines, so folds shrink the document).
    let vis_lines = map.visible_lines();
    if vis_lines > height && height > 0 {
        let x = gl.content.right().saturating_sub(1);
        let thumb_h = ((height * height) / vis_lines).max(1);
        let top_vis = view.scroll_line.saturating_sub(map.hidden_before(view.scroll_line));
        let thumb_y = (top_vis * height) / vis_lines.max(1);
        for i in 0..thumb_h.min(height) {
            let yy = gl.content.y + (thumb_y + i).min(height - 1) as u16;
            put(buf, x, yy, app.glyphs.scroll_thumb, 1, Style::new().fg(t.scrollbar).bg(t.bg));
        }
    }
    if find_active {
        draw_find_bar(app, buf, gl);
    }
}

fn syntax_style(t: &crate::theme::Theme, scope: Option<yace_syntax::Scope>) -> Style {
    use yace_syntax::Scope;
    let Some(s) = scope else { return Style::new().fg(t.fg) };
    let style = Style::new().fg(t.syntax[s as usize]);
    match s {
        Scope::Heading | Scope::Bold => style.add_modifier(Modifier::BOLD),
        Scope::Italic => style.add_modifier(Modifier::ITALIC),
        Scope::Link => style.add_modifier(Modifier::UNDERLINED),
        _ => style,
    }
}

fn draw_empty_group(app: &App, buf: &mut Buffer, r: Rect, active: bool) {
    let t = &app.theme;
    if r.height < 8 || r.width < 30 {
        return;
    }
    let lines: &[(&str, &str)] = &[
        ("YACE", ""),
        ("", ""),
        ("Show All Commands", "Ctrl+Shift+P / F1"),
        ("Open Folder", "Ctrl+K Ctrl+O"),
        ("Open Recent", "Ctrl+R"),
        ("Focus Explorer", "Ctrl+Shift+E"),
        ("New File", "Ctrl+N"),
    ];
    let y0 = r.y + r.height / 3;
    for (i, (a, b)) in lines.iter().enumerate() {
        let y = y0 + i as u16;
        if y >= r.bottom() {
            break;
        }
        let style = if i == 0 {
            Style::new().fg(if active { t.focus_border } else { t.muted }).add_modifier(Modifier::BOLD)
        } else {
            Style::new().fg(t.muted)
        };
        let text = if b.is_empty() { a.to_string() } else { format!("{a:<20}{b}") };
        let w = UnicodeWidthStr::width(text.as_str()) as u16;
        let x = r.x + r.width.saturating_sub(w) / 2;
        put(buf, x, y, &text, r.width, style.bg(t.bg));
    }
}

fn editor_cursor(app: &App, wl: &WorkbenchLayout) -> Option<(u16, u16)> {
    let gl = wl.groups.iter().find(|g| g.id == app.layout.active)?;
    let view = app.layout.active_group().active_view()?;
    let doc = app.docs.doc(view.doc)?;
    let rope = doc.rope();
    let tw = app.settings_for(doc.path()).editor.tab_size.max(1);
    let map = crate::display::DisplayMap::new(rope, &view.folds, app.wrap_width(view.doc, gl.text_width as usize), tw);
    let (row, col) = map.row_of(view.sel.primary().head);
    let height = gl.content.height as usize;
    let dy = map.distance(map.clamp(view.top()), row, height.saturating_sub(1))?;
    let scroll_col = if map.wraps() { 0 } else { view.scroll_col };
    if col < scroll_col || col >= scroll_col + gl.text_width as usize {
        return None;
    }
    Some((gl.text_x + (col - scroll_col) as u16, gl.content.y + dy as u16))
}

fn draw_status(app: &App, buf: &mut Buffer, r: Rect) {
    let t = &app.theme;
    let bg = if app.workspace.roots().is_empty() { t.status_no_folder_bg } else { t.status_bg };
    let base = Style::new().fg(t.status_fg).bg(bg);
    fill(buf, r, base);
    let mut left;
    let mut right: Vec<String> = Vec::new();
    let active = app.active();
    if let Some((_, v, d)) = active
        && let Some(doc) = app.docs.doc(d)
    {
        let view = &app.layout.active_group().views[v];
        if let Some(p) = doc.path() {
            match app.workspace.relative(p) {
                Some((root, rel)) => left = format!(" {} › {}", root.display_name(), rel.display()),
                None => left = format!(" {}", p.display()),
            }
        } else {
            left = format!(" {}", app.docs.name(d));
        }
        match doc.disk_state {
            DiskState::Deleted => left.push_str("  (deleted on disk)"),
            DiskState::Conflicted => left.push_str("  (changed on disk)"),
            DiskState::InSync => {}
        }
        if doc.large_file {
            left.push_str("  [large file]");
        }
        let rope = doc.rope();
        let head = view.sel.primary().head.min(rope.len_chars());
        let line = rope.char_to_line(head);
        let ls = rope.line_to_char(line);
        let col = rope.slice(ls..head).to_string().graphemes(true).count() + 1;
        right.push(format!("Ln {}, Col {}", line + 1, col));
        if view.sel.is_multi() {
            right.push(format!("{} selections", view.sel.len()));
        } else if view.sel.has_non_empty() {
            right.push(format!("({} selected)", view.sel.primary().len()));
        }
        let s = app.settings_for(doc.path());
        right.push(if s.editor.insert_spaces {
            format!("Spaces: {}", s.editor.tab_size)
        } else {
            format!("Tab Size: {}", s.editor.tab_size)
        });
        right.push(if doc.has_bom { "UTF-8 with BOM".into() } else { "UTF-8".into() });
        right.push(doc.line_ending.label().into());
        right.push(app.syntax.language(view.doc).name.into());
    } else if !app.workspace.roots().is_empty() {
        left = format!(" {}", app.workspace.name());
    } else {
        left = " No folder opened".into();
    }
    // Chord / notification override the left side.
    let mut left_style = base;
    if app.resolver.is_pending() {
        left = format!(" ({}) was pressed. Waiting for second key of chord…", app.resolver.pending());
    } else if let Some(n) = app.notifications.last() {
        left = format!(" {}", n.text);
        left_style = match n.level {
            Severity::Error => Style::new().fg(t.status_fg).bg(t.error),
            Severity::Warning => Style::new().fg(t.bg).bg(t.warning),
            Severity::Info => base,
        };
    }
    let right_text = format!("{}  ", right.join("   "));
    let rw = UnicodeWidthStr::width(right_text.as_str()) as u16;
    let lw_max = r.width.saturating_sub(rw + 1);
    let shown_right = if lw_max < 10 { 0 } else { rw };
    let left_max = r.width.saturating_sub(shown_right);
    let left = ellipsize(&left, left_max as usize, app.glyphs.ellipsis);
    fill(buf, Rect { width: (UnicodeWidthStr::width(left.as_str()) as u16 + 1).min(left_max), ..r }, left_style);
    put(buf, r.x, r.y, &left, left_max, left_style);
    if shown_right > 0 {
        put(buf, r.right().saturating_sub(rw), r.y, &right_text, rw, base);
    }
}

/// Geometry of the top overlay, shared with mouse handling.
#[derive(Debug, Clone, Default)]
pub struct OverlayGeom {
    pub outer: Rect,
    pub input: Option<Rect>,
    pub list: Option<Rect>,
    pub buttons: Vec<(Rect, usize)>,
}

pub fn overlay_geometry(area: Rect, o: &Overlay) -> OverlayGeom {
    let w = area.width.saturating_sub(4).clamp(10, 80);
    let x = area.x + (area.width.saturating_sub(w)) / 2;
    match o {
        Overlay::Picker(p) => {
            let rows = p.filtered.len().clamp(1, 14) as u16;
            let h = (rows + 3).min(area.height.saturating_sub(2).max(3));
            let outer = Rect { x, y: area.y + 1.min(area.height), width: w, height: h };
            let input = Rect { x: outer.x + 1, y: outer.y + 1, width: outer.width.saturating_sub(2), height: 1 };
            let list = Rect {
                x: outer.x + 1,
                y: outer.y + 2,
                width: outer.width.saturating_sub(2),
                height: h.saturating_sub(3),
            };
            OverlayGeom { outer, input: Some(input), list: Some(list), buttons: vec![] }
        }
        Overlay::Prompt(_) => {
            let outer = Rect { x, y: area.y + 1.min(area.height), width: w, height: 5.min(area.height) };
            let input = Rect { x: outer.x + 1, y: outer.y + 2, width: outer.width.saturating_sub(2), height: 1 };
            OverlayGeom { outer, input: Some(input), list: None, buttons: vec![] }
        }
        Overlay::Confirm(c) => {
            let h = (c.message.len() as u16 + 5).min(area.height);
            let y = area.y + area.height.saturating_sub(h) / 3;
            let outer = Rect { x, y, width: w, height: h };
            let mut bx = outer.x + 2;
            let by = outer.bottom().saturating_sub(2);
            let mut buttons = Vec::new();
            for (i, (label, _)) in c.buttons.iter().enumerate() {
                let bw = UnicodeWidthStr::width(label.as_str()) as u16 + 4;
                buttons.push((Rect { x: bx, y: by, width: bw.min(outer.right().saturating_sub(bx)), height: 1 }, i));
                bx += bw + 1;
            }
            OverlayGeom { outer, input: None, list: None, buttons }
        }
        Overlay::Info { lines, .. } => {
            let h = (lines.len() as u16 + 4).min(area.height);
            let y = area.y + area.height.saturating_sub(h) / 3;
            OverlayGeom { outer: Rect { x, y, width: w, height: h }, ..Default::default() }
        }
    }
}

fn draw_box(app: &App, buf: &mut Buffer, r: Rect, title: &str) {
    let t = &app.theme;
    let style = Style::new().fg(t.overlay_fg).bg(t.overlay_bg);
    fill(buf, r, style);
    let b = Style::new().fg(t.overlay_border).bg(t.overlay_bg);
    let ascii = app.glyphs.vbar == "|";
    let (h, v, tl, tr, bl, br) =
        if ascii { ("-", "|", "+", "+", "+", "+") } else { ("─", "│", "╭", "╮", "╰", "╯") };
    if r.width < 2 || r.height < 2 {
        return;
    }
    for x in r.x + 1..r.right() - 1 {
        put(buf, x, r.y, h, 1, b);
        put(buf, x, r.bottom() - 1, h, 1, b);
    }
    for y in r.y + 1..r.bottom() - 1 {
        put(buf, r.x, y, v, 1, b);
        put(buf, r.right() - 1, y, v, 1, b);
    }
    put(buf, r.x, r.y, tl, 1, b);
    put(buf, r.right() - 1, r.y, tr, 1, b);
    put(buf, r.x, r.bottom() - 1, bl, 1, b);
    put(buf, r.right() - 1, r.bottom() - 1, br, 1, b);
    if !title.is_empty() {
        let tt = format!(" {title} ");
        put(buf, r.x + 2, r.y, &tt, r.width.saturating_sub(4), b.add_modifier(Modifier::BOLD));
    }
}

fn draw_input(
    app: &App,
    buf: &mut Buffer,
    r: Rect,
    input: &crate::overlay::LineInput,
    placeholder: &str,
) -> (u16, u16) {
    let t = &app.theme;
    let style = Style::new().fg(t.overlay_fg).bg(t.input_bg);
    fill(buf, r, style);
    let cursor_cells = input.cursor_cells() as u16;
    let scroll = cursor_cells.saturating_sub(r.width.saturating_sub(2));
    if input.text.is_empty() {
        put(buf, r.x + 1, r.y, placeholder, r.width.saturating_sub(1), style.fg(t.muted));
    } else {
        let visible: String = {
            let mut w = 0u16;
            input
                .text
                .graphemes(true)
                .skip_while(|g| {
                    let gw = UnicodeWidthStr::width(*g) as u16;
                    let skip = w + gw <= scroll;
                    if skip {
                        w += gw;
                    }
                    skip
                })
                .collect()
        };
        let s = if input.all_selected { style.bg(t.selection_bg) } else { style };
        put(buf, r.x + 1, r.y, &visible, r.width.saturating_sub(1), s);
    }
    (r.x + 1 + cursor_cells - scroll, r.y)
}

fn draw_overlay(app: &App, buf: &mut Buffer, area: Rect, o: &Overlay) -> Option<(u16, u16)> {
    let t = &app.theme;
    let geom = overlay_geometry(area, o);
    match o {
        Overlay::Picker(p) => {
            draw_box(app, buf, geom.outer, &p.title);
            let cursor = draw_input(app, buf, geom.input?, &p.input, "Type to filter");
            let list = geom.list?;
            if p.filtered.is_empty() {
                put(buf, list.x + 1, list.y, &p.empty_text, list.width, Style::new().fg(t.muted).bg(t.overlay_bg));
            }
            let height = list.height as usize;
            let scroll = if p.selected >= height { p.selected + 1 - height } else { 0 };
            for (row, idx) in p.filtered.iter().enumerate().skip(scroll).take(height) {
                let it = &p.items[*idx];
                let y = list.y + (row - scroll) as u16;
                let selected = row == p.selected;
                let style = if selected {
                    Style::new().fg(t.list_active_fg).bg(t.overlay_selected_bg)
                } else {
                    Style::new().fg(t.overlay_fg).bg(t.overlay_bg)
                };
                fill(buf, Rect { x: list.x, y, width: list.width, height: 1 }, style);
                let hint_w = UnicodeWidthStr::width(it.hint.as_str()) as u16;
                let room = list.width.saturating_sub(hint_w + 3);
                let mut x = list.x + 1;
                x += put_highlighted(
                    buf,
                    x,
                    y,
                    &ellipsize(&it.label, room as usize, app.glyphs.ellipsis),
                    room,
                    style,
                    style.fg(t.match_fg).add_modifier(Modifier::BOLD),
                    p.highlight(*idx),
                );
                if !it.detail.is_empty() {
                    let left = room.saturating_sub(x - list.x);
                    if left > 4 {
                        put(
                            buf,
                            x + 2,
                            y,
                            &ellipsize(&it.detail, (left - 2) as usize, app.glyphs.ellipsis),
                            left - 2,
                            style.fg(t.muted),
                        );
                    }
                }
                if hint_w > 0 {
                    put(buf, list.right().saturating_sub(hint_w + 1), y, &it.hint, hint_w, style.fg(t.muted));
                }
            }
            Some(cursor)
        }
        Overlay::Prompt(p) => {
            draw_box(app, buf, geom.outer, &p.title);
            let hint = p.error.clone().or_else(|| p.hint.clone()).unwrap_or_default();
            let hs = if p.error.is_some() {
                Style::new().fg(t.error).bg(t.overlay_bg)
            } else {
                Style::new().fg(t.muted).bg(t.overlay_bg)
            };
            put(
                buf,
                geom.outer.x + 2,
                geom.outer.y + 1,
                &ellipsize(&hint, geom.outer.width.saturating_sub(4) as usize, app.glyphs.ellipsis),
                geom.outer.width.saturating_sub(4),
                hs,
            );
            let cursor = draw_input(app, buf, geom.input?, &p.input, "");
            put(
                buf,
                geom.outer.x + 2,
                geom.outer.y + 3,
                "Enter to confirm · Escape to cancel",
                geom.outer.width.saturating_sub(4),
                Style::new().fg(t.muted).bg(t.overlay_bg),
            );
            Some(cursor)
        }
        Overlay::Confirm(c) => {
            draw_box(app, buf, geom.outer, &c.title);
            for (i, line) in c.message.iter().enumerate() {
                put(
                    buf,
                    geom.outer.x + 2,
                    geom.outer.y + 1 + i as u16,
                    &ellipsize(line, geom.outer.width.saturating_sub(4) as usize, app.glyphs.ellipsis),
                    geom.outer.width.saturating_sub(4),
                    Style::new().fg(t.overlay_fg).bg(t.overlay_bg),
                );
            }
            for (r, i) in &geom.buttons {
                let selected = *i == c.selected;
                let style = if selected {
                    Style::new().fg(t.list_active_fg).bg(t.focus_border).add_modifier(Modifier::BOLD)
                } else {
                    Style::new().fg(t.overlay_fg).bg(t.input_bg)
                };
                fill(buf, *r, style);
                put(buf, r.x + 2, r.y, &c.buttons[*i].0, r.width.saturating_sub(2), style);
            }
            None
        }
        Overlay::Info { title, lines } => {
            draw_box(app, buf, geom.outer, title);
            for (i, line) in lines.iter().enumerate() {
                put(
                    buf,
                    geom.outer.x + 2,
                    geom.outer.y + 1 + i as u16,
                    line,
                    geom.outer.width.saturating_sub(4),
                    Style::new().fg(t.overlay_fg).bg(t.overlay_bg),
                );
            }
            let foot = "Escape to close";
            put(
                buf,
                geom.outer.x + 2,
                geom.outer.bottom().saturating_sub(2),
                foot,
                geom.outer.width.saturating_sub(4),
                Style::new().fg(t.muted).bg(t.overlay_bg),
            );
            None
        }
    }
}

/// Like `put`, but chars at `indices` (char positions in `text`) use `hl`.
#[allow(clippy::too_many_arguments)]
fn put_highlighted(
    buf: &mut Buffer,
    x: u16,
    y: u16,
    text: &str,
    max: u16,
    style: Style,
    hl: Style,
    indices: &[usize],
) -> u16 {
    if indices.is_empty() {
        return put(buf, x, y, text, max, style);
    }
    let mut used = 0u16;
    for (ci, c) in text.chars().enumerate() {
        let w = unicode_width::UnicodeWidthChar::width(c).unwrap_or(0) as u16;
        if used + w > max {
            break;
        }
        let st = if indices.binary_search(&ci).is_ok() { hl } else { style };
        used += put(buf, x + used, y, c.encode_utf8(&mut [0; 4]), w.max(1), st);
    }
    used
}

// ---------------------------------------------------------------------------
// Find bar

/// Clickable parts of the find bar (shared by drawing and mouse handling).
#[derive(Debug, Clone)]
pub struct FindBarLayout {
    pub row: Rect,
    pub replace_row: Option<Rect>,
    pub query: Rect,
    pub replace: Option<Rect>,
    pub status: Rect,
    /// ('c' | 'w' | 'r' | 'l', rect)
    pub toggles: Vec<(char, Rect)>,
    pub close: Rect,
}

pub fn find_bar_layout(app: &App, gl: &GroupLayout) -> Option<FindBarLayout> {
    if gl.content.width < 24 || gl.content.height < 2 {
        return None;
    }
    let w = gl.content.width.saturating_sub(3).min(50);
    let x = gl.content.right().saturating_sub(w + 1);
    let row = Rect { x, y: gl.content.y, width: w, height: 1 };
    let replace_row = app.find.replace_visible.then_some(Rect { y: gl.content.y + 1, ..row });
    // [▌][input][status 9][Aa ab .* ≡][ ×]
    let right_part = 9 + 12 + 2;
    let input_w = w.saturating_sub(right_part + 1).max(6);
    let query = Rect { x: row.x + 1, width: input_w, ..row };
    let status = Rect { x: query.right(), width: 9, ..row };
    let mut tx = status.right();
    let mut toggles = Vec::new();
    for (c, label_w) in [('c', 2u16), ('w', 2), ('r', 2), ('l', 1)] {
        toggles.push((c, Rect { x: tx, width: label_w, ..row }));
        tx += label_w + 1;
    }
    let close = Rect { x: row.right().saturating_sub(2), width: 2, ..row };
    let replace = replace_row.map(|r| Rect { x: r.x + 1, width: input_w, ..r });
    Some(FindBarLayout { row, replace_row, query, replace, status, toggles, close })
}

fn draw_find_bar(app: &App, buf: &mut Buffer, gl: &GroupLayout) {
    let Some(lay) = find_bar_layout(app, gl) else { return };
    let t = &app.theme;
    let bar = Style::new().fg(t.overlay_fg).bg(t.overlay_bg);
    let focused = app.focus == Focus::Find && app.overlays.is_empty();
    for r in std::iter::once(lay.row).chain(lay.replace_row) {
        fill(buf, r, bar);
        put(buf, r.x, r.y, if focused { "▌" } else { " " }, 1, Style::new().fg(t.focus_border).bg(t.overlay_bg));
    }
    let f = &app.find;
    draw_input(app, buf, lay.query, &f.query, "Find");
    let status = if f.query.text.is_empty() { "Esc closes".to_string() } else { f.status() };
    let st_style = if f.error.is_some() || status == "No results" { bar.fg(t.error) } else { bar.fg(t.muted) };
    put(buf, lay.status.x + 1, lay.status.y, &ellipsize(&status, 8, app.glyphs.ellipsis), 8, st_style);
    let on = |c: char| match c {
        'c' => f.opts.case_sensitive,
        'w' => f.opts.whole_word,
        'r' => f.opts.regex,
        _ => f.within.is_some(),
    };
    for (c, r) in &lay.toggles {
        let label = match c {
            'c' => "Aa",
            'w' => "ab",
            'r' => ".*",
            _ => "≡",
        };
        let s = if on(*c) { Style::new().fg(t.list_active_fg).bg(t.focus_border) } else { bar.fg(t.muted) };
        put(buf, r.x, r.y, label, r.width, s);
    }
    put(buf, lay.close.x, lay.close.y, &format!(" {}", app.glyphs.close), 2, bar);
    if let (Some(rr), Some(ri)) = (lay.replace_row, lay.replace) {
        draw_input(app, buf, ri, &f.replace, "Replace");
        let hint = "⇧^1 one · ^⌥⏎ all";
        put(buf, ri.right() + 1, rr.y, hint, rr.right().saturating_sub(ri.right() + 1), bar.fg(t.muted));
    }
}

fn find_bar_cursor(app: &App, gl: &GroupLayout) -> Option<(u16, u16)> {
    let lay = find_bar_layout(app, gl)?;
    let (ir, input) = match app.find.field {
        crate::app::FindField::Query => (lay.query, &app.find.query),
        crate::app::FindField::Replace => (lay.replace?, &app.find.replace),
    };
    let cells = input.cursor_cells() as u16;
    let scroll = cells.saturating_sub(ir.width.saturating_sub(2));
    Some((ir.x + 1 + cells - scroll, ir.y))
}

// ---------------------------------------------------------------------------
// Search view

#[derive(Debug, Clone)]
pub struct SearchLayout {
    pub fields: Vec<(crate::app::SearchField, Rect)>,
    pub toggles: Rect,
    pub status: Rect,
    pub results: Rect,
}

pub fn search_layout(app: &App, list: Rect) -> SearchLayout {
    use crate::app::SearchField;
    let mut y = list.y;
    let mut fields = Vec::new();
    let input_w = list.width.saturating_sub(12).max(4);
    let toggles = Rect { x: list.x + 1 + input_w + 1, y, width: list.width.saturating_sub(input_w + 2), height: 1 };
    for f in app.search.fields() {
        if f == SearchField::Results {
            continue;
        }
        let w = if f == SearchField::Query { input_w } else { list.width.saturating_sub(2) };
        fields.push((f, Rect { x: list.x + 1, y, width: w, height: 1 }));
        y += 1;
    }
    let status = Rect { x: list.x + 1, y, width: list.width.saturating_sub(2), height: 1 };
    let results = Rect { x: list.x, y: y + 1, width: list.width, height: list.bottom().saturating_sub(y + 1) };
    SearchLayout { fields, toggles, status, results }
}

/// First visible results row for the current selection.
pub fn search_scroll(app: &App, height: usize) -> usize {
    let sel = app.search.selected;
    if height == 0 || sel < height { 0 } else { sel + 1 - height }
}

fn draw_search_view(app: &App, buf: &mut Buffer, list: Rect, focused: bool) {
    use crate::app::{SearchField, SearchRow};
    let t = &app.theme;
    let s = &app.search;
    let lay = search_layout(app, list);
    for (f, r) in &lay.fields {
        let (input, ph) = match f {
            SearchField::Query => (&s.query, "Search"),
            SearchField::Replace => (&s.replace, "Replace"),
            SearchField::Include => (&s.include, "files to include (e.g. *.ts, ./Root Name/src)"),
            SearchField::Exclude => (&s.exclude, "files to exclude"),
            SearchField::Results => continue,
        };
        draw_input(app, buf, *r, input, ph);
        if focused && s.field == *f {
            put(buf, r.x.saturating_sub(1), r.y, "▌", 1, Style::new().fg(t.focus_border).bg(t.sidebar_bg));
        }
    }
    let mut x = lay.toggles.x;
    for (label, on) in [("Aa", s.opts.case_sensitive), ("ab", s.opts.whole_word), (".*", s.opts.regex)] {
        let st = if on {
            Style::new().fg(t.list_active_fg).bg(t.focus_border)
        } else {
            Style::new().fg(t.muted).bg(t.sidebar_bg)
        };
        x += put(buf, x, lay.toggles.y, label, lay.toggles.right().saturating_sub(x), st);
        x += put(buf, x, lay.toggles.y, " ", lay.toggles.right().saturating_sub(x), t.sidebar());
    }
    let status = app.search_status();
    let st = if s.error.is_some() {
        Style::new().fg(t.error).bg(t.sidebar_bg)
    } else {
        Style::new().fg(t.muted).bg(t.sidebar_bg)
    };
    put(
        buf,
        lay.status.x,
        lay.status.y,
        &ellipsize(&status, lay.status.width as usize, app.glyphs.ellipsis),
        lay.status.width,
        st,
    );

    let rows = s.rows();
    let height = lay.results.height as usize;
    let scroll = search_scroll(app, height);
    let results_focused = focused && s.field == SearchField::Results;
    let g = &app.glyphs;
    for (i, row) in rows.iter().enumerate().skip(scroll).take(height) {
        let y = lay.results.y + (i - scroll) as u16;
        let selected = i == s.selected && s.field == SearchField::Results;
        let base = if selected && results_focused {
            Style::new().fg(t.list_active_fg).bg(t.list_active_bg)
        } else if selected {
            Style::new().fg(t.sidebar_fg).bg(t.list_inactive_selection_bg)
        } else {
            t.sidebar()
        };
        let line = Rect { x: lay.results.x, y, width: lay.results.width, height: 1 };
        fill(buf, line, base);
        let right = line.right();
        match *row {
            SearchRow::Root(r) => {
                let name = app.search_root_name(r).to_uppercase();
                put(
                    buf,
                    line.x + 1,
                    y,
                    &ellipsize(&name, (line.width - 1) as usize, g.ellipsis),
                    line.width - 1,
                    base.add_modifier(Modifier::BOLD),
                );
            }
            SearchRow::File(fi) => {
                let f = &s.files[fi];
                let (dir, name) = match f.rel.rfind('/') {
                    Some(p) => (&f.rel[..p], &f.rel[p + 1..]),
                    None => ("", f.rel.as_str()),
                };
                let mut x = line.x + 2;
                x += put(buf, x, y, if f.expanded { g.expanded } else { g.collapsed }, right.saturating_sub(x), base);
                x += put(buf, x, y, " ", right.saturating_sub(x), base);
                let mut ns = base;
                if f.file_excluded {
                    ns = ns.add_modifier(Modifier::CROSSED_OUT).fg(t.muted);
                }
                x += put(buf, x, y, name, right.saturating_sub(x), ns);
                let count = f.matches.iter().map(|m| m.ranges.len().max(1)).sum::<usize>();
                let tail = format!(" {dir}  ({count})");
                put(
                    buf,
                    x,
                    y,
                    &ellipsize(&tail, right.saturating_sub(x) as usize, g.ellipsis),
                    right.saturating_sub(x),
                    base.fg(t.muted),
                );
            }
            SearchRow::Match(fi, mi) => {
                let f = &s.files[fi];
                let m = &f.matches[mi];
                let excluded = f.file_excluded || f.excluded.contains(&mi);
                let num = format!("{:>4} ", m.line + 1);
                let mut x = line.x + 4;
                x += put(buf, x, y, &num, right.saturating_sub(x), base.fg(t.line_number));
                // Trim leading whitespace, keeping match columns aligned.
                let lead = m.text.chars().take_while(|c| c.is_whitespace()).count();
                let shown: String = m.text.chars().skip(lead).collect();
                let hl: Vec<usize> = m
                    .ranges
                    .iter()
                    .flat_map(|(a, b)| (a.saturating_sub(lead)..b.saturating_sub(lead)).collect::<Vec<_>>())
                    .collect();
                let mut ts = base;
                let mut hs = base.bg(t.find_match_bg);
                if excluded {
                    ts = ts.add_modifier(Modifier::CROSSED_OUT).fg(t.muted);
                    hs = ts;
                }
                put_highlighted(buf, x, y, &shown, right.saturating_sub(x), ts, hs, &hl);
            }
        }
    }
}

fn search_view_cursor(app: &App, list: Rect) -> Option<(u16, u16)> {
    let lay = search_layout(app, list);
    let (_, r) = lay.fields.iter().find(|(f, _)| *f == app.search.field)?;
    let input = match app.search.field {
        crate::app::SearchField::Query => &app.search.query,
        crate::app::SearchField::Replace => &app.search.replace,
        crate::app::SearchField::Include => &app.search.include,
        crate::app::SearchField::Exclude => &app.search.exclude,
        crate::app::SearchField::Results => return None,
    };
    let cells = input.cursor_cells() as u16;
    let scroll = cells.saturating_sub(r.width.saturating_sub(2));
    Some((r.x + 1 + cells - scroll, r.y))
}

// ---------------------------------------------------------------------------
// Bottom panel (integrated terminal)

/// One terminal tab in the strip: (tab index, x, width, close x).
pub type PanelTabCell = (usize, u16, u16, u16);

/// Tab-strip geometry plus the "+" button.
pub fn panel_tab_cells(app: &App, strip: Rect) -> (Vec<PanelTabCell>, Option<Rect>) {
    let mut x = strip.x + 10; // " TERMINAL "
    let mut out = Vec::new();
    for i in 0..app.panel.tabs.len() {
        let label = app.terminal_label(i);
        let w = (UnicodeWidthStr::width(label.as_str()) as u16 + 4).min(40);
        if x + w > strip.right().saturating_sub(3) {
            break;
        }
        out.push((i, x, w, x + w - 2));
        x += w;
    }
    let plus = (x + 3 <= strip.right()).then_some(Rect { x: x + 1, y: strip.y, width: 2, height: 1 });
    (out, plus)
}

fn term_color(app: &App, c: yace_terminal::TermColor, fg: bool) -> ratatui::style::Color {
    use ratatui::style::Color;
    use yace_terminal::TermColor;
    let t = &app.theme;
    match c {
        TermColor::Default => {
            if fg {
                t.terminal_fg
            } else {
                t.terminal_bg
            }
        }
        TermColor::Named(i) => t.terminal_ansi[usize::from(i).min(15)],
        TermColor::Indexed(i) => match t.depth {
            crate::theme::Depth::TrueColor | crate::theme::Depth::Ansi256 => Color::Indexed(i),
            _ => t.terminal_ansi[usize::from(i % 16)],
        },
        TermColor::Rgb(r, g, b) => crate::theme::downgrade(Color::Rgb(r, g, b), t.depth),
    }
}

fn draw_panel(app: &App, buf: &mut Buffer, wl: &WorkbenchLayout) {
    let (Some(border), Some(strip), Some(content)) = (wl.panel_border, wl.panel_tabs, wl.panel_content) else { return };
    let t = &app.theme;
    let focused = app.focus == Focus::Terminal && app.overlays.is_empty();
    fill_with(buf, border, app.glyphs.hbar, Style::new().fg(if focused { t.focus_border } else { t.border }).bg(t.bg));
    fill(buf, strip, Style::new().fg(t.sidebar_fg).bg(t.tab_bar_bg));
    put(buf, strip.x, strip.y, " TERMINAL ", 10, t.title(focused).bg(t.tab_bar_bg));
    let (cells, plus) = panel_tab_cells(app, strip);
    for (i, x, w, close_x) in cells {
        let active = i == app.panel.active;
        let style = if active {
            Style::new().fg(t.tab_active_fg).bg(t.terminal_bg)
        } else {
            Style::new().fg(t.tab_inactive_fg).bg(t.tab_inactive_bg)
        };
        let r = Rect { x, y: strip.y, width: w, height: 1 };
        fill(buf, r, style);
        let label = app.terminal_label(i);
        put(
            buf,
            x + 1,
            strip.y,
            &ellipsize(&label, w.saturating_sub(4) as usize, app.glyphs.ellipsis),
            w.saturating_sub(3),
            style,
        );
        put(buf, close_x, strip.y, app.glyphs.close, 1, style);
    }
    if let Some(p) = plus {
        put(buf, p.x, p.y, "+", 1, Style::new().fg(t.sidebar_fg).bg(t.tab_bar_bg).add_modifier(Modifier::BOLD));
    }
    fill(buf, content, Style::new().fg(t.terminal_fg).bg(t.terminal_bg));
    let Some(tab) = app.panel.active_tab() else {
        put(
            buf,
            content.x + 2,
            content.y,
            "No terminals. Press Ctrl+` or Ctrl+J to open one.",
            content.width,
            Style::new().fg(t.muted).bg(t.terminal_bg),
        );
        return;
    };
    if tab.not_started && tab.session.is_none() {
        let msg = format!("Restored terminal — press Enter to start a new shell in {}", tab.cwd.display());
        put(
            buf,
            content.x + 2,
            content.y,
            &ellipsize(&msg, content.width as usize, app.glyphs.ellipsis),
            content.width.saturating_sub(2),
            Style::new().fg(t.muted).bg(t.terminal_bg),
        );
        return;
    }
    let Some(screen) = crate::app::terminal_snapshot(tab) else { return };
    let rows = screen.rows.min(content.height as usize);
    let cols = screen.cols.min(content.width as usize);
    for r in 0..rows {
        let y = content.y + r as u16;
        for c in 0..cols {
            let cell = screen.cell(r, c);
            if cell.width == 0 {
                continue;
            }
            let st = cell.style;
            let (mut fg, mut bg) = (term_color(app, cell.fg, true), term_color(app, cell.bg, false));
            if st.inverse {
                std::mem::swap(&mut fg, &mut bg);
            }
            if st.hidden {
                fg = bg;
            }
            if st.selected {
                bg = t.selection_bg;
            }
            let mut style = Style::new().fg(fg).bg(bg);
            if st.bold {
                style = style.add_modifier(Modifier::BOLD);
            }
            if st.dim {
                style = style.add_modifier(Modifier::DIM);
            }
            if st.italic {
                style = style.add_modifier(Modifier::ITALIC);
            }
            if st.underline {
                style = style.add_modifier(Modifier::UNDERLINED);
            }
            if st.strike {
                style = style.add_modifier(Modifier::CROSSED_OUT);
            }
            let x = content.x + c as u16;
            let mut sym = String::new();
            sym.push(if cell.ch == '\0' { ' ' } else { cell.ch });
            if let Some(e) = &cell.extra {
                sym.push_str(e);
            }
            let w = u16::from(cell.width).min(content.right() - x);
            put(buf, x, y, &sym, w.max(1), style);
        }
    }
    if let Some(code) = tab.exited {
        let msg = match code {
            Some(c) => format!(" Process exited with code {c}. Press Enter to restart, or kill the terminal. "),
            None => " Process ended. Press Enter to restart, or kill the terminal. ".to_string(),
        };
        let y = content.bottom().saturating_sub(1);
        put(
            buf,
            content.x,
            y,
            &ellipsize(&msg, content.width as usize, app.glyphs.ellipsis),
            content.width,
            Style::new().fg(t.bg).bg(t.warning),
        );
    }
    if screen.display_offset > 0 {
        let msg = format!(" ↑ {} lines ", screen.display_offset);
        let w = UnicodeWidthStr::width(msg.as_str()) as u16;
        put(buf, content.right().saturating_sub(w), content.y, &msg, w, Style::new().fg(t.bg).bg(t.info));
    }
}

fn terminal_cursor(app: &App, wl: &WorkbenchLayout) -> Option<(u16, u16)> {
    let content = wl.panel_content?;
    let tab = app.panel.active_tab()?;
    if tab.exited.is_some() {
        return None;
    }
    let screen = crate::app::terminal_snapshot(tab)?;
    if screen.display_offset > 0 {
        return None;
    }
    let (r, c) = screen.cursor?;
    ((r as u16) < content.height && (c as u16) < content.width).then(|| (content.x + c as u16, content.y + r as u16))
}
