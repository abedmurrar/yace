//! Explorer actions and filesystem operations (spec §8.1, §8.2).

use std::{
    fs, io,
    path::{Path, PathBuf},
};

use super::{App, Severity};
use crate::{
    actions::{ConfirmAction, PromptAction},
    explorer::RowKind,
    overlay::{Confirm, Overlay},
};

impl App {
    pub(crate) fn explorer_select(&mut self, preview: bool) {
        let Some(row) = self.explorer.selected_row().cloned() else { return };
        if row.is_dir_like() {
            let i = self.explorer.selected;
            self.explorer.toggle(i);
        } else if row.is_file() {
            self.open_file(&row.path, preview);
            if preview {
                // Space/single-click preview keeps focus in the Explorer.
                self.focus = super::Focus::Explorer;
            }
        } else if let RowKind::Root { missing: Some(_), .. } = row.kind {
            self.root_action(crate::actions::RootAction::Locate, row.root);
        }
    }

    pub(crate) fn explorer_expand(&mut self) {
        let Some(row) = self.explorer.selected_row().cloned() else { return };
        if row.is_dir_like() {
            if row.expanded {
                self.explorer.move_selection(1);
            } else {
                let i = self.explorer.selected;
                self.explorer.set_expanded(i, true);
            }
        } else if row.is_file() {
            self.open_file(&row.path, true);
            self.focus = super::Focus::Explorer;
        }
    }

    pub(crate) fn explorer_collapse(&mut self) {
        let i = self.explorer.selected;
        let Some(row) = self.explorer.rows.get(i).cloned() else { return };
        if row.is_dir_like() && row.expanded {
            self.explorer.set_expanded(i, false);
        } else if let Some(p) = self.explorer.parent_row(i) {
            self.explorer.selected = p;
        }
    }

    pub(crate) fn reveal_active_file(&mut self) {
        let Some(path) = self.active().and_then(|(_, _, d)| self.docs.doc(d)?.path().map(Path::to_path_buf)) else {
            self.notify(Severity::Info, "No file is open");
            return;
        };
        self.explorer.filter.clear();
        self.explorer.reveal(&path);
        self.focus_explorer();
        if self.workspace.root_for_path(&path).is_none() {
            self.notify(Severity::Info, "The active file is not inside any workspace folder");
        }
    }

    fn selected_path(&self) -> Option<std::path::PathBuf> {
        self.explorer.selected_row().filter(|r| !matches!(r.kind, RowKind::Note { .. })).map(|r| r.path.clone())
    }

    pub(crate) fn explorer_new(&mut self, folder: bool) {
        let Some((_, dir)) = self.explorer.target_dir() else {
            self.notify(Severity::Info, "Select a folder in the Explorer first");
            return;
        };
        let (title, action) = if folder {
            ("New Folder", PromptAction::NewFolder { dir: dir.clone() })
        } else {
            ("New File", PromptAction::NewFile { dir: dir.clone() })
        };
        self.prompt(
            title,
            &format!("Name inside {} (use / for subfolders)", dir.display()),
            String::new(),
            action,
            false,
        );
    }

    pub(crate) fn create_entry(&mut self, dir: &Path, name: &str, folder: bool) -> Result<(), String> {
        if name.is_empty() || name.split('/').any(|c| c == "..") {
            return Err("Enter a valid name".into());
        }
        let path = dir.join(name);
        if path.exists() {
            return Err(format!("{} already exists", path.display()));
        }
        let result = if folder {
            fs::create_dir_all(&path)
        } else {
            path.parent()
                .map_or(Ok(()), fs::create_dir_all)
                .and_then(|_| fs::OpenOptions::new().write(true).create_new(true).open(&path).map(|_| ()))
        };
        result.map_err(|e| format!("Could not create {}: {e}", path.display()))?;
        self.explorer.refresh_dir(dir);
        self.explorer.reveal(&path);
        if !folder {
            self.open_file(&path, false);
        }
        Ok(())
    }

    pub(crate) fn explorer_rename(&mut self) {
        let Some(row) = self.explorer.selected_row().cloned() else { return };
        match row.kind {
            RowKind::Root { .. } => self.root_action(crate::actions::RootAction::Rename, row.root),
            RowKind::Note { .. } => {}
            _ => self.prompt(
                "Rename",
                "New name",
                row.label.clone(),
                PromptAction::Rename { path: row.path.clone() },
                false,
            ),
        }
    }

