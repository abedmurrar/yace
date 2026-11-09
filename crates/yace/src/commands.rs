//! The command registry (spec §7, §19). Every action — keybindings, the
//! command palette, mouse handlers, tests — runs through a command id.
//! Ids follow VS Code where an equivalent exists (TODO.md D4).

use std::collections::HashMap;

use serde_json::Value;
use yace_buffer::movement::Direction;
use yace_workspace::session::SplitAxis;

use crate::{
    actions::{PickerAction, PromptAction, RootAction, WorkspaceTarget},
    app::{App, Focus, Move, Op, Severity},
    overlay::{Overlay, Picker, PickerItem},
};

/// Why a command did not run (spec §19.1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CommandError {
    /// No command has this id.
    Unknown(String),
    /// The arguments don't have the expected shape.
    InvalidArgs(String),
    /// Not possible right now (no editor, no terminal, …).
    Unavailable(String),
    /// It ran and failed.
    Failed(String),
}

impl std::fmt::Display for CommandError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CommandError::Unknown(id) => write!(f, "command '{id}' not found"),
            CommandError::InvalidArgs(m) | CommandError::Unavailable(m) | CommandError::Failed(m) => f.write_str(m),
        }
    }
}

pub type CmdResult = Result<(), CommandError>;
pub type Handler = fn(&mut App, Option<&Value>) -> CmdResult;

/// Lets handlers return `()` (always succeeds) or a [`CmdResult`].
pub trait IntoCmdResult {
    fn into_cmd(self) -> CmdResult;
}
impl IntoCmdResult for () {
    fn into_cmd(self) -> CmdResult {
        Ok(())
    }
}
impl IntoCmdResult for CmdResult {
    fn into_cmd(self) -> CmdResult {
        self
    }
}

/// Parse a command's arguments; `None` when there are none (or `null`).
pub fn parse_args<T: serde::de::DeserializeOwned>(v: Option<&Value>, shape: &str) -> Result<Option<T>, CommandError> {
    match v {
        None | Some(Value::Null) => Ok(None),
        Some(v) => T::deserialize(v)
            .map(Some)
            .map_err(|e| CommandError::InvalidArgs(format!("expected arguments {shape} ({e})"))),
    }
}

#[derive(Clone)]
pub struct Command {
    pub id: &'static str,
    pub category: &'static str,
    pub title: &'static str,
    pub handler: Handler,
    /// Listed in the command palette.
    pub palette: bool,
    /// The arguments it accepts, e.g. `{ "text": string }`; empty for none.
    pub args_doc: &'static str,
}

impl std::fmt::Debug for Command {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Command({})", self.id)
    }
}

/// Who handles a command contributed by a plugin.
#[derive(Clone, Debug, PartialEq)]
pub enum PluginCmd {
    /// The plugin's `on_command` (activating it first if needed).
    Plugin(usize),
    /// Show one of the plugin's views (handled by the editor).
    ShowView(usize, String),
}

pub struct Registry {
    list: Vec<Command>,
    index: HashMap<&'static str, usize>,
    palette: Vec<&'static str>,
    /// Commands contributed by plugins (also in `list`).
    plugin: HashMap<&'static str, PluginCmd>,
    builtin_len: usize,
}

/// Plugin command ids and titles live as long as the process: they are
/// leaked once per id (bounded by the installed plugins).
pub(crate) fn intern(s: &str) -> &'static str {
    leak(s)
}

fn leak(s: &str) -> &'static str {
    use std::sync::{Mutex, OnceLock};
    static INTERNED: OnceLock<Mutex<std::collections::HashSet<&'static str>>> = OnceLock::new();
    let mut set = INTERNED.get_or_init(Default::default).lock().unwrap_or_else(|e| e.into_inner());
    if let Some(v) = set.get(s) {
        return v;
    }
    let v: &'static str = Box::leak(s.to_string().into_boxed_str());
    set.insert(v);
    v
}

fn plugin_handler(_: &mut App, _: Option<&Value>) -> CmdResult {
    Ok(())
}

impl Registry {
    pub fn builtin() -> Self {
        let list = builtin_commands();
        debug_assert!(
            {
                let mut ids: Vec<&str> = list.iter().map(|c| c.id).collect();
                ids.sort_unstable();
                ids.windows(2).all(|w| w[0] != w[1])
            },
            "duplicate command id"
        );
        let index = list.iter().enumerate().map(|(i, c)| (c.id, i)).collect();
        let builtin_len = list.len();
        let mut r = Self { list, index, palette: Vec::new(), plugin: HashMap::new(), builtin_len };
        r.rebuild_palette();
        r
    }

    fn rebuild_palette(&mut self) {
        let mut palette: Vec<&Command> = self.list.iter().filter(|c| c.palette).collect();
        palette.sort_by(|a, b| (a.category, a.title).cmp(&(b.category, b.title)));
        self.palette = palette.into_iter().map(|c| c.id).collect();
    }

    /// Replace every plugin-contributed command. Ids that clash with a
    /// built-in command are skipped and returned.
    pub fn set_plugin_commands(&mut self, cmds: Vec<(String, String, String, bool, PluginCmd)>) -> Vec<String> {
        self.list.truncate(self.builtin_len);
        self.index = self.list.iter().enumerate().map(|(i, c)| (c.id, i)).collect();
        self.plugin.clear();
        let mut clashes = Vec::new();
        for (id, category, title, palette, kind) in cmds {
            if self.index.contains_key(id.as_str()) {
                clashes.push(id);
                continue;
            }
            let id = leak(&id);
            self.list.push(Command {
                id,
                category: leak(&category),
                title: leak(&title),
                handler: plugin_handler,
                palette,
                args_doc: "",
            });
            self.index.insert(id, self.list.len() - 1);
            self.plugin.insert(id, kind);
        }
        self.rebuild_palette();
        clashes
    }

    pub fn plugin_command(&self, id: &str) -> Option<&PluginCmd> {
        self.plugin.get(id)
    }
    pub fn get(&self, id: &str) -> Option<&Command> {
        self.index.get(id).map(|i| &self.list[*i])
    }
    pub fn all(&self) -> &[Command] {
        &self.list
    }
    /// Palette entries in display order (index = picker data).
    pub fn palette_ids(&self) -> &[&'static str] {
        &self.palette
    }
}

macro_rules! c {
    ($id:literal, $cat:literal, $title:literal, $f:expr) => {
        c!(@ $id, $cat, $title, $f, true, "")
    };
    (hidden $id:literal, $cat:literal, $title:literal, $f:expr) => {
        c!(@ $id, $cat, $title, $f, false, "")
    };
    (args $doc:literal, $id:literal, $cat:literal, $title:literal, $f:expr) => {
        c!(@ $id, $cat, $title, $f, true, $doc)
    };
    (hidden args $doc:literal, $id:literal, $cat:literal, $title:literal, $f:expr) => {
        c!(@ $id, $cat, $title, $f, false, $doc)
    };
    (@ $id:literal, $cat:literal, $title:literal, $f:expr, $palette:expr, $doc:expr) => {
        Command {
            id: $id,
            category: $cat,
            title: $title,
            palette: $palette,
            args_doc: $doc,
            handler: {
                fn handler(a: &mut App, v: Option<&Value>) -> CmdResult {
                    let f: fn(&mut App, Option<&Value>) -> _ = $f;
                    IntoCmdResult::into_cmd(f(a, v))
                }
                handler
            },
        }
    };
}

/// Fails with "no editor is open" unless an editor is active.
fn need_editor(app: &mut App) -> CmdResult {
    if ed(app) { Ok(()) } else { Err(CommandError::Unavailable("no editor is open".into())) }
}

#[derive(serde::Deserialize)]
struct TextArg {
    text: String,
}

#[derive(serde::Deserialize)]
#[serde(untagged)]
enum LineArg {
    Pos {
        line: usize,
        #[serde(default)]
        column: Option<usize>,
    },
    Number(usize),
    Text(String),
}

#[derive(serde::Deserialize)]
#[serde(untagged)]
enum FileArg {
    Path(String),
    Pos {
        path: String,
        #[serde(default)]
        line: Option<u32>,
        #[serde(default)]
        column: Option<u32>,
    },
}

