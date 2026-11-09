//! Typed continuations for overlays and multi-step flows (unsaved changes,
//! confirmations). Keeping these as data makes every flow testable.

use std::{collections::VecDeque, path::PathBuf};

use yace_workspace::RootId;

use crate::editor::{DocId, GroupId};

#[derive(Debug, Clone, PartialEq)]
pub enum PromptAction {
    NewFile {
        dir: PathBuf,
    },
    NewFolder {
        dir: PathBuf,
    },
    Rename {
        path: PathBuf,
    },
    Move {
        path: PathBuf,
    },
    AddRoot,
    LocateRoot(RootId),
    RenameRoot(RootId),
    SaveAs {
        doc: DocId,
    },
    SaveWorkspaceAs,
    OpenFolder,
    OpenWorkspaceFile,
    OpenFile,
    GotoLine,
    RenameTerminal(u32),
    /// F2: rename the symbol at `pos` of `doc`.
    RenameSymbol {
        doc: DocId,
        pos: usize,
    },
    /// Import a VS Code colour theme JSON file.
    ImportThemeFile,
    /// Extensions: Install from Directory / Package.
    InstallPlugin,
    /// Commit message for the repository at `toplevel`.
    GitCommit {
        toplevel: PathBuf,
        all: bool,
    },
    GitCreateBranch {
        toplevel: PathBuf,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub enum CloseChoice {
    Save,
    DontSave,
    SaveAll,
    Cancel,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ConfirmAction {
    Nothing,
    /// Move keybindings.toml aside and use the defaults.
    RestoreDefaultKeybindings,
    /// Install `App::pending_install`.
    InstallPlugin,
    UninstallPlugin(String),
    /// Approve a plugin folder already in the plugins folder.
    TrustPlugin(PathBuf),
    /// Smart commit: commit every change of the repository at this path.
    GitCommitAll(PathBuf),
    Close(CloseChoice),
    Trash(PathBuf),
    DeletePermanently(PathBuf),
    OverwriteSave(DocId),
    ReloadFromDisk(DocId),
    KeepMine(DocId),
    RemoveRoot(RootId),
    OpenLarge {
        path: PathBuf,
        preview: bool,
    },
    OverwriteExisting {
        doc: DocId,
        path: PathBuf,
    },
    CreateParentAndSave {
        doc: DocId,
        path: PathBuf,
    },
    /// Workspace search: apply the previewed replacement.
    ReplaceInFiles,
    KillTerminal(u32),
    /// Stop running terminals, then continue with the given step.
    KillTerminalsThen(Box<AfterClose>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RootAction {
    Remove,
    Rename,
    MoveUp,
    MoveDown,
    Locate,
}

#[derive(Debug, Clone, PartialEq)]
pub enum PickerAction {
    RunCommand,
    /// Quick Open (files, `>` commands, `:` line, `?` help).
    QuickOpen,
    OpenRecent,
    Root(RootAction),
    ChangeEol,
    NewTerminalInRoot,
    NewTerminalProfile,
    /// Jump to `App::nav_targets[data]` (definitions, references, problems).
    Navigate,
    /// Run `App::code_actions[data]`.
    CodeAction,
    /// Copy `App::notification_log[data]` to the clipboard.
    CopyNotification,
    /// Preferences: Color Theme (live preview while moving).
    SelectTheme,
    /// Import `App::vscode_themes[data]`.
    ImportVsCodeTheme,
    /// Open user theme file `data` (0 = create a new one).
    EditUserTheme,
    /// Extensions: List Installed (data → `App::plugin_pick`).
    PluginList,
    /// An operation on the plugin with this id (data → `PluginOp`).
    PluginOp(String),
    PluginEnable,
    PluginDisable,
    PluginUninstall,
    /// Show `PluginHost::views()[data]`.
    PluginView,
    /// Install `App::plugin_index[data]` (Extensions: Browse Plugins…).
    PluginBrowse,
    /// Run a repository command on repository `data`.
    GitRepo(crate::app::RepoCmd),
    /// Switch to `GitState::branch_pick` entry `data - 1` (0 = create).
    GitBranch,
}

#[derive(Debug, Clone, PartialEq)]
pub enum WorkspaceTarget {
    Empty,
    Folder(PathBuf),
    File(PathBuf),
}

/// What happens once every dirty document in a close flow is resolved.
#[derive(Debug, Clone, PartialEq)]
pub enum AfterClose {
    Quit,
    CloseViews(Vec<(GroupId, DocId)>),
    CloseGroup(GroupId),
    SwitchWorkspace(WorkspaceTarget),
}

#[derive(Debug, Clone, PartialEq)]
pub struct CloseFlow {
    /// Dirty documents still to ask about.
    pub pending: VecDeque<DocId>,
    pub after: AfterClose,
    /// The document currently waiting on a Save As prompt.
    pub saving: Option<DocId>,
}
