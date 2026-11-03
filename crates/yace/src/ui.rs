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
    // Very short terminals drop the status bar first (spec §3.3).
    let status_h = if area.height >= 3 { area.height.min(1) } else { 0 };
    let status = Rect { x: area.x, y: area.bottom().saturating_sub(status_h), width: area.width, height: status_h };
    let main = Rect { height: area.height.saturating_sub(status_h), ..area };
    let narrow = main.width < 50;
    let show_sidebar = app.sidebar_visible && (!narrow || app.focus == Focus::Explorer || app.active().is_none());
    let right = app.settings.workbench.side_bar_location == yace_config::SideBarLocation::Right;
    let (sidebar, sidebar_border, editor) = if show_sidebar && main.width >= 12 {
        let w = if narrow {
            main.width.saturating_sub(1)
        } else {
            app.sidebar_width.min(main.width.saturating_sub(21)).max(10)
        };
        let rest = main.width.saturating_sub(w + 1);
        if right {
            let ed = Rect { width: rest, ..main };
            let border = Rect { x: main.x + rest, width: 1, ..main };
            let sb = Rect { x: main.x + rest + 1, width: w, ..main };
            (Some(sb), Some(border), ed)
        } else {
            let sb = Rect { width: w, ..main };
            let border = Rect { x: main.x + w, width: 1, ..main };
            let ed = Rect { x: main.x + w + 1, width: rest, ..main };
            (Some(sb), Some(border), ed)
        }
    } else {
        (None, None, main)
    };
    // Bottom panel below the editor groups (the sidebar stays full height).
    // On short terminals it only shows while it has focus or is maximised.
    let roomy = editor.height >= 12 || app.focus == Focus::Terminal || app.panel.maximized;
    let (editor, panel_border, panel_tabs, panel_content) = if app.panel.visible && editor.height >= 6 && roomy {
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
            // Tabs only when there is room for text below them.
            let tabs = Rect { height: if rect.height >= 3 { 1 } else { 0 }, ..rect };
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
    /// Nerd Font type icon, drawn before the label (2 cells).
    pub icon: Option<crate::icons::Icon>,
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
    let mut widths: Vec<(usize, String, u16, Option<crate::icons::Icon>)> = group
        .views
        .iter()
        .enumerate()
        .map(|(i, v)| {
            let label = tab_label(app, v.doc, &names);
            let icon = app.nerd_icon(&app.docs.name(v.doc), false);
            let iw = if icon.is_some() { 2 } else { 0 };
            let w = (UnicodeWidthStr::width(label.as_str()) as u16 + 4 + iw).min(area.width.max(8));
            (i, label, w, icon)
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
    for (i, label, w, icon) in widths.drain(start..) {
        if x >= area.right() {
            break;
        }
        let w = w.min(area.right() - x);
        out.push(TabCell { view: i, x, width: w, label, close_x: x + w.saturating_sub(2), icon });
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
    if app.overlays.is_empty() && app.focus == Focus::Editor {
        draw_lsp_popups(app, buf, &wl, area);
    }
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

/// The gutter mark for `line`: added/modified bar, or a deletion marker on
/// the line after removed lines.
fn change_mark(
    app: &App,
    hunks: &[yace_git::Hunk],
    line: u32,
    line_count: u32,
) -> Option<(&'static str, ratatui::style::Color)> {
    use yace_git::HunkKind;
    let t = &app.theme;
    let ascii = app.glyphs.vbar == "|";
    let mono = t.is_mono();
    // Hunks are sorted; find the first that ends after `line`.
    let i = hunks.partition_point(|h| h.lines.end <= line && !(h.lines.is_empty() && h.lines.start == line));
    let h = hunks.get(i)?;
    match h.kind {
        HunkKind::Added if h.lines.contains(&line) => Some((
            if mono {
                "+"
            } else if ascii {
                "|"
            } else {
                "▎"
            },
            t.git_added,
        )),
        HunkKind::Modified if h.lines.contains(&line) => Some((
            if mono {
                "~"
            } else if ascii {
                "|"
            } else {
                "▎"
            },
            t.git_modified,
        )),
        HunkKind::Deleted if h.lines.start == line || (h.lines.start >= line_count && line + 1 == line_count) => {
            let below = h.lines.start >= line_count;
            Some((
                if mono {
                    "-"
                } else if ascii {
                    "_"
                } else if below {
                    "▁"
                } else {
                    "▔"
                },
                t.git_deleted,
            ))
        }
        _ => None,
    }
}

/// Colour of a Git state (spec §8.5).
fn git_color(t: &crate::theme::Theme, k: yace_git::Kind) -> ratatui::style::Color {
    use yace_git::Kind;
    match k {
        Kind::Conflicted => t.error,
        Kind::Modified | Kind::Renamed => t.git_modified,
        Kind::Added | Kind::Untracked => t.git_added,
        Kind::Deleted => t.git_deleted,
        Kind::Ignored => t.muted,
    }
}

/// A theme colour named by a plugin (`error`, `warning`, `info`, `muted`,
/// `gitModified`, …); unknown names use the normal text colour.
fn token_color(t: &crate::theme::Theme, name: &str) -> ratatui::style::Color {
    match name {
        "error" => t.error,
        "warning" => t.warning,
        "info" => t.info,
        "muted" | "description" => t.muted,
        "gitModified" | "modified" => t.git_modified,
        "gitAdded" | "added" => t.git_added,
        "gitDeleted" | "deleted" => t.git_deleted,
        "accent" | "focus" => t.focus_border,
        _ => t.sidebar_fg,
    }
}

/// The Source Control view: per repository, its branch and its conflicts,
/// staged changes, changes and untracked files (spec §15.1, §4.7).
fn draw_scm_view(app: &App, buf: &mut Buffer, list: Rect, focused: bool) {
    use crate::app::{ScmGroup, ScmRow};
    let t = &app.theme;
    let muted = Style::new().fg(t.muted).bg(t.sidebar_bg);
    if app.git.repos.is_empty() {
        let msg = if app.git.missing {
            "Git was not found (git.path)"
        } else if !app.settings.git.enabled {
            "Git is turned off (git.enabled)"
        } else if app.git.detecting {
            "Looking for repositories…"
        } else {
            "No Git repository in this workspace"
        };
        put(buf, list.x + 2, list.y, msg, list.width.saturating_sub(2), muted.add_modifier(Modifier::ITALIC));
        return;
    }
    let rows = app.scm_rows();
    let height = list.height as usize;
    let sel = app.scm.selected.min(rows.len().saturating_sub(1));
    let scroll = if sel >= height { sel + 1 - height } else { 0 };
    let g = &app.glyphs;
    for (i, row) in rows.iter().enumerate().skip(scroll).take(height) {
        let y = list.y + (i - scroll) as u16;
        let mut style = t.sidebar();
        if i == sel {
            style = t.sidebar().patch(t.list_selected(focused));
            fill(buf, Rect { x: list.x, y, width: list.width, height: 1 }, style);
        }
        let avail = |x: u16| list.right().saturating_sub(x);
        let mut x = list.x + 1;
        match row {
            ScmRow::Repo(r) => {
                let repo = &app.git.repos[*r];
                let open = !app.scm.collapsed.contains(&repo.info.toplevel.display().to_string());
                x += put(buf, x, y, if open { g.expanded } else { g.collapsed }, avail(x), style);
                x += put(buf, x, y, " ", avail(x), style);
                x += put(
                    buf,
                    x,
                    y,
                    &ellipsize(&repo.name.to_uppercase(), avail(x) as usize, g.ellipsis),
                    avail(x),
                    style.add_modifier(Modifier::BOLD),
                );
                let info = match (&repo.status, &repo.error) {
                    (_, Some(e)) => format!("{} {}", g.warning, e.lines().next().unwrap_or("")),
                    (Some(st), _) => st.label(),
                    (None, None) => "…".into(),
                };
                let s = if repo.error.is_some() {
                    style.fg(t.error)
                } else if i == sel {
                    style
                } else {
                    style.fg(t.muted)
                };
                put(buf, x + 2, y, &ellipsize(&info, avail(x + 2) as usize, g.ellipsis), avail(x + 2), s);
            }
            ScmRow::Group { repo, group, count } => {
                let key = format!("{}#{group:?}", app.git.repos[*repo].info.toplevel.display());
                let open = !app.scm.collapsed.contains(&key);
                x += 2;
                x += put(buf, x, y, if open { g.expanded } else { g.collapsed }, avail(x), style);
                x += put(buf, x, y, " ", avail(x), style);
                let s = if *group == ScmGroup::Conflicts { style.fg(t.error) } else { style };
                x += put(buf, x, y, group.title(), avail(x), s.add_modifier(Modifier::BOLD));
                put(buf, x + 1, y, &count.to_string(), avail(x + 1), if i == sel { style } else { style.fg(t.muted) });
            }
            ScmRow::File { path, kind, .. } => {
                x += 5;
                let trimmed = path.trim_end_matches('/');
                let (dir, name) = match trimmed.rsplit_once('/') {
                    Some((d, n)) => (d, n),
                    None => ("", trimmed),
                };
                if !path.ends_with('/')
                    && let Some(ic) = app.file_icon(name)
                {
                    put(buf, x - 2, y, ic.glyph, 1, style.fg(ic.color));
                }
                let name = if path.ends_with('/') { format!("{name}/") } else { name.to_string() };
                let letter_w = 2;
                let room = avail(x).saturating_sub(letter_w + 1);
                let used = put(
                    buf,
                    x,
                    y,
                    &ellipsize(&name, room as usize, g.ellipsis),
                    room,
                    style.fg(if i == sel && focused { t.list_active_fg } else { git_color(t, *kind) }),
                );
                if !dir.is_empty() && room > used + 2 {
                    put(
                        buf,
                        x + used + 1,
                        y,
                        &ellipsize(dir, (room - used - 1) as usize, g.ellipsis),
                        room - used - 1,
                        if i == sel { style } else { muted.bg(style.bg.unwrap_or(t.sidebar_bg)) },
                    );
                }
                put(buf, list.right().saturating_sub(letter_w + 1), y, kind.letter(), 1, style.fg(git_color(t, *kind)));
            }
        }
    }
}

/// A plugin's tree view, drawn by the editor (spec §18.12: plugins never
/// write to the screen themselves).
fn draw_plugin_view(app: &App, buf: &mut Buffer, list: Rect, focused: bool) {
    let t = &app.theme;
    let rows = app.plugin_view_rows();
    if rows.is_empty() {
        let s = Style::new().fg(t.muted).bg(t.sidebar_bg).add_modifier(Modifier::ITALIC);
        put(
            buf,
            list.x + 2,
            list.y,
            &ellipsize(&app.plugin_view_empty_text(), list.width.saturating_sub(3) as usize, app.glyphs.ellipsis),
            list.width.saturating_sub(2),
            s,
        );
        return;
    }
    let ui = app.plugin_view_ui();
    let height = list.height as usize;
    let sel = ui.selected.min(rows.len() - 1);
    let scroll = if sel >= height { sel + 1 - height } else { 0 };
    let g = &app.glyphs;
    for (i, (depth, node, has, open)) in rows.iter().enumerate().skip(scroll).take(height) {
        let y = list.y + (i - scroll) as u16;
        let mut style = t.sidebar();
        if i == sel {
            style = t.sidebar().patch(t.list_selected(focused));
            fill(buf, Rect { x: list.x, y, width: list.width, height: 1 }, style);
        }
        let avail = |x: u16| list.right().saturating_sub(x);
        let mut x = list.x + 1 + ((*depth as u16) * 2).min(list.width.saturating_sub(4));
        let marker = if *has { if *open { g.expanded } else { g.collapsed } } else { " " };
        x += put(buf, x, y, marker, avail(x), style);
        x += put(buf, x, y, " ", avail(x), style);
        if !node.icon.is_empty() {
            let icon: String = node.icon.chars().filter(|c| !c.is_control()).take(2).collect();
            x += put(buf, x, y, &icon, avail(x), style.fg(token_color(t, &node.color)));
            x += put(buf, x, y, " ", avail(x), style);
        }
        let label: String = node.label.chars().filter(|c| !c.is_control()).collect();
        let lstyle = if node.color.is_empty() || i == sel { style } else { style.fg(token_color(t, &node.color)) };
        x += put(buf, x, y, &ellipsize(&label, avail(x) as usize, g.ellipsis), avail(x), lstyle);
        if !node.description.is_empty() && avail(x) > 4 {
            let d: String = node.description.chars().filter(|c| !c.is_control()).collect();
            put(buf, x + 1, y, &ellipsize(&d, avail(x + 1) as usize, g.ellipsis), avail(x + 1), style.fg(t.muted));
        }
    }
}

fn draw_sidebar(app: &App, buf: &mut Buffer, wl: &WorkbenchLayout) {
    let (Some(sb), Some(title), Some(list)) = (wl.sidebar, wl.explorer_title, wl.explorer_list) else { return };
    let t = &app.theme;
    let focused = app.focus == Focus::Explorer && app.overlays.is_empty();
    fill(buf, sb, t.sidebar());
    if app.sidebar_view == SidebarView::Git {
        let n: usize = app
            .git
            .repos
            .iter()
            .filter_map(|r| r.status.as_ref())
            .map(|s| s.files.iter().filter(|f| !f.ignored).count())
            .sum();
        let head = if n > 0 { format!(" SOURCE CONTROL ({n})") } else { " SOURCE CONTROL".into() };
        put(buf, title.x, title.y, &head, title.width, t.title(focused));
        if let Some(b) = wl.sidebar_border {
            fill_with(
                buf,
                b,
                app.glyphs.vbar,
                Style::new().fg(if focused { t.focus_border } else { t.border }).bg(t.bg),
            );
        }
        draw_scm_view(app, buf, list, focused);
        return;
    }
    if app.sidebar_view == SidebarView::Plugin && app.plugins.active_view.is_some() {
        put(buf, title.x, title.y, &format!(" {}", app.plugin_view_title()), title.width, t.title(focused));
        if let Some(b) = wl.sidebar_border {
            fill_with(
                buf,
                b,
                app.glyphs.vbar,
                Style::new().fg(if focused { t.focus_border } else { t.border }).bg(t.bg),
            );
        }
        draw_plugin_view(app, buf, list, focused);
        return;
    }
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
            style = t.sidebar().patch(t.list_selected(focused));
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
                let name = row.path.file_name().map(|n| n.to_string_lossy()).unwrap_or_default();
                let is_dir = matches!(kind, RowKind::Dir | RowKind::SymlinkDir);
                let (marker, icon) = if is_dir {
                    (
                        if row.expanded { g.expanded } else { g.collapsed },
                        if row.expanded { g.folder_open } else { g.folder },
                    )
                } else {
                    (" ", g.file)
                };
                // Unicode: a coloured dot in the (empty) marker column of a
                // file; Nerd Font: the type's devicon in the icon column.
                let typed = if is_dir { app.nerd_icon(&name, true) } else { app.file_icon(&name) };
                let nerd = app.settings.workbench.glyphs == yace_config::Glyphs::NerdFont;
                match typed {
                    Some(ic) if !nerd => {
                        x += put(buf, x, y, ic.glyph, avail(x), style.fg(ic.color));
                        x += put(buf, x, y, " ", avail(x), style);
                        x += put(buf, x, y, icon, avail(x), style);
                    }
                    Some(ic) => {
                        x += put(buf, x, y, marker, avail(x), style);
                        x += put(buf, x, y, " ", avail(x), style);
                        x += put(buf, x, y, ic.glyph, avail(x), style.fg(ic.color));
                        x += put(buf, x, y, " ", avail(x), style);
                    }
                    None => {
                        x += put(buf, x, y, marker, avail(x), style);
                        x += put(buf, x, y, " ", avail(x), style);
                        x += put(buf, x, y, icon, avail(x), style);
                    }
                }
                let dirty =
                    app.docs.find_by_path(&row.path).and_then(|d| app.docs.doc(d)).is_some_and(|d| d.is_dirty());
                let mut s = style;
                if matches!(kind, RowKind::Broken) {
                    s = s.fg(t.error).add_modifier(Modifier::CROSSED_OUT);
                }
                let git = app.git_decoration(&row.path);
                if let Some(k) = git {
                    s = s.fg(git_color(t, k));
                }
                if app.is_cut(&row.path) {
                    s = s.add_modifier(Modifier::DIM);
                }
                let mut label = row.label.clone();
                if matches!(kind, RowKind::SymlinkDir | RowKind::SymlinkFile) {
                    label = format!("{label} {}", g.symlink);
                }
                let badge = app.plugin_decorations.get(&crate::editor::path_key(&row.path));
                // Git letter (files) or dot (folders with changes), then plugin badges.
                let git_mark = git
                    .filter(|k| *k != yace_git::Kind::Ignored)
                    .map(|k| if row.is_dir_like() { "•" } else { k.letter() });
                let badge_w = badge.map(|(b, _)| UnicodeWidthStr::width(b.as_str()) as u16 + 1).unwrap_or(0)
                    + git_mark.map(|_| 2).unwrap_or(0);
                let room = avail(x).saturating_sub(if dirty { 2 } else { 0 } + badge_w);
                x += put(buf, x, y, &ellipsize(&label, room as usize, g.ellipsis), room, s);
                if dirty {
                    put(buf, x + 1, y, g.dirty, avail(x + 1), style.fg(t.dirty));
                }
                let mut bx = list.right().saturating_sub(badge_w + 1);
                if let (Some(m), Some(k)) = (git_mark, git) {
                    put(buf, bx, y, m, 1, style.fg(git_color(t, k)));
                    bx += 2;
                }
                if let Some((b, color)) = badge {
                    put(buf, bx, y, b, badge_w, style.fg(token_color(t, color)));
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
        let mut style = t.tab(active);
        if v.preview {
            style = style.add_modifier(Modifier::ITALIC);
        }
        fill(buf, Rect { x: cell.x, y: gl.tabs.y, width: cell.width, height: 1 }, style);
        let iw = if cell.icon.is_some() && cell.width > 8 { 2 } else { 0 };
        let label = ellipsize(&cell.label, cell.width.saturating_sub(4 + iw) as usize, app.glyphs.ellipsis);
        let doc = app.docs.doc(v.doc);
        if let (Some(ic), true) = (cell.icon, iw > 0) {
            put(buf, cell.x + 1, gl.tabs.y, ic.glyph, 1, style.fg(ic.color));
        }
        let label_style = match doc.map(|d| d.disk_state) {
            Some(DiskState::Deleted) => style.add_modifier(Modifier::CROSSED_OUT),
            _ => style,
        };
        put(buf, cell.x + 1 + iw, gl.tabs.y, &label, cell.width.saturating_sub(3 + iw), label_style);
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
    let sel_style = t.selection(group_focused);
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
    // Diagnostics intersecting the viewport: underline + gutter colour.
    let diag_color = |sev: u8| match sev {
        1 => t.error,
        2 => t.warning,
        3 => t.info,
        _ => t.muted,
    };
    let vis_diags: Vec<(usize, usize, u8)> = app
        .doc_diagnostics(view.doc)
        .iter()
        .filter_map(|x| x.chars.map(|(a, b)| (a, if b > a { b } else { a + 1 }, x.severity)))
        .filter(|(a, b, _)| *b > vis_start && *a < vis_end)
        .collect();
    let mut line_sev: std::collections::HashMap<usize, u8> = std::collections::HashMap::new();
    for (a, _, s) in &vis_diags {
        let l = rope.char_to_line((*a).min(rope.len_chars()));
        let e = line_sev.entry(l).or_insert(*s);
        *e = (*e).min(*s);
    }
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
    let is_diff = app.syntax.language(view.doc).id == "diff";
    // Git change bars in the gutter's last column (spec §15.1).
    let hunks = app.git_hunks(view.doc);
    let change_x = gl.content.x + gl.gutter.saturating_sub(1);
    let line_count = rope.len_lines().saturating_sub(1).max(1) as u32;
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
                let fg = match line_sev.get(&line) {
                    Some(sev) => diag_color(*sev),
                    None if is_cursor_line => t.line_number_active,
                    None => t.line_number,
                };
                let style = Style::new().fg(fg).bg(t.bg);
                put(buf, gl.content.x, y, &s, gl.gutter, style);
            }
            if !hunks.is_empty()
                && gl.gutter >= 3
                && let Some((glyph, color)) = change_mark(app, hunks, line as u32, line_count)
            {
                put(buf, change_x, y, glyph, 1, Style::new().fg(color).bg(t.bg));
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
        // Unified diffs are coloured by line prefix (no parser needed).
        let diff_style = is_diff.then(|| diff_line_style(t, &content.chars().take(4).collect::<String>())).flatten();
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
                let mut style = diff_style.unwrap_or_else(|| syntax_style(t, scope)).bg(line_bg);
                if !vis_diags.is_empty()
                    && let Some(sev) = vis_diags.iter().filter(|(a, b, _)| ci >= *a && ci < *b).map(|x| x.2).min()
                {
                    style = style.add_modifier(Modifier::UNDERLINED).underline_color(diag_color(sev));
                }
                if let Some(m) = find_matches.iter().find(|(a, b)| ci >= *a && ci < *b) {
                    style = style.patch(t.find_match(Some(*m) == current_match));
                }
                if selected {
                    style = style.patch(sel_style);
                }
                if bracket.is_some_and(|(a, b)| ci == a || ci == b) {
                    style = if t.is_mono() {
                        style.add_modifier(Modifier::UNDERLINED)
                    } else {
                        style.bg(t.bracket_match_bg)
                    };
                }
                if line_heads.contains(&ci) && ci != head && group_focused {
                    style = t.cursor();
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
                put(buf, end_x as u16, y, " ", 1, t.cursor());
            } else if nl_selected {
                put(buf, end_x as u16, y, " ", 1, sel_style);
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

/// Completion list, parameter hints and hover, anchored at the cursor.
fn draw_lsp_popups(app: &App, buf: &mut Buffer, wl: &WorkbenchLayout, area: Rect) {
    if app.completion.is_none() && app.hover.is_none() && app.signature.is_none() {
        return;
    }
    let Some((cx, cy)) = editor_cursor(app, wl) else { return };
    let Some((g, v, d)) = app.active() else { return };
    let t = &app.theme;
    let bottom = wl.status.y; // keep the status bar visible
    let base = Style::new().fg(t.overlay_fg).bg(t.overlay_bg);
    let head = app.layout.group(g).map(|gr| gr.views[v].sel.primary().head).unwrap_or(0);
    // Parameter hints: one row above the cursor (below on the top row).
    if let Some(sig) = app.signature.as_ref().filter(|s| s.doc == d) {
        let w = (UnicodeWidthStr::width(sig.label.as_str()) as u16 + 2).min(area.width);
        let y = if cy > area.y { cy - 1 } else { cy + 1 };
        let x = cx.min(area.right().saturating_sub(w));
        fill(buf, Rect { x, y, width: w, height: 1 }, base);
        let mut xx = x + 1;
        for (i, ch) in sig.label.chars().enumerate() {
            let s = ch.to_string();
            let cw = UnicodeWidthStr::width(s.as_str()).max(1) as u16;
            if xx + cw > x + w.saturating_sub(1) {
                break;
            }
            let active = sig.active.is_some_and(|(a, b)| i >= a && i < b);
            let st =
                if active { base.fg(t.match_fg).add_modifier(Modifier::BOLD | Modifier::UNDERLINED) } else { base };
            put(buf, xx, y, &s, cw, st);
            xx += cw;
        }
    }
    // Hover: a box above the cursor (below if there is no room).
    if let Some(h) = app.hover.as_ref().filter(|h| h.doc == d) {
        let maxw = 80.min(area.width.saturating_sub(2)) as usize;
        let mut lines: Vec<String> = Vec::new();
        for l in &h.lines {
            if l.is_empty() {
                lines.push(String::new());
                continue;
            }
            let mut cur = String::new();
            for word in l.split(' ') {
                if !cur.is_empty() && UnicodeWidthStr::width(cur.as_str()) + UnicodeWidthStr::width(word) + 1 > maxw {
                    lines.push(std::mem::take(&mut cur));
                }
                if !cur.is_empty() {
                    cur.push(' ');
                }
                cur.push_str(word);
            }
            lines.push(cur);
        }
        lines.truncate(15);
        let w =
            (lines.iter().map(|l| UnicodeWidthStr::width(l.as_str())).max().unwrap_or(0) as u16 + 2).min(area.width);
        let hgt = lines.len() as u16;
        let y = if cy >= area.y + hgt { cy - hgt } else { (cy + 1).min(bottom.saturating_sub(hgt)) };
        let x = cx.min(area.right().saturating_sub(w));
        for (i, l) in lines.iter().enumerate() {
            let yy = y + i as u16;
            fill(buf, Rect { x, y: yy, width: w, height: 1 }, base);
            put(buf, x + 1, yy, l, w.saturating_sub(2), base);
        }
    }
    // Completion list below the cursor (above if there is no room).
    if let Some(c) = app.completion.as_ref().filter(|c| c.doc == d) {
        let rows = c.shown.len().min(10) as u16;
        if rows == 0 {
            return;
        }
        let scroll = c.selected.saturating_sub(rows as usize - 1);
        let visible = &c.shown[scroll..(scroll + rows as usize).min(c.shown.len())];
        let kind_of = |i: usize| crate::app::completion_kind(c.items[i].kind.unwrap_or(0));
        let query = yace_search::fuzzy::Query::new(c.filtered_for.as_deref().unwrap_or(""));
        let label_w = visible.iter().map(|i| UnicodeWidthStr::width(c.items[*i].label.as_str())).max().unwrap_or(0);
        let detail_w = visible
            .iter()
            .map(|i| c.items[*i].detail.as_deref().map(UnicodeWidthStr::width).unwrap_or(0))
            .max()
            .unwrap_or(0)
            .min(40);
        let kind_w = visible.iter().map(|i| kind_of(*i).len()).max().unwrap_or(0);
        let w = ((label_w + detail_w + kind_w + 6) as u16).clamp(20, 80).min(area.width);
        let typed = head.saturating_sub(c.anchor) as u16;
        let x = cx.saturating_sub(typed + 1).min(area.right().saturating_sub(w));
        let y = if cy + 1 + rows <= bottom { cy + 1 } else { cy.saturating_sub(rows) };
        for (r, i) in visible.iter().enumerate() {
            let item = &c.items[*i];
            let hl = match item.filter_text {
                None => yace_search::fuzzy::match_label(&query, &item.label).map(|m| m.indices).unwrap_or_default(),
                Some(_) => Vec::new(),
            };
            let yy = y + r as u16;
            let selected = scroll + r == c.selected;
            let st = if selected { t.overlay_selected() } else { Style::new().fg(t.overlay_fg).bg(t.overlay_bg) };
            fill(buf, Rect { x, y: yy, width: w, height: 1 }, st);
            let kind = kind_of(*i);
            let kx = (x + w).saturating_sub(kind.len() as u16 + 1);
            let mut xx = x + 1;
            for (ci, ch) in item.label.chars().enumerate() {
                let s = ch.to_string();
                let cw = UnicodeWidthStr::width(s.as_str()).max(1) as u16;
                if xx + cw > kx.saturating_sub(1) {
                    break;
                }
                let style = if hl.contains(&ci) { st.fg(t.match_fg).add_modifier(Modifier::BOLD) } else { st };
                put(buf, xx, yy, &s, cw, style);
                xx += cw;
            }
            if let Some(det) = &item.detail {
                let dx = xx + 2;
                if dx + 4 < kx {
                    put(buf, dx, yy, det, kx.saturating_sub(dx + 1), st.fg(t.muted));
                }
            }
            put(buf, kx, yy, kind, kind.len() as u16, st.fg(t.muted));
        }
    }
}

/// Style of a unified-diff line from its first characters.
fn diff_line_style(t: &crate::theme::Theme, start: &str) -> Option<Style> {
    let s = Style::new();
    Some(
        if start.starts_with("+++")
            || start.starts_with("---")
            || start.starts_with("diff")
            || start.starts_with("index")
        {
            s.fg(t.muted).add_modifier(Modifier::BOLD)
        } else if start.starts_with("@@") {
            s.fg(t.info)
        } else if start.starts_with('+') {
            s.fg(t.git_added)
        } else if start.starts_with('-') {
            s.fg(t.git_deleted)
        } else {
            return None;
        },
    )
}

fn syntax_style(t: &crate::theme::Theme, scope: Option<yace_syntax::Scope>) -> Style {
    t.syntax_style(scope)
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
    let base = t.status(app.workspace.roots().is_empty());
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
        if let Some(s) = app.lsp_status() {
            right.push(s);
        }
        right.push(app.syntax.language(view.doc).name.into());
        // Problems: counts, and the message under the cursor instead of the path.
        let (e, w) = app.lsp.counts();
        if e + w > 0 || !app.lsp.servers.is_empty() {
            let counts = format!(" {} {e} {} {w} ", app.glyphs.error, app.glyphs.warning);
            left = match app.diagnostic_at_cursor() {
                Some(dg) => {
                    let g = if dg.severity == 1 { app.glyphs.error } else { app.glyphs.warning };
                    format!("{counts}  {g} {}", dg.message.lines().next().unwrap_or(""))
                }
                None => format!("{counts} {left}"),
            };
        }
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
    // The branch stays at the far left, even under notifications.
    let branch = app.git_status_label().map(|b| format!(" {b} "));
    let (pl, pr) = app.plugin_status_texts();
    if !pl.is_empty() {
        left = format!("{left}   {}", pl.join("   "));
    }
    for (i, item) in pr.into_iter().enumerate() {
        right.insert(i, item);
    }
    let right_text = format!("{}  ", right.join("   "));
    let rw = UnicodeWidthStr::width(right_text.as_str()) as u16;
    let lw_max = r.width.saturating_sub(rw + 1);
    let shown_right = if lw_max < 10 { 0 } else { rw };
    let mut left_max = r.width.saturating_sub(shown_right);
    let mut lx = r.x;
    if let Some(b) = &branch {
        let bw = (UnicodeWidthStr::width(b.as_str()) as u16).min(left_max / 2);
        put(buf, lx, r.y, b, bw, base);
        lx += bw;
        left_max = left_max.saturating_sub(bw);
    }
    let left = ellipsize(&left, left_max as usize, app.glyphs.ellipsis);
    fill(buf, Rect { x: lx, width: (UnicodeWidthStr::width(left.as_str()) as u16 + 1).min(left_max), ..r }, left_style);
    put(buf, lx, r.y, &left, left_max, left_style);
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
        Overlay::Keybindings(_) => {
            let w = area.width.saturating_sub(4).min(160);
            let outer = Rect {
                x: area.x + (area.width - w) / 2,
                y: area.y + 1.min(area.height),
                width: w,
                height: area.height.saturating_sub(2),
            };
            let inner_w = outer.width.saturating_sub(2);
            let input = Rect { x: outer.x + 1, y: outer.y + 1, width: inner_w, height: 1 };
            // Title, input, header above; status and footer below.
            let list = Rect { x: outer.x + 1, y: outer.y + 3, width: inner_w, height: outer.height.saturating_sub(6) };
            OverlayGeom { outer, input: Some(input), list: Some(list), buttons: vec![] }
        }
    }
}

/// Column widths of the keybinding editor: (title, keys, when, source).
fn kb_columns(width: u16) -> (u16, u16, u16, u16) {
    let w = width.saturating_sub(1);
    if w < 60 {
        let keys = (w / 3).min(22);
        return (w.saturating_sub(keys + 1), keys, 0, 0);
    }
    let (keys, source) = (24, 8);
    let when = (w / 4).min(36);
    (w.saturating_sub(keys + when + source + 3), keys, when, source)
}

fn draw_keybindings(
    app: &App,
    buf: &mut Buffer,
    geom: &OverlayGeom,
    e: &crate::keybindings_editor::KeybindingEditor,
) -> Option<(u16, u16)> {
    use crate::keybindings_editor::KbMode;
    let t = &app.theme;
    let base = Style::new().fg(t.overlay_fg).bg(t.overlay_bg);
    let muted = Style::new().fg(t.muted).bg(t.overlay_bg);
    draw_box(app, buf, geom.outer, &format!("Keyboard Shortcuts ({} of {})", e.filtered.len(), e.rows.len()));
    let input = geom.input?;
    let list = geom.list?;
    if geom.outer.height < 7 || list.width < 10 {
        return None;
    }
    let mut cursor = None;
    match &e.mode {
        KbMode::RecordSearch(keys) => {
            let shown: Vec<String> = keys.iter().map(ToString::to_string).collect();
            let text = format!(
                " Recording keys: {}   (Alt+K or Esc to stop)",
                if shown.is_empty() { "press keys…".into() } else { shown.join(" ") }
            );
            fill(buf, input, Style::new().fg(t.overlay_fg).bg(t.input_bg));
            put(buf, input.x, input.y, &text, input.width, Style::new().fg(t.warning).bg(t.input_bg));
        }
        _ => {
            cursor =
                Some(draw_input(app, buf, input, &e.input, "Search: command, keys, when; @conflicts @user @unbound"))
        }
    }
    let (tw, kw, ww, sw) = kb_columns(list.width);
    let header_y = list.y - 1;
    let head = Style::new().fg(t.muted).bg(t.overlay_bg).add_modifier(Modifier::BOLD);
    let mut x = list.x + 1;
    for (label, w) in [("Command", tw), ("Keybinding", kw), ("When", ww), ("Source", sw)] {
        if w > 0 {
            put(buf, x, header_y, label, w, head);
            x += w + 1;
        }
    }
    let height = list.height as usize;
    let scroll = if e.selected >= height { e.selected + 1 - height } else { 0 };
    if e.filtered.is_empty() {
        put(buf, list.x + 1, list.y, "No matching commands", list.width, muted);
    }
    for (i, &ri) in e.filtered.iter().enumerate().skip(scroll).take(height) {
        let row = &e.rows[ri];
        let y = list.y + (i - scroll) as u16;
        let selected = i == e.selected;
        let style = if selected { t.overlay_selected() } else { base };
        fill(buf, Rect { x: list.x, y, width: list.width, height: 1 }, style);
        let mut x = list.x + 1;
        let used = put(buf, x, y, &ellipsize(&row.title, tw as usize, app.glyphs.ellipsis), tw, style);
        if (used as usize) + row.command.len() + 3 <= tw as usize && row.title != row.command {
            put(buf, x + used + 2, y, &row.command, tw - used - 2, if selected { style } else { muted });
        }
        x += tw + 1;
        let keys_style = if row.conflict { style.fg(t.warning) } else { style };
        let mut keys = row.keys_text();
        if row.conflict {
            keys = format!("{} {keys}", app.glyphs.warning);
        }
        put(buf, x, y, &ellipsize(&keys, kw as usize, app.glyphs.ellipsis), kw, keys_style);
        x += kw + 1;
        if ww > 0 {
            put(
                buf,
                x,
                y,
                &ellipsize(&row.when, ww as usize, app.glyphs.ellipsis),
                ww,
                if selected { style } else { muted },
            );
            x += ww + 1;
        }
        if sw > 0 {
            put(buf, x, y, row.source_text(), sw, if selected { style } else { muted });
        }
    }
    let status_y = geom.outer.bottom().saturating_sub(3);
    let foot_y = geom.outer.bottom().saturating_sub(2);
    let inner = geom.outer.width.saturating_sub(4);
    match &e.mode {
        KbMode::Define(keys) => {
            let title = e.current().map(|r| r.title.as_str()).unwrap_or("");
            let shown: Vec<String> = keys.iter().map(ToString::to_string).collect();
            let mut text = format!("Press the keys for \"{title}\", then Enter (Esc cancels): {}", shown.join(" "));
            let others: Vec<String> = e
                .bound_to(keys)
                .iter()
                .filter(|r| Some(r.command.as_str()) != e.current().map(|c| c.command.as_str()))
                .map(|r| r.title.clone())
                .collect();
            if !keys.is_empty() && !others.is_empty() {
                text.push_str(&format!("   — also bound to: {}", others.join(", ")));
            }
            put(
                buf,
                geom.outer.x + 2,
                status_y,
                &text,
                inner,
                Style::new().fg(t.warning).bg(t.overlay_bg).add_modifier(Modifier::BOLD),
            );
            cursor = None;
        }
        _ if !e.note.is_empty() => {
            put(buf, geom.outer.x + 2, status_y, &e.note, inner, Style::new().fg(t.info).bg(t.overlay_bg));
        }
        _ if !app.keyboard_enhanced => {
            put(
                buf,
                geom.outer.x + 2,
                status_y,
                "This terminal can't send some keys (e.g. Ctrl+Shift+letters, Ctrl+Enter); see docs/keybindings.md",
                inner,
                muted,
            );
        }
        _ => {}
    }
    put(
        buf,
        geom.outer.x + 2,
        foot_y,
        "Enter change keys · Del remove · Alt+R reset to default · Alt+K search by keys · Esc close",
        inner,
        muted,
    );
    cursor
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
        let s = if input.all_selected { style.patch(t.selection(true)) } else { style };
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
                let style =
                    if selected { t.overlay_selected() } else { Style::new().fg(t.overlay_fg).bg(t.overlay_bg) };
                fill(buf, Rect { x: list.x, y, width: list.width, height: 1 }, style);
                let hint_w = UnicodeWidthStr::width(it.hint.as_str()) as u16;
                let mut room = list.width.saturating_sub(hint_w + 3);
                let mut x = list.x + 1;
                let mut icon_w = 0;
                // File rows of Quick Open: the type icon, in a 2-cell column
                // (blank for unknown types, so names stay aligned).
                if p.action == crate::actions::PickerAction::QuickOpen
                    && app.settings.workbench.file_icons
                    && app.settings.workbench.glyphs != yace_config::Glyphs::Ascii
                    && app.quick_open_is_file(it.data)
                    && room > 12
                {
                    if let Some(ic) = app.file_icon(&it.label) {
                        put(buf, x, y, ic.glyph, 1, style.fg(ic.color));
                    }
                    x += 2;
                    room -= 2;
                    icon_w = 2;
                }
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
                    let left = (room + icon_w).saturating_sub(x - list.x);
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
        Overlay::Keybindings(e) => draw_keybindings(app, buf, &geom, e),
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
        let base = if selected { t.sidebar().patch(t.list_selected(results_focused)) } else { t.sidebar() };
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
                let mut hs = base.patch(t.find_match(false));
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
        let style =
            if active && !t.is_mono() { Style::new().fg(t.tab_active_fg).bg(t.terminal_bg) } else { t.tab(active) };
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
            if st.selected && t.is_mono() {
                style = style.add_modifier(Modifier::REVERSED);
            }
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