fn open_file_cmd(a: &mut App, v: Option<&Value>) -> CmdResult {
    let shape = r#""path" or { "path": string, "line"?: number, "column"?: number }"#;
    match parse_args::<FileArg>(v, shape)? {
        None => {
            a.prompt("Open File", "Path of the file to open", a.cwd_with_slash(), PromptAction::OpenFile, true);
            Ok(())
        }
        Some(FileArg::Path(p)) => a.plugin_open_file(&p, 0, 0).map_err(CommandError::Failed),
        Some(FileArg::Pos { path, line, column }) => {
            a.plugin_open_file(&path, line.unwrap_or(0), column.unwrap_or(1)).map_err(CommandError::Failed)
        }
    }
}

fn ed(app: &mut App) -> bool {
    app.active().is_some()
}

fn show_commands(app: &mut App, _: Option<&Value>) {
    app.open_quick_open(">");
}

fn show_info(app: &mut App, title: &str, lines: Vec<String>) {
    app.open_overlay(Overlay::Info { title: title.into(), lines });
}

fn open_config_file(app: &mut App, path: std::path::PathBuf, template: &str) {
    if !path.exists() {
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Err(e) = std::fs::write(&path, template) {
            app.notify(Severity::Error, format!("Could not create {}: {e}", path.display()));
            return;
        }
    }
    app.open_file(&path, false);
}

/// The workspace file's settings, or for a folder opened on its own, its
/// `.vscode/settings.json` (VS Code's folder settings).
fn open_workspace_settings(app: &mut App, _: Option<&Value>) {
    if let yace_workspace::WorkspaceKind::File(f) = &app.workspace.kind {
        let f = f.clone();
        app.open_file(&f, false);
        return;
    }
    let active = app.active().and_then(|(_, _, d)| app.docs.doc(d)).and_then(|d| d.path().map(|p| p.to_path_buf()));
    let root = active
        .and_then(|p| app.workspace.root_for_path(&p).map(|r| r.path.clone()))
        .or_else(|| app.workspace.roots().first().map(|r| r.path.clone()));
    match root {
        Some(r) => open_config_file(
            app,
            r.join(".vscode").join("settings.json"),
            "{\n  // Folder settings (VS Code keys). YACE reads editor, files and search\n  // settings from here; see docs/configuration.md.\n}\n",
        ),
        None => app.notify(Severity::Info, "Open a folder or workspace first"),
    }
}

/// With two parts (Explorer, editor groups) next and previous are the same.
/// F6 / Shift+F6: cycle Editor → Panel → Sidebar (VS Code order), skipping
/// hidden parts. The find bar counts as the editor.
fn cycle_part(app: &mut App, forward: bool) {
    #[derive(PartialEq, Clone, Copy)]
    enum Part {
        Editor,
        Panel,
        Sidebar,
    }
    let mut parts = vec![Part::Editor];
    if app.panel.visible && !app.panel.tabs.is_empty() {
        parts.push(Part::Panel);
    }
    if app.sidebar_visible {
        parts.push(Part::Sidebar);
    }
    let current = match app.focus {
        Focus::Editor | Focus::Find => Part::Editor,
        Focus::Terminal | Focus::TerminalFind => Part::Panel,
        Focus::Explorer => Part::Sidebar,
    };
    let i = parts.iter().position(|p| *p == current).unwrap_or(0) as isize;
    let n = parts.len() as isize;
    match parts[(i + if forward { 1 } else { -1 }).rem_euclid(n) as usize] {
        Part::Editor => app.focus_editor(),
        Part::Panel => app.show_terminal(),
        Part::Sidebar => app.focus_explorer(),
    }
}

fn focus_group_n(app: &mut App, n: usize) {
    let order = app.layout.order();
    if let Some(g) = order.get(n) {
        app.layout.active = *g;
    } else if n == order.len() {
        // Ctrl+2 with one group creates a split (VS Code behaviour).
        app.layout.split(SplitAxis::Horizontal);
    }
    app.focus_editor();
}

fn focus_dir(app: &mut App, forward: bool) {
    let order = app.layout.order();
    if let Some(i) = order.iter().position(|g| *g == app.layout.active) {
        let j = if forward { (i + 1).min(order.len() - 1) } else { i.saturating_sub(1) };
        app.layout.active = order[j];
    }
    app.focus_editor();
}

fn open_at_index(app: &mut App, i: usize) {
    let g = app.layout.active_group_mut();
    if i < g.views.len() {
        g.activate(i);
    }
    app.focus_editor();
}

fn cycle_editor(app: &mut App, delta: isize) {
    let g = app.layout.active_group_mut();
    if g.views.is_empty() {
        return;
    }
    let n = g.views.len() as isize;
    let i = (g.active as isize + delta).rem_euclid(n) as usize;
    g.activate(i);
    app.focus_editor();
}

fn cycle_mru(app: &mut App, forward: bool) {
    let g = app.layout.active_group_mut();
    if g.views.len() < 2 {
        return;
    }
    let target = if forward { g.mru.get(1).copied() } else { g.mru.last().copied() };
    if let Some(d) = target
        && let Some(i) = g.find(d)
    {
        g.activate(i);
    }
    app.focus_editor();
}

fn move_tab(app: &mut App, delta: isize) {
    let g = app.layout.active_group_mut();
    let i = g.active;
    let j = i as isize + delta;
    if j >= 0 && (j as usize) < g.views.len() {
        g.views.swap(i, j as usize);
        g.active = j as usize;
    }
}

fn move_editor_to_group(app: &mut App, forward: bool) {
    let Some((gid, vi, doc)) = app.active() else { return };
    let target = app.layout.next_group_or_split(forward);
    if target == gid {
        return;
    }
    let view = app.layout.group_mut(gid).and_then(|g| g.remove(vi));
    if let Some(mut v) = view {
        v.preview = false;
        if let Some(t) = app.layout.group_mut(target) {
            if let Some(existing) = t.find(doc) {
                t.activate(existing);
            } else {
                t.views.push(v);
                let n = t.views.len() - 1;
                t.activate(n);
            }
        }
        if app.layout.group(gid).is_some_and(|g| g.views.is_empty()) {
            app.layout.close_group(gid);
        }
        app.layout.active = target;
        app.focus_editor();
    }
}

fn close_others(app: &mut App, to_right_only: bool) {
    let g = app.layout.active_group();
    let gid = g.id;
    let active = g.active;
    let docs: Vec<_> = g
        .views
        .iter()
        .enumerate()
        .filter(|(i, _)| if to_right_only { *i > active } else { *i != active })
        .map(|(_, v)| v.doc)
        .collect();
    app.close_views(gid, docs);
}

fn close_all(app: &mut App, saved_only: bool) {
    let g = app.layout.active_group();
    let gid = g.id;
    let docs: Vec<_> = g
        .views
        .iter()
        .map(|v| v.doc)
        .filter(|d| !saved_only || app.docs.doc(*d).is_some_and(|x| !x.is_dirty()))
        .collect();
    app.close_views(gid, docs);
}

fn performance(app: &mut App, _: Option<&Value>) {
    let mut lines = app.metrics.report();
    lines.push(format!("Workspace folders: {}", app.workspace.roots().len()));
    lines.push(format!("Open documents: {}", app.docs.len()));
    lines.push(format!("Watched directories: {} ({} for Quick Open)", app.watched_dirs().len(), app.index_watch.len()));
    lines.push(format!("Keyboard protocol: {}", if app.keyboard_enhanced { "enhanced (Kitty)" } else { "legacy" }));
    show_info(app, "Performance Metrics", lines);
}

fn config_errors(app: &mut App, _: Option<&Value>) {
    let lines = if app.config_errors.is_empty() {
        vec!["No configuration problems.".to_string()]
    } else {
        app.config_errors.iter().map(|e| e.to_string()).collect()
    };
    show_info(app, "Configuration Errors", lines);
}

fn keybinding_conflicts(app: &mut App, _: Option<&Value>) {
    let c = app.keymap.conflicts();
    let lines = if c.is_empty() {
        vec!["No keybinding conflicts.".to_string()]
    } else {
        c.iter()
            .map(|c| {
                let when = c.when.to_string();
                format!(
                    "{}{}: {} (active: {})",
                    c.keys,
                    if when.is_empty() { String::new() } else { format!(" [{when}]") },
                    c.commands.join(", "),
                    c.commands.last().unwrap()
                )
            })
            .collect()
    };
    show_info(app, "Keybinding Conflicts", lines);
}

/// Every notification of this session, newest first and searchable;
/// Enter copies the message (e.g. an install command) to the clipboard.
fn notifications(app: &mut App, _: Option<&Value>) {
    app.show_notification_list();
}

