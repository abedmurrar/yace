//! Workspace lifecycle (spec §4.3): open/switch, add/remove/rename/reorder
//! roots, save as, recent workspaces.

use std::path::PathBuf;

use yace_workspace::{
    RootId, RootStatus, Workspace, WorkspaceKind,
    paths::{absolutize, expand_tilde},
    session::RecentKind,
};

use super::{App, Focus, Severity};
use crate::{
    actions::{AfterClose, ConfirmAction, PickerAction, PromptAction, RootAction, WorkspaceTarget},
    editor::{DocumentStore, Layout},
    explorer::Explorer,
    overlay::{Confirm, LineInput, Overlay, Picker, PickerItem, Prompt},
};

impl App {
    /// Install a workspace at startup or after a switch (no prompts).
    pub fn install_workspace(&mut self, ws: Workspace, restore: bool) {
        if !self.workspace.roots().is_empty() {
            self.plugin_event("workspaceClosed", serde_json::json!({ "name": self.workspace.name() }));
        }
        self.kill_all_terminals();
        // Unsaved changes were settled before switching.
        self.backups_close();
        self.workspace = ws;
        self.docs = DocumentStore::default();
        self.layout = Layout::default();
        self.closed_tabs.clear();
        self.recent_files.clear();
        self.conflict_prompted.clear();
        self.reload_settings();
        self.explorer = Explorer::new(&self.settings.files.exclude, self.settings.explorer.show_hidden);
        self.sidebar_width = self.settings.workbench.sidebar_width;
        self.panel.height = self.settings.terminal.panel_height.max(3);
        self.panel.maximized = false;
        self.sidebar_visible = true;
        self.explorer.sync_roots(&self.workspace);
        self.focus = Focus::Explorer;
        if restore && self.restore_sessions {
            self.restore_session();
        }
        self.recover_backups();
        self.touch_recent();
        self.emit_title();
        self.plugins_workspace_changed(true);
        self.git_workspace_changed();
        for r in self.workspace.roots().to_vec() {
            if let RootStatus::Missing(why) = &r.status {
                self.notify(
                    Severity::Warning,
                    format!("Workspace folder '{}' is unavailable: {why}", r.display_name()),
                );
            }
        }
        self.needs_render = true;
    }

    fn touch_recent(&mut self) {
        let label = self.workspace.name();
        match &self.workspace.kind {
            WorkspaceKind::Folder => {
                if let Some(r) = self.workspace.roots().first() {
                    self.recent.touch(RecentKind::Folder, r.path.clone(), label);
                }
            }
            WorkspaceKind::File(p) | WorkspaceKind::Untitled(p) => {
                if self.workspace.roots().is_empty() && matches!(self.workspace.kind, WorkspaceKind::Untitled(_)) {
                    return;
                }
                self.recent.touch(RecentKind::Workspace, p.clone(), label);
            }
        }
        if let Err(e) = self.recent.save(&self.paths.recent_file()) {
            tracing::warn!("could not save recent list: {e}");
        }
    }

    /// Ask about unsaved changes, then switch.
    pub(crate) fn switch_workspace(&mut self, target: WorkspaceTarget) {
        if self.confirm_running_terminals(AfterClose::SwitchWorkspace(target.clone())) {
            return;
        }
        let docs = self.docs.ids();
        self.start_close(docs, AfterClose::SwitchWorkspace(target));
    }

    pub(crate) fn switch_workspace_now(&mut self, target: WorkspaceTarget) {
        let ws = match &target {
            WorkspaceTarget::Empty => Ok(Workspace::empty_untitled(self.new_untitled_backing())),
            WorkspaceTarget::Folder(p) => Ok(Workspace::from_folder(p)),
            WorkspaceTarget::File(p) => Workspace::open_file(p),
        };
        match ws {
            Ok(ws) => {
                self.save_session();
                self.install_workspace(ws, true);
            }
            Err(e) => self.notify(Severity::Error, e.to_string()),
        }
    }

