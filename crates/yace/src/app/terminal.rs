//! The integrated terminal panel (spec §11): sessions per workspace root,
//! focus-based input routing, lifecycle and the bottom panel.

use std::path::{Path, PathBuf};

use crossbeam_channel::{Receiver, Sender, unbounded};
use yace_config::{ConfirmKill, TerminalCwd};
use yace_keymap::{KeyCode, KeyPress, Mods};
use yace_search::FindOptions;
use yace_terminal::{
    Notice, Screen, Session, SessionConfig, SessionId, TermMatch, TermQuery, default_shell, encode_key, encode_paste,
};
use yace_workspace::{RootId, RootStatus};

use super::{App, Focus, Severity};
use crate::{
    actions::{AfterClose, ConfirmAction, PickerAction, PromptAction},
    overlay::{Confirm, LineInput, Overlay, Picker, PickerItem},
    workers::{TERM_FIND_COUNT_CAP, TermFindJob},
};

/// Find in the terminal's scrollback (`Ctrl+F` in a terminal). The search
/// runs on the terminal find worker; drawing highlights the visible
/// matches with `highlight`.
#[derive(Debug, Default)]
pub struct TermFindState {
    pub visible: bool,
    pub query: LineInput,
    pub opts: FindOptions,
    /// The match shown (selected) and the terminal it is in.
    pub current: Option<TermMatch>,
    pub session: Option<SessionId>,
    /// Matches (at most 1000) and the current one's number.
    pub count: Option<(usize, Option<usize>)>,
    pub searching: bool,
    pub error: Option<String>,
    seq: u64,
    /// The compiled query, for highlighting while drawing.
    pub highlight: std::cell::RefCell<Option<TermQuery>>,
}

impl TermFindState {
    /// "3 of 41", "1000+", "No results"…
    pub fn status(&self) -> String {
        if let Some(e) = &self.error {
            return e.clone();
        }
        if self.query.text.is_empty() {
            return String::new();
        }
        match self.count {
            None if self.searching => "…".into(),
            None | Some((0, _)) => "No results".into(),
            Some((n, at)) => {
                let total = if n >= TERM_FIND_COUNT_CAP { format!("{n}+") } else { n.to_string() };
                match at {
                    Some(i) => format!("{i} of {total}"),
                    None => format!("? of {total}"),
                }
            }
        }
    }
}

/// Commands that keep working while a terminal has focus; every other key
/// goes to the shell (VS Code's `commandsToSkipShell`, spec §11.6).
pub const DEFAULT_SKIP_SHELL: &[&str] = &[
    "workbench.action.quickOpen",
    "workbench.action.showCommands",
    "workbench.action.terminal.toggleTerminal",
    "workbench.action.togglePanel",
    "workbench.action.terminal.new",
    "workbench.action.terminal.focusNext",
    "workbench.action.terminal.focusPrevious",
    "workbench.action.terminal.copySelection",
    "workbench.action.terminal.paste",
    "workbench.action.terminal.scrollUp",
    "workbench.action.terminal.scrollDown",
    "workbench.action.terminal.scrollUpPage",
    "workbench.action.terminal.scrollDownPage",
    "workbench.action.toggleMaximizedPanel",
    "workbench.view.explorer",
    "workbench.action.findInFiles",
    "workbench.action.focusFirstEditorGroup",
    "workbench.action.focusActiveEditorGroup",
    "workbench.action.focusSecondEditorGroup",
    "workbench.action.focusThirdEditorGroup",
    "workbench.action.focusNextPart",
    "workbench.action.focusPreviousPart",
    "workbench.action.terminal.focusFind",
    "workbench.action.terminal.split",
    "workbench.action.terminal.focusNextPane",
    "workbench.action.terminal.focusPreviousPane",
    "workbench.action.terminal.resizePaneLeft",
    "workbench.action.terminal.resizePaneRight",
    "workbench.action.terminal.findNext",
    "workbench.action.terminal.findPrevious",
];

#[derive(Debug)]
pub struct TermTab {
    pub id: SessionId,
    pub session: Option<Session>,
    /// Display name ("zsh", "claude", …).
    pub name: String,
    /// Title set by the shell (OSC 0/2), if any.
    pub title: Option<String>,
    pub root: Option<RootId>,
    pub cwd: PathBuf,
    pub profile: Option<String>,
    /// Exit status once the shell ended.
    pub exited: Option<Option<i32>>,
    /// Restored from a previous session: starts when first shown.
    pub not_started: bool,
    /// Typed into the shell once it starts (plugin tasks).
    pub startup: Option<String>,
    /// Terminals with the same group are shown side by side (one tab).
    pub group: u32,
    /// Share of the group's width.
    pub weight: f32,
    /// PTY size last applied (rows, cols).
    pub applied_size: Option<(u16, u16)>,
}