    pub(crate) fn rename_entry(&mut self, path: &Path, name: &str) -> Result<(), String> {
        if name.is_empty() || name.contains('/') {
            return Err("Enter a name without '/' (use Move to change folders)".into());
        }
        let target = path.with_file_name(name);
        self.move_path(path, &target)
    }

    pub(crate) fn explorer_move(&mut self) {
        let Some(p) = self.selected_path() else { return };
        if self.explorer.selected_row().is_some_and(|r| r.depth == 0) {
            self.notify(Severity::Info, "Workspace folders cannot be moved from here; use Locate or Remove");
            return;
        }
        self.prompt("Move", "Destination path", p.display().to_string(), PromptAction::Move { path: p }, true);
    }

    pub(crate) fn move_entry(&mut self, path: &Path, dest: &str) -> Result<(), String> {
        let mut target = yace_workspace::paths::absolutize(&yace_workspace::paths::expand_tilde(dest));
        if target.is_dir()
            && !target.starts_with(path)
            && let Some(name) = path.file_name()
        {
            target = target.join(name);
        }
        self.move_path(path, &target)
    }

    fn move_path(&mut self, from: &Path, to: &Path) -> Result<(), String> {
        if from == to {
            return Ok(());
        }
        if to.exists() {
            return Err(format!("{} already exists", to.display()));
        }
        if to.starts_with(from) {
            return Err("Cannot move a folder into itself".into());
        }
        if let Some(parent) = to.parent()
            && !parent.is_dir()
        {
            return Err(format!("{} does not exist", parent.display()));
        }
        match fs::rename(from, to) {
            Ok(()) => {}
            Err(e) if e.raw_os_error() == Some(18) => {
                // EXDEV: across filesystems — copy then delete.
                copy_recursive(from, to).map_err(|e| format!("Copy failed: {e}"))?;
                remove_any(from).map_err(|e| format!("Copied, but could not remove the original: {e}"))?;
            }
            Err(e) => return Err(format!("Could not move: {e}")),
        }
        self.retarget_docs(from, to);
        for d in [from.parent(), to.parent()].into_iter().flatten() {
            self.explorer.refresh_dir(d);
        }
        self.explorer.reveal(to);
        Ok(())
    }

    pub(crate) fn explorer_delete(&mut self) {
        let Some(row) = self.explorer.selected_row().cloned() else { return };
        if row.depth == 0 {
            // Roots are never deleted from disk; offer removal instead.
            self.root_action(crate::actions::RootAction::Remove, row.root);
            return;
        }
        if matches!(row.kind, RowKind::Note { .. }) {
            return;
        }
        let what = if row.is_dir_like() { "folder" } else { "file" };
        if !self.settings.explorer.confirm_delete {
            self.trash_entry(&row.path);
            return;
        }
        self.open_overlay(Overlay::Confirm(Confirm {
            title: format!("Delete {what}"),
            message: vec![format!("Move '{}' to the Trash?", row.label), row.path.display().to_string()],
            buttons: vec![
                ("Move to Trash".into(), ConfirmAction::Trash(row.path.clone())),
                ("Cancel".into(), ConfirmAction::Nothing),
            ],
            selected: 0,
            cancel: ConfirmAction::Nothing,
        }));
    }

    pub(crate) fn trash_entry(&mut self, path: &Path) {
        match trash::delete(path) {
            Ok(()) => {
                if let Some(p) = path.parent() {
                    self.explorer.refresh_dir(p);
                }
                self.check_open_docs_on_disk();
            }
            Err(e) => self.open_overlay(Overlay::Confirm(Confirm {
                title: "Could not move to Trash".into(),
                message: vec![
                    format!("{}: {e}", path.display()),
                    "Delete it permanently instead? This cannot be undone.".into(),
                ],
                buttons: vec![
                    ("Cancel".into(), ConfirmAction::Nothing),
                    ("Delete Permanently".into(), ConfirmAction::DeletePermanently(path.to_path_buf())),
                ],
                selected: 0,
                cancel: ConfirmAction::Nothing,
            })),
        }
    }

    pub(crate) fn delete_permanently(&mut self, path: &Path) {
        match remove_any(path) {
            Ok(()) => {
                if let Some(p) = path.parent() {
                    self.explorer.refresh_dir(p);
                }
                self.check_open_docs_on_disk();
            }
            Err(e) => self.notify(Severity::Error, format!("Could not delete {}: {e}", path.display())),
        }
    }