fn notifications_in_editor(app: &mut App, _: Option<&Value>) {
    let text: String = app
        .notification_log
        .iter()
        .rev()
        .map(|(l, t)| format!("[{}] {t}\n", format!("{l:?}").to_lowercase()))
        .collect();
    if text.is_empty() {
        app.notify(Severity::Info, "No notifications");
        return;
    }
    app.new_untitled_with(&text);
}

fn about(app: &mut App, _: Option<&Value>) {
    show_info(
        app,
        "About YACE",
        vec![
            format!("YACE — Yet Another Code Editor {}", env!("CARGO_PKG_VERSION")),
            "A native, terminal-based, multi-root code editor.".into(),
            "Built on alacritty_terminal, ripgrep's crates, Ratatui, crossterm, Ropey and more;".into(),
            "keyboard shortcuts and workspaces follow VS Code. See CREDITS.md.".into(),
            "Licence: MIT OR Apache-2.0; third-party licences in THIRD_PARTY_LICENSES.md.".into(),
            format!("Config: {}", app.paths.config_dir.display()),
            format!("State:  {}", app.paths.state_dir.display()),
        ],
    );
}

fn builtin_commands() -> Vec<Command> {
    vec![
        // --- Workbench -------------------------------------------------------
        c!("workbench.action.showCommands", "View", "Show All Commands", show_commands),
        c!(args r#""prefix", e.g. ">" for commands"#, "workbench.action.quickOpen", "Go", "Go to File…", |a, v| {
            let prefix = parse_args::<String>(v, r#""prefix" (a string)"#)?.unwrap_or_default();
            a.open_quick_open(&prefix);
            Ok(())
        }),
        c!(hidden args r#"{ "text": string }"#, "type", "Edit", "Type", |a, v| {
            let Some(t) = parse_args::<TextArg>(v, r#"{ "text": string }"#)? else {
                return Err(CommandError::InvalidArgs(r#"expected arguments { "text": string }"#.into()));
            };
            need_editor(a)?;
            a.insert_text(&t.text);
            Ok(())
        }),
        c!(args r#"{ "snippet": string }"#, "editor.action.insertSnippet", "Editor", "Insert Snippet", |a, v| {
            #[derive(serde::Deserialize)]
            struct S {
                snippet: String,
            }
            let Some(s) = parse_args::<S>(v, r#"{ "snippet": string }"#)? else {
                return Err(CommandError::InvalidArgs(r#"expected arguments { "snippet": string }"#.into()));
            };
            need_editor(a)?;
            a.insert_snippet(&s.snippet);
            Ok(())
        }),
        c!(hidden args r#"{ "text": string }"#, "workbench.action.terminal.sendSequence", "Terminal", "Send Sequence", |a, v| {
            let Some(t) = parse_args::<TextArg>(v, r#"{ "text": string }"#)? else {
                return Err(CommandError::InvalidArgs(r#"expected arguments { "text": string }"#.into()));
            };
            if a.terminal_send(&t.text) {
                Ok(())
            } else {
                Err(CommandError::Unavailable("no terminal is running".into()))
            }
        }),
        c!("workbench.action.quickOpenHelp", "Help", "Quick Open Help", |a, _| a.open_quick_open("?")),
        c!("workbench.action.navigateBack", "Go", "Go Back", |a, _| a.navigate_back()),
        c!("workbench.action.navigateForward", "Go", "Go Forward", |a, _| a.navigate_forward()),
        c!("workbench.view.explorer", "View", "Show Explorer", |a, _| {
            a.sidebar_view = crate::app::SidebarView::Explorer;
            a.focus_explorer()
        }),
        c!("workbench.view.search", "View", "Show Search", |a, _| a.search_open(false)),
        c!("workbench.action.toggleSidebarVisibility", "View", "Toggle Primary Side Bar Visibility", |a, _| {
            a.sidebar_visible = !a.sidebar_visible;
            if !a.sidebar_visible && a.focus == Focus::Explorer {
                a.focus_editor();
            }
        }),
        c!("yace.sidebar.increaseWidth", "View", "Increase Side Bar Width", |a, _| a.sidebar_width =
            a.sidebar_width.saturating_add(4)),
        c!("yace.sidebar.decreaseWidth", "View", "Decrease Side Bar Width", |a, _| a.sidebar_width =
            a.sidebar_width.saturating_sub(4).max(10)),
        c!("workbench.action.togglePanel", "View", "Toggle Panel", |a, _| a.toggle_panel()),
        c!("workbench.action.terminal.toggleTerminal", "View", "Toggle Terminal", |a, _| a.toggle_terminal()),
        c!("workbench.action.terminal.focus", "Terminal", "Focus Terminal", |a, _| a.show_terminal()),
        c!("workbench.action.terminal.new", "Terminal", "Create New Terminal", |a, _| a.new_terminal_default()),
        c!("yace.terminal.newInRoot", "Terminal", "Open Terminal in Workspace Folder…", |a, _| a
            .new_terminal_in_root_picker()),
        c!("yace.terminal.newInActiveFileDir", "Terminal", "Open Terminal in Active File's Folder", |a, _| {
            a.new_terminal_in_active_file_dir()
        }),
        c!("openInIntegratedTerminal", "Terminal", "Open in Integrated Terminal (Explorer selection)", |a, _| {
            a.new_terminal_in_explorer_dir()
        }),
        c!("workbench.action.terminal.newWithProfile", "Terminal", "Create New Terminal (With Profile)…", |a, _| {
            a.new_terminal_with_profile_picker()
        }),
        c!("workbench.action.terminal.kill", "Terminal", "Kill the Active Terminal Instance", |a, _| a
            .kill_terminal_requested()),
        c!("workbench.action.terminal.rename", "Terminal", "Rename…", |a, _| a.rename_terminal_prompt()),
        c!("workbench.action.terminal.focusNext", "Terminal", "Focus Next Terminal", |a, _| a.focus_terminal_tab(1)),
        c!("workbench.action.terminal.focusPrevious", "Terminal", "Focus Previous Terminal", |a, _| a
            .focus_terminal_tab(-1)),
        c!("workbench.action.terminal.clear", "Terminal", "Clear", |a, _| a.terminal_clear()),
        c!(hidden "workbench.action.terminal.scrollUp", "Terminal", "Scroll Up (Line)", |a, _| a.terminal_scroll(1)),
        c!(hidden "workbench.action.terminal.scrollDown", "Terminal", "Scroll Down (Line)", |a, _| a.terminal_scroll(-1)),
        c!(hidden "workbench.action.terminal.scrollUpPage", "Terminal", "Scroll Up (Page)", |a, _| {
            let p = a.terminal_page();
            a.terminal_scroll(p)
        }),
        c!(hidden "workbench.action.terminal.scrollDownPage", "Terminal", "Scroll Down (Page)", |a, _| {
            let p = a.terminal_page();
            a.terminal_scroll(-p)
        }),
        c!("workbench.action.terminal.copySelection", "Terminal", "Copy Selection", |a, _| {
            a.terminal_copy();
        }),
        c!("workbench.action.terminal.paste", "Terminal", "Paste into Active Terminal", |a, _| a.terminal_paste()),
        c!("workbench.action.terminal.focusFind", "Terminal", "Find", |a, _| a.terminal_find_open()),
        c!("workbench.action.terminal.split", "Terminal", "Split Terminal", |a, _| a.split_terminal()),
        c!("workbench.action.terminal.unsplit", "Terminal", "Unsplit Terminal", |a, _| a.unsplit_terminal()),
        c!("workbench.action.terminal.focusNextPane", "Terminal", "Focus Next Terminal in Terminal Group", |a, _| a
            .focus_terminal_pane(1)),
        c!(
            "workbench.action.terminal.focusPreviousPane",
            "Terminal",
            "Focus Previous Terminal in Terminal Group",
            |a, _| a.focus_terminal_pane(-1)
        ),
        c!("workbench.action.terminal.resizePaneLeft", "Terminal", "Resize Terminal Left", |a, _| a
            .resize_terminal_pane(false)),
        c!("workbench.action.terminal.resizePaneRight", "Terminal", "Resize Terminal Right", |a, _| a
            .resize_terminal_pane(true)),
        c!("workbench.action.terminal.findNext", "Terminal", "Find Next (Up)", |a, _| a.terminal_find_next(true)),
        c!("workbench.action.terminal.findPrevious", "Terminal", "Find Previous (Down)", |a, _| a
            .terminal_find_next(false)),
        c!("workbench.action.terminal.hideFind", "Terminal", "Hide Find", |a, _| a.terminal_find_close()),
        c!(hidden "workbench.action.terminal.toggleFindCaseSensitive", "Terminal", "Toggle Find Match Case", |a, _| a
            .terminal_find_toggle('c')),
        c!(hidden "workbench.action.terminal.toggleFindWholeWord", "Terminal", "Toggle Find Whole Word", |a, _| a
            .terminal_find_toggle('w')),
        c!(hidden "workbench.action.terminal.toggleFindRegex", "Terminal", "Toggle Find Regex", |a, _| a
            .terminal_find_toggle('r')),
        c!("workbench.action.toggleMaximizedPanel", "View", "Toggle Maximized Panel", |a, _| {
            a.panel.maximized = !a.panel.maximized;
            a.show_terminal();
        }),
        c!("yace.panel.increaseHeight", "View", "Increase Panel Height", |a, _| a.panel.height =
            a.panel.height.saturating_add(3)),
        c!("yace.panel.decreaseHeight", "View", "Decrease Panel Height", |a, _| a.panel.height =
            a.panel.height.saturating_sub(3).max(4)),
        c!("workbench.action.findInFiles", "Search", "Find in Files", |a, _| a.search_open(false)),
        c!("workbench.action.replaceInFiles", "Search", "Replace in Files", |a, _| a.search_open(true)),
        c!("search.action.clearSearchResults", "Search", "Clear Search Results", |a, _| a.search_clear()),
        c!("workbench.action.search.toggleQueryDetails", "Search", "Toggle Search Details", |a, _| {
            a.search.show_details = !a.search.show_details;
            a.search_open(false)
        }),
        c!("search.action.replaceAll", "Search", "Replace All in Files", |a, _| a.search_replace_all_confirm()),
        c!("filesExplorer.findInFolder", "Search", "Find in Folder…", |a, _| a.search_in_folder()),
        c!("actions.find", "Editor", "Find", |a, _| a.find_open(false)),
        c!("editor.action.startFindReplaceAction", "Editor", "Replace", |a, _| a.find_open(true)),
        c!("editor.action.nextMatchFindAction", "Editor", "Find Next", |a, _| a.find_next(true)),
        c!("editor.action.previousMatchFindAction", "Editor", "Find Previous", |a, _| a.find_next(false)),
        c!("editor.action.selectAllMatches", "Editor", "Select All Occurrences of Find Match", |a, _| a
            .find_select_all()),
        c!("editor.action.replaceOne", "Editor", "Replace", |a, _| a.replace_one()),
        c!("editor.action.replaceAll", "Editor", "Replace All", |a, _| a.replace_all()),
        c!("closeFindWidget", "Editor", "Close Find", |a, _| a.find_close()),
        c!(hidden "toggleFindCaseSensitive", "Editor", "Toggle Match Case", |a, _| a.find_toggle('c')),
        c!(hidden "toggleFindWholeWord", "Editor", "Toggle Match Whole Word", |a, _| a.find_toggle('w')),
        c!(hidden "toggleFindRegex", "Editor", "Toggle Use Regular Expression", |a, _| a.find_toggle('r')),
        c!(hidden "toggleFindInSelection", "Editor", "Toggle Find in Selection", |a, _| a.find_toggle('l')),
        c!(hidden "yace.find.toggleField", "Editor", "Switch Find/Replace Field", |a, _| a.find_toggle_field()),
        c!("workbench.action.gotoSymbol", "Go", "Go to Symbol in Editor", |a, _| a.open_quick_open("@")),
        c!("workbench.action.showAllSymbols", "Go", "Go to Symbol in Workspace", |a, _| a.open_quick_open("#")),
        c!("editor.action.triggerSuggest", "Editor", "Trigger Suggest", |a, _| a.request_completion(None, true)),
        c!("toggleSuggestionDetails", "Editor", "Toggle Suggestion Details", |a, _| a.toggle_suggestion_details()),
        c!("editor.action.triggerParameterHints", "Editor", "Trigger Parameter Hints", |a, _| a
            .request_signature_help()),
        c!("editor.action.showHover", "Editor", "Show Hover", |a, _| a.lsp_hover()),
        c!("editor.action.rename", "Editor", "Rename Symbol", |a, _| a.lsp_rename_prompt()),
        c!("editor.action.revealDefinition", "Editor", "Go to Definition", |a, _| a
            .lsp_goto("textDocument/definition", "Definition")),
        c!("editor.action.revealDeclaration", "Editor", "Go to Declaration", |a, _| a
            .lsp_goto("textDocument/declaration", "Declaration")),
        c!("editor.action.goToTypeDefinition", "Editor", "Go to Type Definition", |a, _| a
            .lsp_goto("textDocument/typeDefinition", "Type definition")),
        c!("editor.action.goToImplementation", "Editor", "Go to Implementations", |a, _| a
            .lsp_goto("textDocument/implementation", "Implementation")),
        c!("editor.action.goToReferences", "Editor", "Go to References", |a, _| a.lsp_references()),
        c!("editor.action.quickFix", "Editor", "Quick Fix…", |a, _| a.lsp_code_actions()),
        c!("editor.action.formatDocument", "Editor", "Format Document", |a, _| {
            if let Some((_, _, d)) = a.active() {
                a.lsp_format(d, false, false);
            }
        }),
        c!("editor.action.formatSelection", "Editor", "Format Selection", |a, _| {
            if let Some((_, _, d)) = a.active() {
                a.lsp_format(d, true, false);
            }
        }),
        c!("editor.action.marker.nextInFiles", "Go", "Go to Next Problem", |a, _| a.next_problem(true)),
        c!("editor.action.marker.prevInFiles", "Go", "Go to Previous Problem", |a, _| a.next_problem(false)),
        c!("workbench.actions.view.problems", "View", "Problems", |a, _| a.show_problems()),
        c!("yace.lsp.restart", "Developer", "Restart Language Server", |a, _| a.lsp_restart_cmd()),
        c!("yace.lsp.stop", "Developer", "Stop Language Server", |a, _| a.lsp_stop_cmd()),
        c!("yace.lsp.showRunning", "Developer", "Show Running Language Servers", |a, _| a.lsp_show_running()),
        c!("yace.lsp.showOutput", "Developer", "Show Language Server Output", |a, _| a.lsp_show_output()),
        c!(hidden "selectNextSuggestion", "Editor", "Select Next Suggestion", |a, _| a.completion_move(1)),
        c!(hidden "selectPrevSuggestion", "Editor", "Select Previous Suggestion", |a, _| a.completion_move(-1)),
        c!(hidden "selectNextPageSuggestion", "Editor", "Select Next Page", |a, _| a.completion_move(8)),
        c!(hidden "selectPrevPageSuggestion", "Editor", "Select Previous Page", |a, _| a.completion_move(-8)),
        c!(hidden "acceptSelectedSuggestion", "Editor", "Accept Suggestion", |a, _| a.accept_completion()),
        c!(hidden "hideSuggestWidget", "Editor", "Hide Suggestions", |a, _| {
            a.completion = None;
        }),
        c!(hidden "closeParameterHints", "Editor", "Close Parameter Hints", |a, _| {
            a.signature = None;
        }),
        c!(hidden "editor.action.hideHover", "Editor", "Hide Hover", |a, _| {
            a.hover = None;
        }),
        c!("workbench.action.selectTheme", "Preferences", "Color Theme", |a, _| a.open_theme_picker()),
        c!("yace.createThemeFromCurrent", "Preferences", "Create Theme from Current", |a, _| a
            .create_theme_from_current()),
        c!("yace.editUserTheme", "Preferences", "Edit User Theme…", |a, _| a.edit_user_theme()),
        c!("yace.importVSCodeTheme", "Preferences", "Import VS Code Color Theme…", |a, _| a.import_vscode_theme()),
        c!("yace.importVSCodeThemeFile", "Preferences", "Import VS Code Color Theme from File…", |a, _| a
            .import_vscode_theme_file_prompt()),
        c!("editor.action.inspectTMScopes", "Developer", "Inspect Editor Tokens", |a, _| a.inspect_tokens()),
        c!("workbench.action.openSettingsJson", "Preferences", "Open User Settings (TOML)", |a, _| {
            let p = a.paths.user_settings();
            open_config_file(a, p, yace_config::DEFAULT_CONFIG_TEMPLATE)
        }),
        c!("workbench.action.openGlobalKeybindings", "Preferences", "Open Keyboard Shortcuts", |a, _| a
            .open_keybinding_editor()),
        c!("yace.restoreDefaultKeybindings", "Preferences", "Restore Default Keybindings", |a, _| a
            .restore_default_keybindings_confirm()),
        c!("yace.exportKeybindings", "Preferences", "Export Keybindings", |a, _| a.export_keybindings()),
        c!("workbench.action.openGlobalKeybindingsFile", "Preferences", "Open Keyboard Shortcuts (TOML)", |a, _| {
            let p = a.paths.keybindings();
            open_config_file(a, p, yace_config::DEFAULT_KEYBINDINGS_TEMPLATE)
        }),
        c!("workbench.action.toggleSidebarPosition", "View", "Primary Side Bar Position: Left/Right", |a, _| {
            let right = a.settings.workbench.side_bar_location == yace_config::SideBarLocation::Right;
            let v = if right { "left" } else { "right" };
            match yace_config::edit_setting(&a.paths.user_settings(), "workbench", "sideBarLocation", Some(v.into())) {
                Ok(()) => a.reload_settings(),
                Err(e) => a.notify(Severity::Error, format!("Could not save the setting: {e}")),
            }
        }),
        c!("workbench.action.evenEditorWidths", "View", "Reset Editor Group Sizes", |a, _| a.layout.even_sizes()),
        c!("yace.resetWorkbenchLayout", "View", "Reset Layout (Sidebar, Panel, Group Sizes)", |a, _| {
            a.sidebar_visible = true;
            a.sidebar_width = a.settings.workbench.sidebar_width;
            a.panel.height = crate::app::DEFAULT_PANEL_HEIGHT;
            a.panel.maximized = false;
            a.layout.even_sizes();
            a.notify(Severity::Info, "Layout reset (sidebar, panel and editor group sizes)");
        }),
        // --- Git (spec §15) ---------------------------------------------------------
        c!("git.refresh", "Git", "Refresh", |a, _| a.git_refresh()),
        c!("workbench.action.editor.nextChange", "Git", "Go to Next Change", |a, _| a.git_goto_change(true)),
        c!("workbench.action.editor.previousChange", "Git", "Go to Previous Change", |a, _| a.git_goto_change(false)),
        c!("yace.git.showChange", "Git", "Show Change Under Cursor", |a, _| a.git_show_change()),
        c!("workbench.view.scm", "View", "Show Source Control", |a, _| a.show_scm()),
        c!("git.openChange", "Git", "Open Changes", |a, _| a.git_open_changes_cmd()),
        c!("git.stage", "Git", "Stage Changes", |a, _| a.git_stage_active(true)),
        c!("git.unstage", "Git", "Unstage Changes", |a, _| a.git_stage_active(false)),
        c!("git.stageAll", "Git", "Stage All Changes", |a, _| a.scm_repo_cmd(crate::app::RepoCmd::StageAll)),
        c!("git.unstageAll", "Git", "Unstage All Changes", |a, _| a.scm_repo_cmd(crate::app::RepoCmd::UnstageAll)),
        c!("git.commit", "Git", "Commit", |a, _| a.scm_repo_cmd(crate::app::RepoCmd::Commit { all: false })),
        c!("git.commitAll", "Git", "Commit All", |a, _| a.scm_repo_cmd(crate::app::RepoCmd::Commit { all: true })),
        c!(args r#"{ "branch": string }"#, "git.checkout", "Git", "Checkout to…", |a, v| {
            #[derive(serde::Deserialize)]
            #[serde(untagged)]
            enum B {
                Name(String),
                Obj { branch: String },
            }
            match parse_args::<B>(v, r#"{ "branch": string } or "branch""#)? {
                None => {
                    a.scm_repo_cmd(crate::app::RepoCmd::Checkout);
                    Ok(())
                }
                Some(B::Name(b) | B::Obj { branch: b }) => a.git_switch_to(&b).map_err(CommandError::Unavailable),
            }
        }),
        c!("git.branch", "Git", "Create Branch…", |a, _| a.scm_repo_cmd(crate::app::RepoCmd::CreateBranch)),
        c!("git.showOutput", "Git", "Show Output", |a, _| a.git_show_output()),
        // --- Extensions (spec §18.14) -------------------------------------------
        c!("workbench.extensions.action.listInstalled", "Extensions", "List Installed", |a, _| a.plugins_list()),
        c!("yace.extensions.browse", "Extensions", "Browse Plugins…", |a, _| a.plugins_browse()),
        c!("yace.extensions.installFromDirectory", "Extensions", "Install from Directory…", |a, _| a
            .plugins_install_prompt(false)),
        c!("yace.extensions.installFromPackage", "Extensions", "Install from Package…", |a, _| a
            .plugins_install_prompt(true)),
        c!("yace.extensions.uninstall", "Extensions", "Uninstall…", |a, _| a.plugins_uninstall_picker()),
        c!("yace.extensions.enable", "Extensions", "Enable…", |a, _| a.plugins_enable_picker()),
        c!("yace.extensions.disable", "Extensions", "Disable…", |a, _| a.plugins_disable_picker()),
        c!("yace.extensions.reload", "Extensions", "Reload", |a, _| a.plugins_reload()),
        c!("yace.extensions.showPerformance", "Extensions", "Show Performance", |a, _| a.plugins_performance()),
        c!("yace.extensions.showViews", "Extensions", "Show Plugin Views…", |a, _| a.plugin_views_picker()),
        c!("workbench.action.openWorkspaceSettings", "Preferences", "Open Workspace Settings", open_workspace_settings),
        c!("workbench.action.openRawDefaultSettings", "Preferences", "Open Default Settings (Read-only)", |a, _| {
            let text = format!(
                "# YACE default settings — a generated, read-only reference.\n\
                 # Copy what you want to change into your user settings\n\
                 # (Preferences: Open User Settings) or workspace settings.\n\
                 # Every key is documented in docs/configuration.md.\n\n{}",
                toml::to_string(&yace_config::Settings::default()).unwrap_or_default()
            );
            let path = a.paths.cache_dir.join("default-settings.toml");
            match yace_config::atomic_write(&path, text.as_bytes()) {
                Ok(()) => a.open_file(&path, false),
                Err(e) => a.notify(Severity::Error, format!("Could not write {}: {e}", path.display())),
            }
        }),
        c!("yace.reloadConfiguration", "Preferences", "Reload Settings and Keybindings", |a, _| {
            a.invalidate_theme();
            a.reload_settings();
            a.reload_keymap();
            a.notify(Severity::Info, "Settings and keybindings reloaded");
        }),
        c!("workbench.action.quit", "File", "Quit", |a, _| a.quit_requested()),
        c!("yace.dismissNotification", "View", "Dismiss Notification", |a, _| a.dismiss_notification()),
        c!("yace.showNotifications", "View", "Show Notifications", notifications),
        c!("yace.notificationsInEditor", "View", "Open Notifications in Editor", notifications_in_editor),
        c!("yace.developer.showPerformanceMetrics", "Developer", "Show Performance Metrics", performance),
        c!("yace.developer.showConfigurationErrors", "Developer", "Show Configuration Errors", config_errors),
        c!("yace.developer.showKeybindingConflicts", "Developer", "Show Keybinding Conflicts", keybinding_conflicts),
        c!("yace.about", "Help", "About", about),
        // --- Files --------------------------------------------------------------
        c!("workbench.action.files.newUntitledFile", "File", "New Untitled Text File", |a, _| a.new_untitled()),
        c!(args r#""path" or { "path": string, "line"?: number, "column"?: number }"#, "workbench.action.files.openFile", "File", "Open File…", open_file_cmd),
        c!(hidden args r#""path" or { "path": string, "line"?: number, "column"?: number }"#, "vscode.open", "File", "Open", open_file_cmd),
        c!("workbench.action.files.save", "File", "Save", |a, _| {
            if let Some((_, _, d)) = a.active() {
                a.save_with_format(d);
            }
        }),
        c!("workbench.action.files.saveAs", "File", "Save As…", |a, _| {
            if let Some((_, _, d)) = a.active() {
                a.prompt_save_as(d);
            }
        }),
        c!("workbench.action.files.saveAll", "File", "Save All", |a, _| a.save_all()),
        c!("workbench.action.files.revert", "File", "Revert File", |a, _| a.revert_active()),
        c!("workbench.action.closeActiveEditor", "View", "Close Editor", |a, _| a.close_active_editor()),
        c!("workbench.action.closeOtherEditors", "View", "Close Other Editors in Group", |a, _| close_others(a, false)),
        c!("workbench.action.closeEditorsToTheRight", "View", "Close Editors to the Right in Group", |a, _| {
            close_others(a, true)
        }),
        c!("workbench.action.closeAllEditors", "View", "Close All Editors in Group", |a, _| close_all(a, false)),
        c!("workbench.action.closeEditorsInGroup", "View", "Close All Editors in Group", |a, _| close_all(a, false)),
        c!("workbench.action.closeUnmodifiedEditors", "View", "Close Saved Editors in Group", |a, _| close_all(
            a, true
        )),
        c!("workbench.action.reopenClosedEditor", "View", "Reopen Closed Editor", |a, _| a.reopen_closed()),
        c!("workbench.action.nextEditor", "View", "Open Next Editor", |a, _| cycle_editor(a, 1)),
        c!("workbench.action.previousEditor", "View", "Open Previous Editor", |a, _| cycle_editor(a, -1)),
        c!(
            "workbench.action.openNextRecentlyUsedEditorInGroup",
            "View",
            "Open Next Recently Used Editor in Group",
            |a, _| cycle_mru(a, true)
        ),
        c!(
            "workbench.action.openPreviousRecentlyUsedEditorInGroup",
            "View",
            "Open Previous Recently Used Editor in Group",
            |a, _| cycle_mru(a, false)
        ),
        c!("workbench.action.moveEditorLeftInGroup", "View", "Move Editor Left", |a, _| move_tab(a, -1)),
        c!("workbench.action.moveEditorRightInGroup", "View", "Move Editor Right", |a, _| move_tab(a, 1)),
        c!("workbench.action.keepEditor", "View", "Keep Editor (pin preview)", |a, _| {
            if let Some(v) = a.layout.active_group_mut().active_view_mut() {
                v.preview = false;
            }
        }),
        c!(hidden "workbench.action.openEditorAtIndex1", "View", "Open Editor 1", |a, _| open_at_index(a, 0)),
        c!(hidden "workbench.action.openEditorAtIndex2", "View", "Open Editor 2", |a, _| open_at_index(a, 1)),
        c!(hidden "workbench.action.openEditorAtIndex3", "View", "Open Editor 3", |a, _| open_at_index(a, 2)),
        c!(hidden "workbench.action.openEditorAtIndex4", "View", "Open Editor 4", |a, _| open_at_index(a, 3)),
        c!(hidden "workbench.action.openEditorAtIndex5", "View", "Open Editor 5", |a, _| open_at_index(a, 4)),
        c!(hidden "workbench.action.openEditorAtIndex6", "View", "Open Editor 6", |a, _| open_at_index(a, 5)),
        c!(hidden "workbench.action.openEditorAtIndex7", "View", "Open Editor 7", |a, _| open_at_index(a, 6)),
        c!(hidden "workbench.action.openEditorAtIndex8", "View", "Open Editor 8", |a, _| open_at_index(a, 7)),
        c!(hidden "workbench.action.lastEditorInGroup", "View", "Open Last Editor in Group", |a, _| {
            let n = a.layout.active_group().views.len();
            if n > 0 {
                open_at_index(a, n - 1)
            }
        }),
        // --- Editor groups ------------------------------------------------------
        c!("filesExplorer.copy", "Explorer", "Copy", |a, _| a.explorer_copy_files(false)),
        c!("filesExplorer.cut", "Explorer", "Cut", |a, _| a.explorer_copy_files(true)),
        c!("filesExplorer.paste", "Explorer", "Paste", |a, _| a.explorer_paste()),
        c!("explorer.openToSide", "Explorer", "Open to the Side", |a, _| a.explorer_open_beside()),
        c!("revealFileInOS", "File", "Reveal in File Manager", |a, _| a.reveal_in_os()),
        c!("workbench.action.splitEditor", "View", "Split Editor Right", |a, _| {
            a.layout.split(SplitAxis::Horizontal);
            a.focus_editor();
        }),
        c!("workbench.action.splitEditorDown", "View", "Split Editor Down", |a, _| {
            a.layout.split(SplitAxis::Vertical);
            a.focus_editor();
        }),
        c!(hidden "workbench.action.splitEditorOrthogonal", "View", "Split Editor Orthogonal", |a, _| {
            a.layout.split(SplitAxis::Vertical);
            a.focus_editor();
        }),
        c!("workbench.action.focusNextPart", "View", "Focus Next Part", |a, _| cycle_part(a, true)),
        c!("workbench.action.focusPreviousPart", "View", "Focus Previous Part", |a, _| cycle_part(a, false)),
        c!("workbench.action.focusActiveEditorGroup", "View", "Focus Active Editor Group", |a, _| a.focus_editor()),
        c!("workbench.action.focusFirstEditorGroup", "View", "Focus First Editor Group", |a, _| focus_group_n(a, 0)),
        c!("workbench.action.focusSecondEditorGroup", "View", "Focus Second Editor Group", |a, _| focus_group_n(a, 1)),
        c!("workbench.action.focusThirdEditorGroup", "View", "Focus Third Editor Group", |a, _| focus_group_n(a, 2)),
        c!("workbench.action.focusLeftGroup", "View", "Focus Left Editor Group", |a, _| focus_dir(a, false)),
        c!("workbench.action.focusRightGroup", "View", "Focus Right Editor Group", |a, _| focus_dir(a, true)),
        c!(hidden "workbench.action.focusAboveGroup", "View", "Focus Editor Group Above", |a, _| focus_dir(a, false)),
        c!(hidden "workbench.action.focusBelowGroup", "View", "Focus Editor Group Below", |a, _| focus_dir(a, true)),
        c!("workbench.action.moveEditorToNextGroup", "View", "Move Editor into Next Group", |a, _| {
            move_editor_to_group(a, true)
        }),
        c!("workbench.action.moveEditorToPreviousGroup", "View", "Move Editor into Previous Group", |a, _| {
            move_editor_to_group(a, false)
        }),
        c!("workbench.action.closeGroup", "View", "Close Editor Group", |a, _| {
            let g = a.layout.active;
            let docs = a.layout.active_group().views.iter().map(|v| v.doc).collect::<Vec<_>>();
            let losing = docs.into_iter().filter(|d| a.layout.views_of(*d).iter().all(|(gg, _)| *gg == g)).collect();
            a.start_close(losing, crate::actions::AfterClose::CloseGroup(g));
        }),
        c!("workbench.action.toggleMaximizeEditorGroup", "View", "Toggle Maximize Editor Group", |a, _| a
            .layout
            .maximized =
            !a.layout.maximized),
        c!("workbench.action.increaseViewSize", "View", "Increase Current View Size", |a, _| a
            .layout
            .resize_active(0.05)),
        c!("workbench.action.decreaseViewSize", "View", "Decrease Current View Size", |a, _| a
            .layout
            .resize_active(-0.05)),
        c!("yace.resetLayout", "View", "Reset Editor Layout", |a, _| {
            a.layout.reset();
            a.sidebar_visible = true;
            a.sidebar_width = a.settings.workbench.sidebar_width;
        }),
        // --- Workspace ---------------------------------------------------------
        c!("workbench.action.files.openFolder", "File", "Open Folder…", |a, _| {
            a.prompt(
                "Open Folder",
                "Folder to open (replaces the current workspace)",
                a.cwd_with_slash(),
                PromptAction::OpenFolder,
                true,
            )
        }),
        c!("workbench.action.openWorkspace", "File", "Open Workspace from File…", |a, _| {
            a.prompt(
                "Open Workspace",
                "Path of a .code-workspace file",
                a.cwd_with_slash(),
                PromptAction::OpenWorkspaceFile,
                true,
            )
        }),
        c!("workbench.action.openRecent", "File", "Open Recent…", |a, _| a.show_recent()),
        c!("workbench.action.addRootFolder", "Workspaces", "Add Folder to Workspace…", |a, _| {
            a.prompt(
                "Add Folder to Workspace",
                "Any folder — it does not need to share a parent with the others",
                a.cwd_with_slash(),
                PromptAction::AddRoot,
                true,
            )
        }),
        c!("workbench.action.removeRootFolder", "Workspaces", "Remove Folder from Workspace…", |a, _| a
            .with_root(RootAction::Remove)),
        c!("yace.workspace.renameRoot", "Workspaces", "Rename Workspace Folder…", |a, _| a
            .with_root(RootAction::Rename)),
        c!("yace.workspace.moveRootUp", "Workspaces", "Move Workspace Folder Up", |a, _| a
            .with_root(RootAction::MoveUp)),
        c!("yace.workspace.moveRootDown", "Workspaces", "Move Workspace Folder Down", |a, _| a
            .with_root(RootAction::MoveDown)),
        c!("yace.workspace.locateRoot", "Workspaces", "Locate Missing Workspace Folder…", |a, _| a
            .with_root(RootAction::Locate)),
        c!("workbench.action.saveWorkspaceAs", "Workspaces", "Save Workspace As…", |a, _| {
            let d = a.workspace_file_default();
            a.prompt("Save Workspace As", "Path of the .code-workspace file", d, PromptAction::SaveWorkspaceAs, true)
        }),
        c!("yace.workspace.save", "Workspaces", "Save Workspace", |a, _| {
            if a.workspace.file().is_some() && !matches!(a.workspace.kind, yace_workspace::WorkspaceKind::Untitled(_)) {
                match a.workspace.save() {
                    Ok(()) => a.notify(Severity::Info, "Workspace saved"),
                    Err(e) => a.notify(Severity::Error, e.to_string()),
                }
            } else {
                let d = a.workspace_file_default();
                a.prompt(
                    "Save Workspace As",
                    "Path of the .code-workspace file",
                    d,
                    PromptAction::SaveWorkspaceAs,
                    true,
                )
            }
        }),
        c!("workbench.action.closeFolder", "Workspaces", "Close Workspace", |a, _| a
            .switch_workspace(WorkspaceTarget::Empty)),
        c!("yace.workspace.new", "Workspaces", "New Empty Workspace", |a, _| a
            .switch_workspace(WorkspaceTarget::Empty)),
        // --- Explorer -----------------------------------------------------------
        c!("workbench.files.action.showActiveFileInExplorer", "File", "Reveal Active File in Explorer View", |a, _| a
            .reveal_active_file()),
        c!("workbench.files.action.refreshFilesExplorer", "File", "Refresh Explorer", |a, _| a.explorer.refresh_all()),
        c!("explorer.newFile", "File", "New File…", |a, _| a.explorer_new(false)),
        c!("explorer.newFolder", "File", "New Folder…", |a, _| a.explorer_new(true)),
        c!("renameFile", "File", "Rename…", |a, _| a.explorer_rename()),
        c!("yace.explorer.move", "File", "Move…", |a, _| a.explorer_move()),
        c!("deleteFile", "File", "Delete (Move to Trash)", |a, _| a.explorer_delete()),
        c!("copyFilePath", "File", "Copy Path of Active File", |a, _| a.copy_path(false)),
        c!("copyRelativeFilePath", "File", "Copy Relative Path of Active File", |a, _| a.copy_path(true)),
        c!("yace.explorer.toggleHidden", "View", "Toggle Hidden Files in Explorer", |a, _| {
            a.explorer.show_hidden = !a.explorer.show_hidden;
            a.explorer.rebuild_rows();
        }),
        c!(hidden "list.focusUp", "List", "Focus Up", |a, _| a.explorer.move_selection(-1)),
        c!(hidden "list.focusDown", "List", "Focus Down", |a, _| a.explorer.move_selection(1)),
        c!(hidden "list.focusFirst", "List", "Focus First", |a, _| a.explorer.selected = 0),
        c!(hidden "list.focusLast", "List", "Focus Last", |a, _| a.explorer.selected = a.explorer.rows.len().saturating_sub(1)),
        c!(hidden "list.focusPageUp", "List", "Page Up", |a, _| {
            let h = crate::ui::explorer_list_height(a) as isize;
            a.explorer.move_selection(-h.max(1));
        }),
        c!(hidden "list.focusPageDown", "List", "Page Down", |a, _| {
            let h = crate::ui::explorer_list_height(a) as isize;
            a.explorer.move_selection(h.max(1));
        }),
        c!(hidden "list.collapse", "List", "Collapse", |a, _| a.explorer_collapse()),
        c!(hidden "list.expand", "List", "Expand", |a, _| a.explorer_expand()),
        c!(hidden "list.select", "List", "Open", |a, _| {
            a.explorer.filter.clear();
            let key = a.explorer.selected_row().map(|r| r.path.clone());
            a.explorer.rebuild_rows();
            if let Some(p) = key {
                a.explorer.select_path(&p);
            }
            a.explorer_select(false)
        }),
        c!(hidden "list.toggleExpandOrPreview", "List", "Toggle / Preview", |a, _| a.explorer_select(true)),
        c!(hidden "list.clearFilter", "List", "Clear Filter", |a, _| {
            a.explorer.filter.clear();
            let key = a.explorer.selected_row().map(|r| r.path.clone());
            a.explorer.rebuild_rows();
            if let Some(p) = key {
                a.explorer.select_path(&p);
            }
        }),
        c!(hidden "list.filterBackspace", "List", "Filter Backspace", |a, _| {
            a.explorer.filter.pop();
            a.explorer.rebuild_rows();
        }),
        // --- Editing -------------------------------------------------------------
        c!(hidden "cursorLeft", "Editor", "Cursor Left", |a, _| a.cmd_move(Move::Left, false)),
        c!(hidden "cursorRight", "Editor", "Cursor Right", |a, _| a.cmd_move(Move::Right, false)),
        c!(hidden "cursorUp", "Editor", "Cursor Up", |a, _| a.cmd_move(Move::Up, false)),
        c!(hidden "cursorDown", "Editor", "Cursor Down", |a, _| a.cmd_move(Move::Down, false)),
        c!(hidden "cursorLeftSelect", "Editor", "Select Left", |a, _| a.cmd_move(Move::Left, true)),
        c!(hidden "cursorRightSelect", "Editor", "Select Right", |a, _| a.cmd_move(Move::Right, true)),
        c!(hidden "cursorUpSelect", "Editor", "Select Up", |a, _| a.cmd_move(Move::Up, true)),
        c!(hidden "cursorDownSelect", "Editor", "Select Down", |a, _| a.cmd_move(Move::Down, true)),
        c!(hidden "cursorWordLeft", "Editor", "Cursor Word Left", |a, _| a.cmd_move(Move::WordLeft, false)),
        c!(hidden "cursorWordRight", "Editor", "Cursor Word Right", |a, _| a.cmd_move(Move::WordRight, false)),
        c!(hidden "cursorWordLeftSelect", "Editor", "Select Word Left", |a, _| a.cmd_move(Move::WordLeft, true)),
        c!(hidden "cursorWordRightSelect", "Editor", "Select Word Right", |a, _| a.cmd_move(Move::WordRight, true)),
        c!(hidden "cursorHome", "Editor", "Cursor Home", |a, _| a.cmd_move(Move::Home, false)),
        c!(hidden "cursorEnd", "Editor", "Cursor End", |a, _| a.cmd_move(Move::End, false)),
        c!(hidden "cursorHomeSelect", "Editor", "Select to Home", |a, _| a.cmd_move(Move::Home, true)),
        c!(hidden "cursorEndSelect", "Editor", "Select to End", |a, _| a.cmd_move(Move::End, true)),
        c!(hidden "cursorTop", "Editor", "Cursor Top", |a, _| a.cmd_move(Move::Top, false)),
        c!(hidden "cursorBottom", "Editor", "Cursor Bottom", |a, _| a.cmd_move(Move::Bottom, false)),
        c!(hidden "cursorTopSelect", "Editor", "Select to Top", |a, _| a.cmd_move(Move::Top, true)),
        c!(hidden "cursorBottomSelect", "Editor", "Select to Bottom", |a, _| a.cmd_move(Move::Bottom, true)),
        c!(hidden "cursorPageUp", "Editor", "Cursor Page Up", |a, _| a.cmd_move(Move::PageUp, false)),
        c!(hidden "cursorPageDown", "Editor", "Cursor Page Down", |a, _| a.cmd_move(Move::PageDown, false)),
        c!(hidden "cursorPageUpSelect", "Editor", "Select Page Up", |a, _| a.cmd_move(Move::PageUp, true)),
        c!(hidden "cursorPageDownSelect", "Editor", "Select Page Down", |a, _| a.cmd_move(Move::PageDown, true)),
        c!(hidden "scrollLineUp", "Editor", "Scroll Line Up", |a, _| a.scroll_lines(-1)),
        c!(hidden "scrollLineDown", "Editor", "Scroll Line Down", |a, _| a.scroll_lines(1)),
        c!(hidden "scrollPageUp", "Editor", "Scroll Page Up", |a, _| {
            let h = a.active_view_height() as isize;
            a.scroll_lines(-h)
        }),
        c!(hidden "scrollPageDown", "Editor", "Scroll Page Down", |a, _| {
            let h = a.active_view_height() as isize;
            a.scroll_lines(h)
        }),
        c!(hidden "deleteLeft", "Editor", "Delete Left", |a, _| a.run_op(Op::Backspace)),
        c!(hidden "deleteRight", "Editor", "Delete Right", |a, _| a.run_op(Op::Delete)),
        c!(hidden "deleteWordLeft", "Editor", "Delete Word Left", |a, _| a.run_op(Op::DeleteWordLeft)),
        c!(hidden "deleteWordRight", "Editor", "Delete Word Right", |a, _| a.run_op(Op::DeleteWordRight)),
        c!(hidden "yace.editor.newline", "Editor", "Insert Newline", |a, _| a.run_op(Op::Newline)),
        c!(hidden "tab", "Editor", "Tab", |a, _| a.run_op(Op::Tab)),
        c!(hidden "outdent", "Editor", "Outdent", |a, _| a.run_op(Op::Outdent)),
        c!("editor.action.indentLines", "Editor", "Indent Line", |a, _| a.run_op(Op::IndentLines)),
        c!("editor.action.outdentLines", "Editor", "Outdent Line", |a, _| a.run_op(Op::Outdent)),
        c!("editor.action.insertLineAfter", "Editor", "Insert Line Below", |a, _| a.run_op(Op::LineBelow)),
        c!("editor.action.insertLineBefore", "Editor", "Insert Line Above", |a, _| a.run_op(Op::LineAbove)),
        c!("editor.action.selectAll", "Selection", "Select All", |a, _| a
            .set_selection_with(|r, _| yace_buffer::ops::select_all(r))),
        c!("expandLineSelection", "Selection", "Expand Line Selection", |a, _| a
            .set_selection_with(yace_buffer::ops::select_line)),
        c!("undo", "Edit", "Undo", |a, _| a.undo(false)),
        c!("redo", "Edit", "Redo", |a, _| a.undo(true)),
        c!("editor.action.clipboardCopyAction", "Edit", "Copy", |a, _| {
            if ed(a) {
                a.copy(false)
            }
        }),
        c!("editor.action.clipboardCutAction", "Edit", "Cut", |a, _| {
            if ed(a) {
                a.copy(true)
            }
        }),
        c!("editor.action.clipboardPasteAction", "Edit", "Paste", |a, _| a.paste()),
        c!("editor.action.addSelectionToNextFindMatch", "Selection", "Add Selection to Next Find Match", |a, _| {
            a.set_selection_with(yace_buffer::ops::add_next_match)
        }),
        c!("editor.action.selectHighlights", "Selection", "Select All Occurrences of Find Match", |a, _| {
            a.set_selection_with(yace_buffer::ops::select_all_matches)
        }),
        c!("editor.action.insertCursorAbove", "Selection", "Add Cursor Above", |a, _| a
            .add_cursor(Direction::Backward)),
        c!("editor.action.insertCursorBelow", "Selection", "Add Cursor Below", |a, _| a.add_cursor(Direction::Forward)),
        c!(hidden "removeSecondaryCursors", "Selection", "Remove Secondary Cursors", |a, _| a.collapse_cursors(true)),
        c!(hidden "cancelSelection", "Selection", "Cancel Selection", |a, _| a.collapse_cursors(false)),
        c!("editor.action.moveLinesUpAction", "Selection", "Move Line Up", |a, _| a.run_op(Op::MoveLinesUp)),
        c!("editor.action.moveLinesDownAction", "Selection", "Move Line Down", |a, _| a.run_op(Op::MoveLinesDown)),
        c!("editor.action.copyLinesUpAction", "Selection", "Copy Line Up", |a, _| a.run_op(Op::CopyLinesUp)),
        c!("editor.action.copyLinesDownAction", "Selection", "Copy Line Down", |a, _| a.run_op(Op::CopyLinesDown)),
        c!("editor.action.deleteLines", "Editor", "Delete Line", |a, _| a.run_op(Op::DeleteLines)),
        c!("editor.action.commentLine", "Editor", "Toggle Line Comment", |a, _| a.run_op(Op::ToggleComment)),
        c!("editor.action.blockComment", "Editor", "Toggle Block Comment", |a, _| a.run_op(Op::ToggleBlockComment)),
        c!("editor.fold", "Editor", "Fold", |a, _| a.fold_at_cursor()),
        c!("editor.unfold", "Editor", "Unfold", |a, _| a.unfold_at_cursor()),
        c!("editor.toggleFold", "Editor", "Toggle Fold", |a, _| a.toggle_fold_at_cursor()),
        c!("editor.foldAll", "Editor", "Fold All", |a, _| a.fold_all()),
        c!("editor.foldRecursively", "Editor", "Fold Recursively", |a, _| a.fold_recursively()),
        c!("editor.unfoldRecursively", "Editor", "Unfold Recursively", |a, _| a.unfold_recursively()),
        c!("editor.foldAllBlockComments", "Editor", "Fold All Block Comments", |a, _| a.fold_all_block_comments()),
        c!("editor.foldLevel1", "Editor", "Fold Level 1", |a, _| a.fold_level(1)),
        c!("editor.foldLevel2", "Editor", "Fold Level 2", |a, _| a.fold_level(2)),
        c!("editor.foldLevel3", "Editor", "Fold Level 3", |a, _| a.fold_level(3)),
        c!("editor.foldLevel4", "Editor", "Fold Level 4", |a, _| a.fold_level(4)),
        c!("editor.foldLevel5", "Editor", "Fold Level 5", |a, _| a.fold_level(5)),
        c!("editor.foldLevel6", "Editor", "Fold Level 6", |a, _| a.fold_level(6)),
        c!("editor.foldLevel7", "Editor", "Fold Level 7", |a, _| a.fold_level(7)),
        c!("editor.unfoldAll", "Editor", "Unfold All", |a, _| a.unfold_all()),
        c!("editor.action.toggleWordWrap", "View", "Toggle Word Wrap", |a, _| a.toggle_word_wrap()),
        c!("editor.action.jumpToBracket", "Go", "Go to Bracket", |a, _| a.jump_to_bracket()),
        c!("editor.action.trimTrailingWhitespace", "Editor", "Trim Trailing Whitespace", |a, _| a
            .run_op(Op::TrimWhitespace)),
        c!(args r#"{ "line": number, "column"?: number }, a number or "42:8""#, "workbench.action.gotoLine", "Go", "Go to Line/Column…", |a, v| {
            let shape = r#"{ "line": number, "column"?: number }, a number or "42:8""#;
            let arg = parse_args::<LineArg>(v, shape)?;
            need_editor(a)?;
            match arg {
                None => {
                    a.prompt("Go to Line", "Line number, optionally :column", String::new(), PromptAction::GotoLine, false);
                    Ok(())
                }
                Some(LineArg::Text(t)) => a.goto_line(&t).map_err(CommandError::InvalidArgs),
                Some(LineArg::Number(line)) | Some(LineArg::Pos { line, column: None }) => {
                    a.goto_line(&line.to_string()).map_err(CommandError::InvalidArgs)
                }
                Some(LineArg::Pos { line, column: Some(c) }) => {
                    a.goto_line(&format!("{line}:{c}")).map_err(CommandError::InvalidArgs)
                }
            }
        }),
        c!("yace.editor.changeEol", "Editor", "Change End of Line Sequence", |a, _| {
            if ed(a) {
                let items = vec![
                    PickerItem {
                        label: "LF".into(),
                        detail: "Unix / macOS (\\n)".into(),
                        hint: String::new(),
                        data: 0,
                    },
                    PickerItem {
                        label: "CRLF".into(),
                        detail: "Windows (\\r\\n)".into(),
                        hint: String::new(),
                        data: 1,
                    },
                ];
                a.open_overlay(Overlay::Picker(Picker::new(
                    "Select End of Line Sequence",
                    items,
                    PickerAction::ChangeEol,
                )));
            }
        }),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_default_binding_targets_a_registered_command() {
        let reg = Registry::builtin();
        let km = yace_keymap::default_keymap();
        let missing: Vec<&str> =
            km.bindings().iter().map(|b| b.command.as_str()).filter(|c| reg.get(c).is_none()).collect();
        assert!(missing.is_empty(), "unregistered commands: {missing:?}");
    }

    #[test]
    fn ids_are_unique() {
        let all = builtin_commands();
        let mut ids: Vec<&str> = all.iter().map(|c| c.id).collect();
        ids.sort();
        let before = ids.len();
        ids.dedup();
        assert_eq!(before, ids.len());
    }
}