impl TermTab {
    pub fn is_running(&self) -> bool {
        self.session.as_ref().is_some_and(Session::is_running)
    }
    pub fn has_running_job(&self) -> bool {
        self.session.as_ref().is_some_and(Session::has_running_job)
    }
}

#[derive(Debug)]
pub struct PanelState {
    pub visible: bool,
    /// Preferred height in rows (clamped by the layout).
    pub height: u16,
    pub maximized: bool,
    pub tabs: Vec<TermTab>,
    pub active: usize,
    next_id: SessionId,
    next_group: u32,
    notice_tx: Sender<Notice>,
    pub notice_rx: Receiver<Notice>,
}

/// Bottom panel height (rows of terminal content) for a new window and
/// after "View: Reset Layout".
pub const DEFAULT_PANEL_HEIGHT: u16 = 12;

impl Default for PanelState {
    fn default() -> Self {
        let (notice_tx, notice_rx) = unbounded();
        Self {
            visible: false,
            height: DEFAULT_PANEL_HEIGHT,
            maximized: false,
            tabs: Vec::new(),
            active: 0,
            next_id: 1,
            next_group: 1,
            notice_tx,
            notice_rx,
        }
    }
}

impl PanelState {
    pub fn active_tab(&self) -> Option<&TermTab> {
        self.tabs.get(self.active)
    }
    pub fn active_tab_mut(&mut self) -> Option<&mut TermTab> {
        self.tabs.get_mut(self.active)
    }
    pub fn running_jobs(&self) -> usize {
        self.tabs.iter().filter(|t| t.has_running_job()).count()
    }
    fn tab_index(&self, id: SessionId) -> Option<usize> {
        self.tabs.iter().position(|t| t.id == id)
    }
    /// Indices of the terminals shown with `idx` (its group), in order.
    pub fn panes_of(&self, idx: usize) -> Vec<usize> {
        let Some(g) = self.tabs.get(idx).map(|t| t.group) else { return vec![] };
        (0..self.tabs.len()).filter(|i| self.tabs[*i].group == g).collect()
    }
    /// The terminals shown now (the active terminal's group).
    pub fn visible_panes(&self) -> Vec<usize> {
        self.panes_of(self.active)
    }
    /// Groups in tab-strip order, each with its terminals.
    pub fn groups(&self) -> Vec<Vec<usize>> {
        let mut out: Vec<(u32, Vec<usize>)> = Vec::new();
        for (i, t) in self.tabs.iter().enumerate() {
            match out.iter_mut().find(|(g, _)| *g == t.group) {
                Some((_, v)) => v.push(i),
                None => out.push((t.group, vec![i])),
            }
        }
        out.into_iter().map(|(_, v)| v).collect()
    }
    fn new_group(&mut self) -> u32 {
        let g = self.next_group;
        self.next_group += 1;
        g
    }
}

/// Narrowest terminal pane, in columns.
pub const MIN_PANE_COLS: u16 = 10;

/// Snapshot of the active terminal for drawing.
pub fn snapshot(tab: &TermTab) -> Option<Screen> {
    let s = tab.session.as_ref()?;
    let mut screen = Screen::default();
    s.screen(&mut screen);
    s.mark_seen();
    Some(screen)
}

impl App {
    /// A clone of the notice receiver (the runtime forwards notices).
    pub fn terminal_notices(&self) -> Receiver<Notice> {
        self.panel.notice_rx.clone()
    }

    /// Process pending terminal notices without blocking (tests).
    pub fn drain_terminal_notices(&mut self) {
        let pending: Vec<Notice> = self.panel.notice_rx.try_iter().collect();
        for n in pending {
            self.on_terminal_notice(n);
        }
    }

    pub(crate) fn on_terminal_notice(&mut self, n: Notice) {
        match n {
            Notice::Output(_) => {}
            Notice::Title(id, t) => {
                if let Some(i) = self.panel.tab_index(id) {
                    self.panel.tabs[i].title = Some(t).filter(|t| !t.trim().is_empty());
                }
            }
            Notice::Exit(id, code) => {
                if let Some(i) = self.panel.tab_index(id) {
                    self.panel.tabs[i].exited = Some(code);
                }
            }
            Notice::Bell(_) => {}
        }
        self.needs_render = true;
    }

