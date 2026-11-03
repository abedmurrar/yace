//! Typed continuations for overlays and multi-step flows (unsaved changes,
//! confirmations). Keeping these as data makes every flow testable.

use std::{collections::VecDeque, path::PathBuf};

use yace_workspace::RootId;

use crate::editor::{DocId, GroupId};

#[derive(Debug, Clone, PartialEq)]
pub enum PromptAction {
    NewFile { dir: PathBuf },
    NewFolder { dir: PathBuf },
    Rename { path: PathBuf },
    Move { path: PathBuf },
    AddRoot,
    LocateRoot(RootId),
    RenameRoot(RootId),
    SaveAs { doc: DocId },
    SaveWorkspaceAs,
    OpenFolder,
    OpenWorkspaceFile,
    OpenFile,
    GotoLine,
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
