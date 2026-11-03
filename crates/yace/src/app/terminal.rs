//! The integrated terminal panel (spec §11): sessions per workspace root,
//! focus-based input routing, lifecycle and the bottom panel.

use std::path::{Path, PathBuf};

use crossbeam_channel::{Receiver, Sender, unbounded};
use yace_config::{ConfirmKill, TerminalCwd};
use yace_keymap::{KeyCode, KeyPress, Mods};
use yace_terminal::{Notice, Screen, Session, SessionConfig, SessionId, default_shell, encode_key, encode_paste};
use yace_workspace::{RootId, RootStatus};

use super::{App, Focus, Severity};
use crate::{
    actions::{AfterClose, ConfirmAction, PickerAction, PromptAction},
    overlay::{Confirm, Overlay, Picker, PickerItem},
};

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
    notice_tx: Sender<Notice>,
    pub notice_rx: Receiver<Notice>,
    /// Size last applied to the sessions (rows, cols).
    applied_size: Option<(u16, u16)>,
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
            notice_tx,
            notice_rx,
            applied_size: None,
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
}

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
    fn terminal_target(&self) -> (Option<RootId>, PathBuf) {
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

    fn session_config(&self, cwd: &Path, profile: Option<&str>) -> (SessionConfig, String) {
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
        let (rows, cols) = self.panel.applied_size.unwrap_or((10, 80));
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
        let (cwd, profile, id) = {
            let t = &self.panel.tabs[idx];
            (t.cwd.clone(), t.profile.clone(), t.id)
        };
        let cwd = if cwd.is_dir() { cwd } else { std::env::current_dir().unwrap_or_else(|_| PathBuf::from("/")) };
        let (cfg, name) = self.session_config(&cwd, profile.as_deref());
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
        self.panel.applied_size = None; // re-apply the panel size
    }

    /// Open a new terminal in `cwd` (root-aware) and focus it.
    pub(crate) fn new_terminal(&mut self, root: Option<RootId>, cwd: PathBuf, profile: Option<String>) {
        let id = self.panel.next_id;
        self.panel.next_id += 1;
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
        });
        self.panel.active = self.panel.tabs.len() - 1;
        self.panel.visible = true;
        let idx = self.panel.active;
        self.sync_terminal_size();
        self.spawn_into(idx);
        self.sync_terminal_size();
        self.focus_changed(Focus::Terminal);
    }

    pub(crate) fn new_terminal_default(&mut self) {
        let (root, cwd) = self.terminal_target();
        self.new_terminal(root, cwd, None);
    }

    /// `` Ctrl+` ``: show and focus the terminal, or hide it.
    pub(crate) fn toggle_terminal(&mut self) {
        if self.panel.visible && self.focus == Focus::Terminal {
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
            if self.focus == Focus::Terminal {
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
        let idx = self.panel.active;
        if self.panel.tabs.get(idx).is_some_and(|t| t.not_started && t.session.is_none()) {
            self.sync_terminal_size();
            self.spawn_into(idx);
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
        let mut tab = self.panel.tabs.remove(i);
        if let Some(s) = tab.session.as_mut() {
            s.kill();
        }
        if self.panel.tabs.is_empty() {
            self.panel.visible = false;
            self.panel.active = 0;
            if self.focus == Focus::Terminal {
                self.focus_changed(Focus::Editor);
            }
        } else {
            self.panel.active = self.panel.active.min(self.panel.tabs.len() - 1);
            self.ensure_active_started();
        }
        self.request_trim();
    }

    pub(crate) fn kill_all_terminals(&mut self) {
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

    pub(crate) fn focus_terminal_tab(&mut self, delta: isize) {
        let n = self.panel.tabs.len();
        if n == 0 {
            return;
        }
        self.panel.active = (self.panel.active as isize + delta).rem_euclid(n as isize) as usize;
        self.ensure_active_started();
        self.focus_changed(Focus::Terminal);
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
            let skip = DEFAULT_SKIP_SHELL.contains(&cmd.as_str())
                || self.settings.terminal.commands_to_skip_shell.contains(&cmd);
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

    /// Keep every session's PTY size equal to the panel content area.
    pub(crate) fn sync_terminal_size(&mut self) {
        let Some(r) = crate::ui::workbench_layout(self).panel_content else { return };
        let size = (r.height.max(1), r.width.max(2));
        if self.panel.applied_size == Some(size) {
            return;
        }
        self.panel.applied_size = Some(size);
        for t in &self.panel.tabs {
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
        for (root, t) in tabs {
            let id = self.panel.next_id;
            self.panel.next_id += 1;
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
            });
        }
        self.panel.active = active.min(self.panel.tabs.len().saturating_sub(1));
        self.panel.visible = visible && !self.panel.tabs.is_empty();
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