    /// Root and directory for a new terminal, per `terminal.cwd`.
    pub(crate) fn terminal_target(&self) -> (Option<RootId>, PathBuf) {
        let active_file = self.active().and_then(|(_, _, d)| self.docs.doc(d)?.path().map(Path::to_path_buf));
        let first = self.workspace.roots().iter().find(|r| r.status == RootStatus::Available);
        let active_root = active_file.as_deref().and_then(|p| self.workspace.root_for_path(p));
        let home = || std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("/"));
        match self.settings.terminal.cwd {
            TerminalCwd::ActiveFileDir => {
                if let Some(dir) = active_file.as_deref().and_then(Path::parent) {
                    return (active_root.map(|r| r.id), dir.to_path_buf());
                }
            }
            TerminalCwd::FirstRoot => {
                if let Some(r) = first {
                    return (Some(r.id), r.path.clone());
                }
            }
            TerminalCwd::ActiveRoot => {}
        }
        match active_root.or(first) {
            Some(r) => (Some(r.id), r.path.clone()),
            None => (None, std::env::current_dir().unwrap_or_else(|_| home())),
        }
    }

    fn session_config(&self, cwd: &Path, profile: Option<&str>, size: Option<(u16, u16)>) -> (SessionConfig, String) {
        let t = &self.settings.terminal;
        let prof = profile
            .or(Some(t.default_profile.as_str()).filter(|p| !p.is_empty()))
            .and_then(|name| t.profiles.iter().find(|p| p.name == name));
        let shell = prof
            .map(|p| p.shell.clone())
            .filter(|s| !s.is_empty())
            .or_else(|| Some(t.shell.clone()).filter(|s| !s.is_empty()))
            .unwrap_or_else(default_shell);
        let args = match prof {
            Some(p) if !p.shell.is_empty() || !p.args.is_empty() => p.args.clone(),
            _ => t.args.clone(),
        };
        let mut env: Vec<(String, String)> = t.env.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
        if let Some(p) = prof {
            env.extend(p.env.iter().map(|(k, v)| (k.clone(), v.clone())));
        }
        let (rows, cols) = size.unwrap_or((10, 80));
        let name = prof.map(|p| p.name.clone()).unwrap_or_else(|| {
            Path::new(&shell).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "shell".into())
        });
        let cfg = SessionConfig {
            shell,
            args,
            env,
            cwd: cwd.to_path_buf(),
            rows,
            cols,
            scrollback: t.scrollback.clamp(100, 1_000_000),
            startup_command: prof.map(|p| p.startup_command.clone()).filter(|c| !c.is_empty()),
        };
        (cfg, name)
    }

    fn spawn_into(&mut self, idx: usize) {
        let (cwd, profile, id, size) = {
            let t = &self.panel.tabs[idx];
            (t.cwd.clone(), t.profile.clone(), t.id, t.applied_size)
        };
        let cwd = if cwd.is_dir() { cwd } else { std::env::current_dir().unwrap_or_else(|_| PathBuf::from("/")) };
        let (cfg, name) = self.session_config(&cwd, profile.as_deref(), size);
        match Session::spawn(&cfg, id, self.panel.notice_tx.clone()) {
            Ok(s) => {
                let t = &mut self.panel.tabs[idx];
                t.session = Some(s);
                t.exited = None;
                t.title = None;
                if t.not_started {
                    t.name = format!("{name} (new session)");
                    t.not_started = false;
                } else {
                    t.name = name;
                }
            }
            Err(e) => {
                self.notify(Severity::Error, format!("Could not start {}: {e}", cfg.shell));
                self.panel.tabs[idx].exited = Some(None);
            }
        }
        self.panel.tabs[idx].applied_size = None; // re-apply the pane size
    }

    /// Open a new terminal in `cwd` (root-aware) and focus it.
    pub(crate) fn new_terminal(&mut self, root: Option<RootId>, cwd: PathBuf, profile: Option<String>) {
        let id = self.panel.next_id;
        self.panel.next_id += 1;
        let group = self.panel.new_group();
        self.panel.tabs.push(TermTab {
            id,
            session: None,
            name: String::new(),
            title: None,
            root,
            cwd,
            profile,
            exited: None,
            not_started: false,
            startup: None,
            group,
            weight: 1.0,
            applied_size: None,
        });
        self.panel.active = self.panel.tabs.len() - 1;
        self.panel.visible = true;
        let idx = self.panel.active;
        self.sync_terminal_size();
        self.spawn_into(idx);
        self.sync_terminal_size();
        self.focus_changed(Focus::Terminal);
    }

    /// A terminal opened for a plugin: a name, and a command typed into the
    /// user's shell once it starts (the shell stays usable afterwards).
    pub(crate) fn new_terminal_with(
        &mut self,
        root: Option<RootId>,
        cwd: PathBuf,
        name: Option<String>,
        startup: Option<String>,
    ) -> SessionId {
        self.new_terminal(root, cwd, None);
        let idx = self.panel.tabs.len() - 1;
        let t = &mut self.panel.tabs[idx];
        if let Some(n) = name {
            t.name = n;
        }
        if let (Some(cmd), Some(sess)) = (startup.as_ref(), t.session.as_mut()) {
            sess.write(format!("{cmd}\r").as_bytes());
        }
        t.startup = startup;
        t.id
    }

    pub(crate) fn new_terminal_default(&mut self) {
        let (root, cwd) = self.terminal_target();
        self.new_terminal(root, cwd, None);
    }

    /// `` Ctrl+` ``: show and focus the terminal, or hide it.
    pub(crate) fn toggle_terminal(&mut self) {
        if self.panel.visible && matches!(self.focus, Focus::Terminal | Focus::TerminalFind) {
            self.panel.visible = false;
            self.focus_changed(Focus::Editor);
            return;
        }
        self.show_terminal();
    }

    /// `Ctrl+J`: show or hide the panel.
    pub(crate) fn toggle_panel(&mut self) {
        if self.panel.visible {
            self.panel.visible = false;
            if matches!(self.focus, Focus::Terminal | Focus::TerminalFind) {
                self.focus_changed(Focus::Editor);
            }
        } else {
            self.show_terminal();
        }
    }

    pub(crate) fn show_terminal(&mut self) {
        self.panel.visible = true;
        if self.panel.tabs.is_empty() {
            self.new_terminal_default();
        } else {
            self.ensure_active_started();
            self.focus_changed(Focus::Terminal);
        }
    }

    /// Start a restored ("not started") terminal when it is shown.
    pub(crate) fn ensure_active_started(&mut self) {
        if !self.panel.visible {
            return;
        }
        for idx in self.panel.visible_panes() {
            if self.panel.tabs.get(idx).is_some_and(|t| t.not_started && t.session.is_none()) {
                self.sync_terminal_size();
                self.spawn_into(idx);
            }
        }
    }

    pub(crate) fn new_terminal_in_root_picker(&mut self) {
        let items = self
            .workspace
            .roots()
            .iter()
            .filter(|r| r.status == RootStatus::Available)
            .map(|r| PickerItem::new(r.display_name(), r.path.display().to_string(), "", r.id.0 as usize))
            .collect::<Vec<_>>();
        if items.is_empty() {
            self.new_terminal_default();
            return;
        }
        let mut p = Picker::new("Open Terminal in Workspace Folder", items, PickerAction::NewTerminalInRoot);
        p.empty_text = "No workspace folders".into();
        self.open_overlay(Overlay::Picker(p));
    }

    pub(crate) fn new_terminal_in_root(&mut self, id: RootId) {
        if let Some(r) = self.workspace.root(id) {
            let path = r.path.clone();
            self.new_terminal(Some(id), path, None);
        }
    }

    pub(crate) fn new_terminal_in_active_file_dir(&mut self) {
        let Some(path) = self.active().and_then(|(_, _, d)| self.docs.doc(d)?.path().map(Path::to_path_buf)) else {
            self.new_terminal_default();
            return;
        };
        let dir = path.parent().map(Path::to_path_buf).unwrap_or_default();
        let root = self.workspace.root_for_path(&path).map(|r| r.id);
        self.new_terminal(root, dir, None);
    }

    /// Explorer: "Open in Integrated Terminal".
    pub(crate) fn new_terminal_in_explorer_dir(&mut self) {
        let Some((root, dir)) = self.explorer.target_dir() else {
            self.new_terminal_default();
            return;
        };
        self.new_terminal(Some(root), dir, None);
    }

    pub(crate) fn new_terminal_with_profile_picker(&mut self) {
        let mut items: Vec<PickerItem> = self
            .settings
            .terminal
            .profiles
            .iter()
            .enumerate()
            .map(|(i, p)| {
                let what = if p.startup_command.is_empty() {
                    p.shell.clone()
                } else {
                    format!("runs `{}`", p.startup_command)
                };
                PickerItem::new(p.name.clone(), what, "", i + 1)
            })
            .collect();
        items.insert(0, PickerItem::new(format!("Default shell ({})", default_shell()), "", "", 0));
        self.open_overlay(Overlay::Picker(Picker::new(
            "New Terminal with Profile",
            items,
            PickerAction::NewTerminalProfile,
        )));
    }

    pub(crate) fn new_terminal_with_profile(&mut self, index: usize) {
        let profile = index.checked_sub(1).and_then(|i| self.settings.terminal.profiles.get(i)).map(|p| p.name.clone());
        let (root, cwd) = self.terminal_target();
        self.new_terminal(root, cwd, profile);
    }

    /// Kill the active terminal, asking first when configured.
    pub(crate) fn kill_terminal_requested(&mut self) {
        let Some(tab) = self.panel.active_tab() else { return };
        let ask = match self.settings.terminal.confirm_kill {
            ConfirmKill::Always => tab.is_running(),
            ConfirmKill::WhenRunning => tab.has_running_job(),
            ConfirmKill::Never => false,
        };
        let id = tab.id;
        if ask {
            let name = tab.name.clone();
            self.open_overlay(Overlay::Confirm(Confirm {
                title: "Kill terminal".into(),
                message: vec![
                    format!("A process is still running in '{name}'."),
                    "Killing the terminal stops it.".into(),
                ],
                buttons: vec![
                    ("Kill".into(), ConfirmAction::KillTerminal(id)),
                    ("Cancel".into(), ConfirmAction::Nothing),
                ],
                selected: 1,
                cancel: ConfirmAction::Nothing,
            }));
        } else {
            self.kill_terminal(id);
        }
    }

    pub(crate) fn kill_terminal(&mut self, id: SessionId) {
        let Some(i) = self.panel.tab_index(id) else { return };
        // Stay in the same group when it still has a terminal.
        let sibling = self.panel.panes_of(i).into_iter().filter(|p| *p != i).min_by_key(|p| p.abs_diff(i));
        let mut tab = self.panel.tabs.remove(i);
        if let Some(s) = tab.session.as_mut() {
            s.kill();
        }
        if self.panel.tabs.is_empty() {
            self.panel.visible = false;
            self.panel.active = 0;
            self.terminal_find_close();
            if matches!(self.focus, Focus::Terminal | Focus::TerminalFind) {
                self.focus_changed(Focus::Editor);
            }
        } else {
            self.panel.active = match sibling {
                Some(p) if i == self.panel.active || p == self.panel.active => {
                    if p > i {
                        p - 1
                    } else {
                        p
                    }
                }
                _ if self.panel.active > i => self.panel.active - 1,
                _ => self.panel.active.min(self.panel.tabs.len() - 1),
            };
            self.ensure_active_started();
        }
        self.request_trim();
    }

    pub fn kill_all_terminals(&mut self) {
        for t in &mut self.panel.tabs {
            if let Some(s) = t.session.as_mut() {
                s.kill();
            }
        }
        self.panel.tabs.clear();
        self.panel.visible = false;
        self.panel.active = 0;
    }

    /// Before quitting or switching workspace: ask when commands are still
    /// running in terminals (spec §11.8). Returns true if it asked.
    pub(crate) fn confirm_running_terminals(&mut self, then: AfterClose) -> bool {
        let n = self.panel.running_jobs();
        if n == 0 {
            return false;
        }
        self.open_overlay(Overlay::Confirm(Confirm {
            title: "Running processes".into(),
            message: vec![
                format!("{n} terminal{} still running a command.", if n == 1 { " is" } else { "s are" }),
                "Continuing will stop them.".into(),
            ],
            buttons: vec![
                ("Kill and Continue".into(), ConfirmAction::KillTerminalsThen(Box::new(then))),
                ("Cancel".into(), ConfirmAction::Nothing),
            ],
            selected: 1,
            cancel: ConfirmAction::Nothing,
        }));
        true
    }

    pub(crate) fn rename_terminal_prompt(&mut self) {
        let Some(tab) = self.panel.active_tab() else { return };
        let name = tab.name.clone();
        let id = tab.id;
        self.prompt("Rename Terminal", "New name", name, PromptAction::RenameTerminal(id), false);
    }

    pub(crate) fn rename_terminal(&mut self, id: SessionId, name: &str) {
        if let Some(i) = self.panel.tab_index(id) {
            self.panel.tabs[i].name = name.to_string();
        }
    }

    /// `Ctrl+PageDown` / `Ctrl+PageUp`: the next or previous tab (group).
    pub(crate) fn focus_terminal_tab(&mut self, delta: isize) {
        let groups = self.panel.groups();
        let n = groups.len();
        if n == 0 {
            return;
        }
        let cur = groups.iter().position(|g| g.contains(&self.panel.active)).unwrap_or(0);
        let next = (cur as isize + delta).rem_euclid(n as isize) as usize;
        self.panel.active = groups[next][0];
        self.ensure_active_started();
        self.focus_changed(Focus::Terminal);
    }

    // --- split panes -----------------------------------------------------------

    /// Split the active terminal: a new shell beside it, in the same
    /// folder (the shell's current one when known).
    pub(crate) fn split_terminal(&mut self) {
        let Some(tab) = self.panel.active_tab() else {
            self.new_terminal_default();
            return;
        };
        let content_w = crate::ui::workbench_layout(self).panel_content.map_or(0, |r| r.width);
        let n = self.panel.visible_panes().len() as u16 + 1;
        if content_w < n * MIN_PANE_COLS + (n - 1) {
            self.notify(Severity::Info, "Not enough room to split the terminal");
            return;
        }
        let cwd = tab.session.as_ref().and_then(Session::current_dir).unwrap_or_else(|| tab.cwd.clone());
        let (root, profile, group) = (tab.root, tab.profile.clone(), tab.group);
        let id = self.panel.next_id;
        self.panel.next_id += 1;
        let at = self.panel.active + 1;
        self.panel.tabs.insert(
            at,
            TermTab {
                id,
                session: None,
                name: String::new(),
                title: None,
                root,
                cwd,
                profile,
                exited: None,
                not_started: false,
                startup: None,
                group,
                weight: 1.0,
                applied_size: None,
            },
        );
        // Split evenly, as VS Code does.
        for i in self.panel.panes_of(at) {
            self.panel.tabs[i].weight = 1.0;
        }
        self.panel.active = at;
        self.panel.visible = true;
        self.sync_terminal_size();
        self.spawn_into(at);
        self.sync_terminal_size();
        self.focus_changed(Focus::Terminal);
    }

    /// Focus the next / previous pane of the active tab.
    pub(crate) fn focus_terminal_pane(&mut self, delta: isize) {
        let panes = self.panel.visible_panes();
        if panes.len() < 2 {
            return;
        }
        let cur = panes.iter().position(|p| *p == self.panel.active).unwrap_or(0);
        self.panel.active = panes[(cur as isize + delta).rem_euclid(panes.len() as isize) as usize];
        self.focus_changed(Focus::Terminal);
    }

    /// Move the active pane's divider left or right by about 5 columns
    /// (its right divider; the last pane moves its left one).
    pub(crate) fn resize_terminal_pane(&mut self, right: bool) {
        let panes = self.panel.visible_panes();
        if panes.len() < 2 {
            return;
        }
        let cur = panes.iter().position(|p| *p == self.panel.active).unwrap_or(0);
        let (a, b) = if cur + 1 < panes.len() { (panes[cur], panes[cur + 1]) } else { (panes[cur - 1], panes[cur]) };
        let width = crate::ui::workbench_layout(self).panel_content.map_or(80, |r| r.width).max(1) as f32;
        let total: f32 = panes.iter().map(|i| self.panel.tabs[*i].weight).sum();
        let step = total * 5.0 / width;
        let d = if right { step } else { -step };
        self.set_divider(a, b, d);
    }

    /// Shift weight between neighbouring panes `a` (left) and `b`, keeping
    /// both at least [`MIN_PANE_COLS`] wide.
    pub(crate) fn set_divider(&mut self, a: usize, b: usize, delta: f32) {
        let panes = self.panel.panes_of(a);
        let width = crate::ui::workbench_layout(self).panel_content.map_or(80, |r| r.width).max(1) as f32;
        let total: f32 = panes.iter().map(|i| self.panel.tabs[*i].weight).sum();
        let min = total * f32::from(MIN_PANE_COLS + 1) / width;
        let (wa, wb) = (self.panel.tabs[a].weight, self.panel.tabs[b].weight);
        let d = delta.clamp(min - wa, wb - min);
        if wa + d >= min && wb - d >= min {
            self.panel.tabs[a].weight = wa + d;
            self.panel.tabs[b].weight = wb - d;
        }
    }

    /// Move the active terminal out of its split into a tab of its own.
    pub(crate) fn unsplit_terminal(&mut self) {
        let idx = self.panel.active;
        if self.panel.panes_of(idx).len() < 2 {
            return;
        }
        let g = self.panel.new_group();
        let t = &mut self.panel.tabs[idx];
        t.group = g;
        t.weight = 1.0;
        self.sync_terminal_size();
    }

    pub(crate) fn terminal_scroll(&mut self, lines: i32) {
        if let Some(s) = self.panel.active_tab().and_then(|t| t.session.as_ref()) {
            s.scroll(lines);
        }
    }

    pub(crate) fn terminal_page(&self) -> i32 {
        crate::ui::workbench_layout(self)
            .panel_content
            .map(|r| i32::from(r.height.saturating_sub(1)).max(1))
            .unwrap_or(10)
    }

    pub(crate) fn terminal_copy(&mut self) -> bool {
        let Some(text) = self.panel.active_tab().and_then(|t| t.session.as_ref()).and_then(Session::selection_text)
        else {
            return false;
        };
        if let Some(bytes) = self.clipboard.set_text(text) {
            self.emit(super::Effect::Write(bytes));
        }
        if let Some(s) = self.panel.active_tab().and_then(|t| t.session.as_ref()) {
            s.select_clear();
        }
        true
    }

    pub(crate) fn terminal_paste(&mut self) {
        let text = self.clipboard.get().text;
        self.terminal_paste_text(&text);
    }

    /// Write raw text (escape sequences included) to the active terminal;
    /// false when none is running (`workbench.action.terminal.sendSequence`).
    pub(crate) fn terminal_send(&mut self, text: &str) -> bool {
        match self.panel.active_tab().and_then(|t| t.session.as_ref()) {
            Some(s) => {
                s.scroll_to_bottom();
                s.write(text.as_bytes());
                true
            }
            None => false,
        }
    }

    pub(crate) fn terminal_paste_text(&mut self, text: &str) {
        if let Some(s) = self.panel.active_tab().and_then(|t| t.session.as_ref()) {
            let bytes = encode_paste(text, s.modes());
            s.scroll_to_bottom();
            s.write(&bytes);
        }
    }

    pub(crate) fn terminal_clear(&mut self) {
        if let Some(s) = self.panel.active_tab().and_then(|t| t.session.as_ref()) {
            s.clear();
        }
    }

    /// Keys while a terminal has focus: skip-shell commands run, everything
    /// else is encoded for the shell.
    pub(crate) fn terminal_key(&mut self, key: KeyPress, ch: Option<char>) {
        let ctx = self.context();
        if let Some(b) = self.keymap.lookup_single(key, &ctx) {
            let cmd = b.command.clone();
            // Pane keys (Alt+arrows, Ctrl+Shift+arrows) reach the shell
            // unless the terminal is split.
            let pane_only = matches!(
                cmd.as_str(),
                "workbench.action.terminal.focusNextPane"
                    | "workbench.action.terminal.focusPreviousPane"
                    | "workbench.action.terminal.resizePaneLeft"
                    | "workbench.action.terminal.resizePaneRight"
            );
            let skip = (DEFAULT_SKIP_SHELL.contains(&cmd.as_str())
                || self.settings.terminal.commands_to_skip_shell.contains(&cmd))
                && (!pane_only || self.panel.visible_panes().len() > 1);
            if skip {
                let args = b.args.clone();
                self.execute(&cmd, args.as_ref());
                return;
            }
        }
        // Ctrl+C copies when there is a selection (VS Code behaviour).
        if key.code == KeyCode::Char('c') && key.mods == Mods::CTRL && self.terminal_copy() {
            return;
        }
        let Some(tab) = self.panel.active_tab() else { return };
        if tab.session.is_none() || tab.exited.is_some() {
            // Not started or exited: Enter (re)starts the shell.
            if key.code == KeyCode::Enter {
                let idx = self.panel.active;
                if let Some(t) = self.panel.tabs.get_mut(idx) {
                    t.session = None;
                }
                self.spawn_into(idx);
                self.sync_terminal_size();
            }
            return;
        }
        let Some(s) = tab.session.as_ref() else { return };
        if let Some(bytes) = encode_key(key, ch, s.modes()) {
            s.select_clear();
            s.scroll_to_bottom();
            s.write(&bytes);
        }
    }

    // --- find in the scrollback ----------------------------------------------

    /// `Ctrl+F` in a terminal: show the find bar (searching for what it
    /// holds again).
    pub(crate) fn terminal_find_open(&mut self) {
        if self.panel.active_tab().and_then(|t| t.session.as_ref()).is_none() {
            return;
        }
        self.term_find.visible = true;
        self.term_find.query.all_selected = true;
        self.focus_changed(Focus::TerminalFind);
        if !self.term_find.query.text.is_empty() {
            self.terminal_find_run(None, true);
        }
    }

    pub(crate) fn terminal_find_close(&mut self) {
        let f = &mut self.term_find;
        f.visible = false;
        f.current = None;
        f.count = None;
        f.error = None;
        f.highlight.replace(None);
        if self.focus == Focus::TerminalFind {
            self.focus_changed(Focus::Terminal);
        }
    }

    /// `Enter` / `F3` (`up`: older output) and `Shift+…`.
    pub(crate) fn terminal_find_next(&mut self, up: bool) {
        if !self.term_find.visible {
            self.terminal_find_open();
            return;
        }
        let same = self.term_find.session == self.panel.active_tab().map(|t| t.id);
        let from = if same { self.term_find.current } else { None };
        self.terminal_find_run(from, up);
    }

    pub(crate) fn terminal_find_toggle(&mut self, c: char) {
        let o = &mut self.term_find.opts;
        match c {
            'c' => o.case_sensitive = !o.case_sensitive,
            'w' => o.whole_word = !o.whole_word,
            _ => o.regex = !o.regex,
        }
        self.terminal_find_run(None, true);
    }

    /// Start a search in the active terminal from `from` (the bottom when
    /// `None`).
    fn terminal_find_run(&mut self, from: Option<TermMatch>, up: bool) {
        let f = &mut self.term_find;
        f.seq += 1;
        f.error = None;
        if f.query.text.is_empty() {
            f.current = None;
            f.count = None;
            f.searching = false;
            f.highlight.replace(None);
            return;
        }
        let o = f.opts;
        let query = match TermQuery::new(&f.query.text, o.case_sensitive, o.whole_word, o.regex) {
            Ok(q) => q,
            Err(e) => {
                f.error = Some(e);
                f.count = None;
                f.highlight.replace(None);
                return;
            }
        };
        f.highlight.replace(Some(query.clone()));
        let Some(tab) = self.panel.active_tab() else { return };
        let Some(s) = tab.session.as_ref() else { return };
        let (session, searcher) = (tab.id, s.searcher());
        let f = &mut self.term_find;
        f.searching = true;
        f.session = Some(session);
        let job = TermFindJob { seq: f.seq, session, searcher, query, from, up };
        self.emit(super::Effect::TermFind(Box::new(job)));
    }

    pub(crate) fn on_term_find_done(
        &mut self,
        seq: u64,
        session: SessionId,
        found: Option<TermMatch>,
        count: (usize, Option<usize>),
    ) {
        if seq != self.term_find.seq {
            return;
        }
        self.term_find.searching = false;
        self.term_find.count = Some(count);
        if let Some(m) = found {
            self.term_find.current = Some(m);
            if let Some(s) = self.panel.tabs.iter().find(|t| t.id == session).and_then(|t| t.session.as_ref()) {
                s.show_match(m);
            }
        }
        self.needs_render = true;
    }

    /// Typing in the terminal find bar searches as you type.
    pub(crate) fn terminal_find_input_key(&mut self, key: KeyPress, ch: Option<char>) {
        let before = self.term_find.query.text.clone();
        self.term_find.query.handle(key, ch);
        if self.term_find.query.text != before {
            self.terminal_find_run(None, true);
        }
    }

    pub(crate) fn terminal_find_paste(&mut self, text: &str) {
        self.term_find.query.insert(text.lines().next().unwrap_or(""));
        self.terminal_find_run(None, true);
    }

    /// Keep each visible terminal's PTY size equal to its pane. Terminals
    /// of other tabs keep their size until shown.
    pub(crate) fn sync_terminal_size(&mut self) {
        let wl = crate::ui::workbench_layout(self);
        let Some(content) = wl.panel_content else { return };
        for (idx, r) in crate::ui::terminal_panes(self, content) {
            let size = (r.height.max(1), r.width.max(2));
            let t = &mut self.panel.tabs[idx];
            if t.applied_size == Some(size) {
                continue;
            }
            t.applied_size = Some(size);
            if let Some(s) = &t.session {
                s.resize(size.0, size.1);
            }
        }
    }

    /// Recreate saved terminal tabs as "not started" placeholders; the
    /// active one starts a *new* shell when the panel is shown (spec §4.9).
    pub(crate) fn restore_terminals(
        &mut self,
        tabs: Vec<(Option<RootId>, yace_workspace::session::TerminalState)>,
        active: usize,
        visible: bool,
    ) {
        // Saved group numbers map to new groups (0: an older session file,
        // one tab per terminal).
        let mut groups: Vec<(u32, u32)> = Vec::new();
        for (root, t) in tabs {
            let id = self.panel.next_id;
            self.panel.next_id += 1;
            let group = match groups.iter().find(|(saved, _)| *saved == t.group && t.group != 0) {
                Some((_, g)) => *g,
                None => {
                    let g = self.panel.new_group();
                    groups.push((t.group, g));
                    g
                }
            };
            self.panel.tabs.push(TermTab {
                id,
                session: None,
                name: t.name,
                title: None,
                root,
                cwd: t.cwd,
                profile: t.profile,
                exited: None,
                not_started: true,
                startup: None,
                group,
                weight: if t.weight > 0.0 { t.weight } else { 1.0 },
                applied_size: None,
            });
        }
        self.panel.active = active.min(self.panel.tabs.len().saturating_sub(1));
        self.panel.visible = visible && !self.panel.tabs.is_empty();
    }

    /// Label for a tab (a group of split terminals): "1: zsh │ node".
    pub fn terminal_group_label(&self, number: usize, panes: &[usize]) -> String {
        if panes.len() == 1 {
            let mut l = self.terminal_label(panes[0]);
            if let Some(rest) = l.split_once(": ").map(|(_, r)| r.to_string()) {
                l = format!("{number}: {rest}");
            }
            return l;
        }
        let names: Vec<String> = panes
            .iter()
            .map(|i| {
                let t = &self.panel.tabs[*i];
                t.title.clone().unwrap_or_else(|| t.name.clone())
            })
            .collect();
        format!("{number}: {}", names.join(" │ "))
    }

    /// Label for a tab: "1: zsh — GLOBAL API".
    pub fn terminal_label(&self, i: usize) -> String {
        let t = &self.panel.tabs[i];
        let mut name = t.title.clone().unwrap_or_else(|| t.name.clone());
        if name.is_empty() {
            name = "terminal".into();
        }
        let root = t.root.and_then(|r| self.workspace.root(r)).map(|r| r.display_name().to_uppercase());
        let mut label = match root {
            Some(r) => format!("{}: {name} — {r}", i + 1),
            None => format!("{}: {name}", i + 1),
        };
        match t.exited {
            Some(Some(code)) => label.push_str(&format!(" [exited {code}]")),
            Some(None) => label.push_str(" [ended]"),
            None if t.not_started => label.push_str(" [not started]"),
            None => {}
        }
        label
    }
}