    pub(crate) fn new_untitled_backing(&self) -> PathBuf {
        let stamp =
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0);
        self.paths.state_dir.join("untitled").join(format!("workspace-{stamp}.code-workspace"))
    }

    pub(crate) fn open_folder_from_input(&mut self, text: &str) -> Result<(), String> {
        let p = absolutize(&expand_tilde(text));
        if !p.is_dir() {
            return Err(format!("{} is not a folder", p.display()));
        }
        self.switch_workspace(WorkspaceTarget::Folder(p));
        Ok(())
    }

    pub(crate) fn open_workspace_from_input(&mut self, text: &str) -> Result<(), String> {
        let p = absolutize(&expand_tilde(text));
        if p.is_dir() {
            self.switch_workspace(WorkspaceTarget::Folder(p));
            return Ok(());
        }
        if !p.is_file() {
            return Err(format!("{} does not exist", p.display()));
        }
        // Validate before asking about unsaved changes.
        Workspace::open_file(&p).map_err(|e| e.to_string())?;
        self.switch_workspace(WorkspaceTarget::File(p));
        Ok(())
    }

    pub(crate) fn open_recent(&mut self, index: usize) {
        let Some(e) = self.recent.entries.get(index).cloned() else { return };
        if !e.path.exists() {
            self.notify(
                Severity::Warning,
                format!("{} no longer exists; removed from the recent list", e.path.display()),
            );
            self.recent.remove(&e.path);
            let _ = self.recent.save(&self.paths.recent_file());
            return;
        }
        let target = match e.kind {
            RecentKind::Folder => WorkspaceTarget::Folder(e.path),
            RecentKind::Workspace => WorkspaceTarget::File(e.path),
        };
        self.switch_workspace(target);
    }

    pub(crate) fn show_recent(&mut self) {
        let home = std::env::var("HOME").unwrap_or_default();
        let items = self
            .recent
            .entries
            .iter()
            .enumerate()
            .map(|(i, e)| {
                let mut detail = e.path.display().to_string();
                if !home.is_empty() && detail.starts_with(&home) {
                    detail = format!("~{}", &detail[home.len()..]);
                }
                PickerItem {
                    label: e.label.clone(),
                    detail,
                    hint: match e.kind {
                        RecentKind::Folder => "folder".into(),
                        RecentKind::Workspace => "workspace".into(),
                    },
                    data: i,
                }
            })
            .collect();
        let mut p = Picker::new("Open Recent", items, PickerAction::OpenRecent);
        p.empty_text = "No recent workspaces".into();
        self.open_overlay(Overlay::Picker(p));
    }

    pub(crate) fn prompt(&mut self, title: &str, hint: &str, initial: String, action: PromptAction, paths: bool) {
        self.open_overlay(Overlay::Prompt(Prompt {
            title: title.into(),
            hint: Some(hint.into()),
            input: LineInput::with_selection(initial),
            action,
            path_completion: paths,
            error: None,
        }));
    }

    pub(crate) fn cwd_with_slash(&self) -> String {
        let d = std::env::current_dir().unwrap_or_default();
        let s = d.display().to_string();
        if s.ends_with('/') { s } else { format!("{s}/") }
    }

    pub(crate) fn add_root_from_input(&mut self, text: &str) -> Result<(), String> {
        let p = absolutize(&expand_tilde(text));
        if !p.is_dir() {
            return Err(format!("{} is not a folder", p.display()));
        }
        if self.workspace.kind == WorkspaceKind::Folder {
            let backing = self.new_untitled_backing();
            self.workspace.make_untitled(backing);
        }
        self.workspace.add_root(&p, None);
        self.after_roots_changed();
        if let Some(r) = self.workspace.roots().last()
            && let RootStatus::Duplicate(_) = r.status
        {
            self.notify(Severity::Warning, format!("{} is already in the workspace", p.display()));
        }
        Ok(())
    }

    pub(crate) fn locate_root(&mut self, id: RootId, text: &str) -> Result<(), String> {
        let p = absolutize(&expand_tilde(text));
        if !p.is_dir() {
            return Err(format!("{} is not a folder", p.display()));
        }
        self.workspace.relocate_root(id, &p);
        self.after_roots_changed();
        Ok(())
    }

    pub(crate) fn remove_root(&mut self, id: RootId) {
        let name = self.workspace.root(id).map(|r| r.display_name()).unwrap_or_default();
        if self.workspace.remove_root(id) {
            self.after_roots_changed();
            self.notify(
                Severity::Info,
                format!("Removed '{name}' from the workspace (files on disk were not touched)"),
            );
        }
    }

    /// Persist root changes to the workspace file and refresh views.
    pub(crate) fn after_roots_changed(&mut self) {
        self.canon_cache.clear();
        if self.workspace.file().is_some()
            && let Err(e) = self.workspace.save()
        {
            self.notify(Severity::Error, format!("Could not update the workspace file: {e}"));
        }
        self.reload_settings();
        self.explorer.sync_roots(&self.workspace);
        self.touch_recent();
        self.emit_title();
        self.plugins_workspace_changed(false);
        self.git_workspace_changed();
    }

    pub(crate) fn save_workspace_as(&mut self, text: &str) -> Result<(), String> {
        if text.is_empty() {
            return Err("Enter a file path".into());
        }
        let mut p = absolutize(&expand_tilde(text));
        if p.extension().is_none_or(|e| e != "code-workspace") {
            p.set_extension("code-workspace");
        }
        if p.is_dir() {
            return Err("That path is a folder".into());
        }
        self.save_session();
        self.workspace.save_as(&p).map_err(|e| e.to_string())?;
        self.touch_recent();
        self.emit_title();
        self.notify(Severity::Info, format!("Workspace saved to {}", p.display()));
        Ok(())
    }

    /// Root targeted by a root command: the Explorer selection, or ask.
    pub(crate) fn with_root(&mut self, action: RootAction) {
        if self.focus == Focus::Explorer
            && let Some(r) = self.explorer.selected_row()
            && r.depth == 0
        {
            let id = r.root;
            self.root_action(action, id);
            return;
        }
        if self.workspace.roots().len() == 1 && action != RootAction::Remove {
            let id = self.workspace.roots()[0].id;
            self.root_action(action, id);
            return;
        }
        let items = self
            .workspace
            .roots()
            .iter()
            .map(|r| PickerItem {
                label: r.display_name(),
                detail: r.path.display().to_string(),
                hint: match &r.status {
                    RootStatus::Missing(m) => format!("unavailable: {m}"),
                    RootStatus::Duplicate(_) => "duplicate".into(),
                    RootStatus::Available => String::new(),
                },
                data: r.id.0 as usize,
            })
            .collect();
        let title = match action {
            RootAction::Remove => "Remove Folder from Workspace",
            RootAction::Rename => "Rename Workspace Folder",
            RootAction::MoveUp => "Move Workspace Folder Up",
            RootAction::MoveDown => "Move Workspace Folder Down",
            RootAction::Locate => "Locate Workspace Folder",
        };
        let mut p = Picker::new(title, items, PickerAction::Root(action));
        p.empty_text = "The workspace has no folders".into();
        self.open_overlay(Overlay::Picker(p));
    }

    pub(crate) fn root_action(&mut self, action: RootAction, id: RootId) {
        let Some(root) = self.workspace.root(id).cloned() else { return };
        match action {
            RootAction::Remove => self.open_overlay(Overlay::Confirm(Confirm {
                title: "Remove folder from workspace".into(),
                message: vec![
                    format!("Remove '{}' from this workspace?", root.display_name()),
                    format!("{} — the folder and its files on disk are NOT deleted.", root.path.display()),
                ],
                buttons: vec![
                    ("Remove from Workspace".into(), ConfirmAction::RemoveRoot(id)),
                    ("Cancel".into(), ConfirmAction::Nothing),
                ],
                selected: 0,
                cancel: ConfirmAction::Nothing,
            })),
            RootAction::Rename => self.prompt(
                "Rename Workspace Folder",
                "Display name (leave empty for the folder name)",
                root.name.clone().unwrap_or_else(|| root.display_name()),
                PromptAction::RenameRoot(id),
                false,
            ),
            RootAction::MoveUp | RootAction::MoveDown => {
                let delta = if action == RootAction::MoveUp { -1 } else { 1 };
                if self.workspace.move_root(id, delta) {
                    if self.workspace.kind == WorkspaceKind::Folder {
                        let b = self.new_untitled_backing();
                        self.workspace.make_untitled(b);
                    }
                    self.after_roots_changed();
                    if let Some(i) = self.explorer.rows.iter().position(|r| r.root == id && r.depth == 0) {
                        self.explorer.selected = i;
                    }
                }
            }
            RootAction::Locate => self.prompt(
                "Locate Workspace Folder",
                "New path for this folder",
                root.path.display().to_string(),
                PromptAction::LocateRoot(id),
                true,
            ),
        }
    }

    pub(crate) fn workspace_file_default(&self) -> String {
        match self.workspace.file() {
            Some(p) if !matches!(self.workspace.kind, WorkspaceKind::Untitled(_)) => p.display().to_string(),
            _ => {
                let name =
                    self.workspace.roots().first().map(|r| r.display_name()).unwrap_or_else(|| "workspace".into());
                format!("{}{}.code-workspace", self.cwd_with_slash(), name.replace(['/', ' '], "-"))
            }
        }
    }
}