    // ------------------------------------------------------------------
    // Copy / cut / paste (spec §8.2)

    /// Remember the selected file or folder for Paste (`filesExplorer.copy`
    /// / `.cut`); its path also goes to the clipboard as text.
    pub(crate) fn explorer_copy_files(&mut self, cut: bool) {
        if self.sidebar_view != super::SidebarView::Explorer {
            return;
        }
        let Some(row) = self.explorer.selected_row().cloned() else { return };
        if matches!(row.kind, RowKind::Note { .. }) {
            return;
        }
        if row.depth == 0 && cut {
            self.notify(Severity::Info, "A workspace folder can't be cut; use Remove Folder from Workspace");
            return;
        }
        let text = row.path.display().to_string();
        if let Some(bytes) = self.clipboard.set_text(text) {
            self.emit(super::Effect::Write(bytes));
        }
        self.file_clipboard = Some(FileClipboard { paths: vec![row.path.clone()], cut });
        let what = if cut { "Cut" } else { "Copied" };
        self.notify(Severity::Info, format!("{what} {} — select a folder and paste (Ctrl+V)", row.label));
    }

    /// Paste into the selected folder (or the selected file's folder).
    pub(crate) fn explorer_paste(&mut self) {
        if self.sidebar_view != super::SidebarView::Explorer {
            return;
        }
        let Some(clip) = self.file_clipboard.clone() else {
            self.notify(Severity::Info, "Nothing to paste: copy or cut a file in the Explorer first");
            return;
        };
        let Some((_, dir)) = self.explorer.target_dir() else {
            self.notify(Severity::Info, "Select a folder in the Explorer first");
            return;
        };
        let mut copies = Vec::new();
        for from in &clip.paths {
            let Some(name) = from.file_name() else { continue };
            if !from.exists() && fs::symlink_metadata(from).is_err() {
                self.notify(Severity::Error, format!("{} no longer exists", from.display()));
                continue;
            }
            if dir.starts_with(from) && from.is_dir() {
                let what = if clip.cut { "move" } else { "copy" };
                self.notify(Severity::Error, format!("Cannot {what} a folder into itself"));
                continue;
            }
            if clip.cut {
                if from.parent() == Some(dir.as_path()) {
                    continue; // already there
                }
                if let Err(e) = self.move_path(from, &dir.join(name)) {
                    self.notify(Severity::Error, e);
                }
            } else {
                copies.push((from.clone(), unique_destination(&dir, &name.to_string_lossy())));
            }
        }
        if clip.cut {
            self.file_clipboard = None;
        }
        if !copies.is_empty() {
            self.emit(super::Effect::CopyPaths(copies));
        }
    }

    /// Background copies finished.
    pub(crate) fn on_paths_copied(&mut self, results: Vec<(PathBuf, Result<(), String>)>) {
        let mut reveal = None;
        for (to, r) in results {
            if let Some(p) = to.parent() {
                self.explorer.refresh_dir(p);
            }
            match r {
                Ok(()) => reveal = reveal.or(Some(to)),
                Err(e) => self.notify(Severity::Error, e),
            }
        }
        if let Some(p) = reveal {
            self.explorer.reveal(&p);
        }
    }

    /// Whether a path is waiting to be moved by Paste (drawn dimmed).
    pub fn is_cut(&self, path: &Path) -> bool {
        self.file_clipboard.as_ref().is_some_and(|c| c.cut && c.paths.iter().any(|p| p == path))
    }

    /// Open the selected file in the group to the right, splitting if
    /// there is none (`explorer.openToSide`).
    pub(crate) fn explorer_open_beside(&mut self) {
        let Some(row) = self.explorer.selected_row().cloned() else { return };
        if !row.is_file() {
            self.explorer_select(false);
            return;
        }
        self.open_beside(&row.path);
    }

    pub(crate) fn open_beside(&mut self, path: &Path) {
        let g = self.layout.next_group_or_split(true);
        self.layout.active = g;
        self.open_file(path, false);
        self.focus_editor();
    }

    /// Open the folder containing the selected (or active) file in the
    /// system file manager (`revealFileInOS`). Only a folder is opened —
    /// never the file itself.
    pub(crate) fn reveal_in_os(&mut self) {
        let path = if self.focus == super::Focus::Explorer && self.sidebar_view == super::SidebarView::Explorer {
            self.selected_path()
        } else {
            self.active().and_then(|(_, _, d)| self.docs.doc(d)?.path().map(Path::to_path_buf))
        };
        let Some(path) = path else {
            self.notify(Severity::Info, "Select a file or folder first");
            return;
        };
        let dir = if path.is_dir() { path } else { path.parent().map(Path::to_path_buf).unwrap_or(path) };
        self.emit(super::Effect::OpenExternal(dir));
    }

    pub(crate) fn copy_path(&mut self, relative: bool) {
        let path = if self.focus == super::Focus::Explorer {
            self.selected_path()
        } else {
            self.active().and_then(|(_, _, d)| self.docs.doc(d)?.path().map(Path::to_path_buf))
        };
        let Some(path) = path else { return };
        let text = if relative {
            match self.workspace.relative(&path) {
                Some((_, rel)) => rel.display().to_string(),
                None => path.display().to_string(),
            }
        } else {
            path.display().to_string()
        };
        if let Some(bytes) = self.clipboard.set_text(text.clone()) {
            self.emit(super::Effect::Write(bytes));
        }
        self.notify(Severity::Info, format!("Copied {text}"));
    }
}

/// Files the Explorer holds for Paste.
#[derive(Debug, Clone, PartialEq)]
pub struct FileClipboard {
    pub paths: Vec<PathBuf>,
    pub cut: bool,
}

/// `dir/name` if free, else VS Code's `name copy.ext`, `name copy 2.ext`, …
pub fn unique_destination(dir: &Path, name: &str) -> PathBuf {
    let first = dir.join(name);
    if fs::symlink_metadata(&first).is_err() {
        return first;
    }
    let (stem, ext) = match name.rsplit_once('.') {
        Some((s, e)) if !s.is_empty() => (s, format!(".{e}")),
        _ => (name, String::new()),
    };
    for n in 1.. {
        let candidate = if n == 1 { format!("{stem} copy{ext}") } else { format!("{stem} copy {n}{ext}") };
        let p = dir.join(candidate);
        if fs::symlink_metadata(&p).is_err() {
            return p;
        }
    }
    unreachable!()
}

/// Copy each `(from, to)`; `to` must not exist. A failed copy removes what
/// it created. Runs on a background thread.
pub fn copy_paths(pairs: Vec<(PathBuf, PathBuf)>) -> Vec<(PathBuf, Result<(), String>)> {
    pairs
        .into_iter()
        .map(|(from, to)| {
            if fs::symlink_metadata(&to).is_ok() {
                return (to.clone(), Err(format!("{} already exists", to.display())));
            }
            let r = copy_recursive(&from, &to).map_err(|e| {
                let _ = remove_any(&to);
                format!("Could not copy {} to {}: {e}", from.display(), to.display())
            });
            (to, r)
        })
        .collect()
}

/// Open a folder in the system file manager, detached.
pub fn open_external(dir: &Path) -> Result<(), String> {
    use std::process::{Command, Stdio};
    let program = std::env::var_os("YACE_OPENER")
        .unwrap_or_else(|| if cfg!(target_os = "macos") { "open".into() } else { "xdg-open".into() });
    let mut cmd = Command::new(&program);
    cmd.arg(dir).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        cmd.process_group(0);
    }
    match cmd.spawn() {
        Ok(mut child) => {
            // Reap it in the background so it never lingers as a zombie.
            std::thread::Builder::new().name("yace-opener".into()).spawn(move || child.wait()).ok();
            Ok(())
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            Err(format!("Could not open the file manager: `{}` is not installed", program.to_string_lossy()))
        }
        Err(e) => Err(format!("Could not open the file manager: {e}")),
    }
}

fn copy_recursive(from: &Path, to: &Path) -> io::Result<()> {
    let meta = fs::symlink_metadata(from)?;
    if meta.file_type().is_symlink() {
        let target = fs::read_link(from)?;
        #[cfg(unix)]
        std::os::unix::fs::symlink(target, to)?;
        #[cfg(not(unix))]
        let _ = target;
        return Ok(());
    }
    if meta.is_dir() {
        fs::create_dir_all(to)?;
        for e in fs::read_dir(from)? {
            let e = e?;
            copy_recursive(&e.path(), &to.join(e.file_name()))?;
        }
        fs::set_permissions(to, meta.permissions())?;
        return Ok(());
    }
    fs::copy(from, to).map(|_| ())
}

fn remove_any(p: &Path) -> io::Result<()> {
    let meta = fs::symlink_metadata(p)?;
    if meta.is_dir() { fs::remove_dir_all(p) } else { fs::remove_file(p) }
}
