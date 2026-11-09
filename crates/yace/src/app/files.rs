//! Opening, saving and closing documents; the unsaved-changes flow; and
//! reacting to external modification (spec §9.9, §9.10, §10.1).

use std::{
    collections::VecDeque,
    path::{Path, PathBuf},
};

use yace_buffer::{DiskState, Document, LineEnding, io, ops};
use yace_config::Eol;

use super::{App, ClosedTab, Effect, Focus, Severity};
use crate::{
    actions::{AfterClose, CloseChoice, CloseFlow, ConfirmAction, PromptAction},
    editor::{DocId, GroupId, View, path_key},
    overlay::{Confirm, LineInput, Overlay, Prompt},
};

/// Files above this size load on a background thread.
pub const ASYNC_LOAD_BYTES: u64 = 1024 * 1024;

/// Result of trying to save one document.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SaveOutcome {
    Saved,
    /// Untitled: a Save As prompt was opened.
    NeedsPath,
    /// Changed on disk: an overwrite confirmation was opened.
    Conflict,
    Failed,
}

impl App {
    pub(crate) fn default_eol(&self, path: Option<&Path>) -> LineEnding {
        match self.settings_for(path).files.eol {
            Eol::Crlf => LineEnding::CrLf,
            _ => LineEnding::Lf,
        }
    }

    /// Open a file in the active group. `preview` reuses the preview tab.
    pub fn open_file(&mut self, path: &Path, preview: bool) {
        self.open_file_inner(path, preview, false);
    }

    pub(crate) fn open_file_confirmed(&mut self, path: &Path, preview: bool, large_ok: bool) {
        self.open_file_inner(path, preview, large_ok);
    }

    fn open_file_inner(&mut self, path: &Path, preview: bool, large_ok: bool) {
        let path = yace_workspace::paths::absolutize(path);
        let preview = preview && self.settings.workbench.enable_preview;
        let gid = self.layout.active;
        if let Some(doc) = self.docs.find_by_path(&path) {
            let group = self.layout.active_group_mut();
            if let Some(i) = group.find(doc) {
                group.activate(i);
                if !preview {
                    group.views[i].preview = false;
                }
            } else {
                let sel = self
                    .layout
                    .views_of(doc)
                    .first()
                    .and_then(|(g, v)| self.layout.group(*g).map(|gr| gr.views[*v].sel.clone()))
                    .unwrap_or_default();
                self.add_view(gid, View { sel, preview, ..View::new(doc) });
            }
            self.after_open(&path);
            return;
        }
        // Large-file confirmation (spec §9.8).
        let threshold = self.settings.files.large_file_threshold_mb.saturating_mul(1024 * 1024);
        let size = io::file_size(&path).unwrap_or(0);
        if !large_ok && size > threshold && threshold > 0 {
            self.open_overlay(Overlay::Confirm(Confirm {
                title: "Large file".into(),
                message: vec![
                    format!("{} is {:.1} MiB.", path.display(), size as f64 / 1048576.0),
                    "It will open in large-file mode with reduced features.".into(),
                ],
                buttons: vec![
                    ("Open".into(), ConfirmAction::OpenLarge { path: path.clone(), preview }),
                    ("Cancel".into(), ConfirmAction::Nothing),
                ],
                selected: 0,
                cancel: ConfirmAction::Nothing,
            }));
            return;
        }
        // Big files load on a worker so the UI never blocks (spec §9.8).
        if path.exists() && size > ASYNC_LOAD_BYTES {
            let eol = self.default_eol(Some(&path));
            let mut placeholder = Document::new_untitled(eol);
            placeholder.set_path(Some(path.clone()));
            let id = self.docs.insert(placeholder);
            if let Some(e) = self.docs.get_mut(id) {
                e.loading = true;
            }
            self.emit(Effect::LoadFile { doc: id, path: path.clone(), eol });
            self.place_new_view(id, preview, gid);
            self.after_open(&path);
            return;
        }
        let doc = if path.exists() {
            match Document::open(&path, self.default_eol(Some(&path))) {
                Ok(mut d) => {
                    d.large_file = size > threshold && threshold > 0;
                    d
                }
                Err(e) => {
                    self.notify(Severity::Error, e.to_string());
                    return;
                }
            }
        } else {
            // A path that does not exist yet (e.g. `yace new.txt`): an empty
            // document that will be created on save.
            let mut d = Document::new_untitled(self.default_eol(Some(&path)));
            d.set_path(Some(path.clone()));
            d
        };
        let id = self.docs.insert(doc);
        self.place_new_view(id, preview, gid);
        self.after_open(&path);
    }

    fn place_new_view(&mut self, id: DocId, preview: bool, gid: GroupId) {
        // Replace the preview tab if there is one (never a dirty one).
        let replace = if preview { self.layout.active_group().preview_index() } else { None };
        match replace {
            Some(i) => {
                let old = self.layout.active_group().views[i].doc;
                let group = self.layout.active_group_mut();
                group.views[i] = View { preview: true, ..View::new(id) };
                group.activate(i);
                self.release_doc_if_unused(old);
            }
            None => self.add_view(gid, View { preview, ..View::new(id) }),
        }
    }

    /// A background load finished.
    pub(crate) fn on_file_loaded(&mut self, doc: DocId, path: PathBuf, result: Result<Box<Document>, String>) {
        let threshold = self.settings.files.large_file_threshold_mb.saturating_mul(1024 * 1024);
        let Some(entry) = self.docs.get_mut(doc) else { return };
        if !entry.loading {
            return;
        }
        match result {
            Ok(mut d) => {
                d.large_file = threshold > 0 && io::file_size(&path).unwrap_or(0) > threshold;
                entry.doc = *d;
                entry.loading = false;
                let pending = entry.pending_cursor.take();
                if let Some((line, col)) = pending
                    && let Some((g, _, active_doc)) = self.active()
                    && active_doc == doc
                    && self.layout.group(g).is_some()
                {
                    self.set_cursor_line_col(line, col);
                }
            }
            Err(e) => {
                self.notify(Severity::Error, e);
                for (g, _) in self.layout.views_of(doc) {
                    if let Some(group) = self.layout.group_mut(g)
                        && let Some(i) = group.find(doc)
                    {
                        group.remove(i);
                    }
                }
                self.release_doc_if_unused(doc);
            }
        }
        self.needs_render = true;
    }

    pub(crate) fn add_view(&mut self, gid: GroupId, view: View) {
        if let Some(group) = self.layout.group_mut(gid) {
            let at = if group.views.is_empty() { 0 } else { group.active + 1 };
            group.views.insert(at, view);
            group.activate(at);
        }
    }

    fn after_open(&mut self, path: &Path) {
        self.recent_files.retain(|p| p != path);
        self.recent_files.insert(0, path.to_path_buf());
        self.recent_files.truncate(100);
        self.focus_changed(Focus::Editor);
        self.reveal_cursor = true;
    }

    /// Drop a document if no view shows it any more.
    pub(crate) fn release_doc_if_unused(&mut self, doc: DocId) {
        if self.layout.views_of(doc).is_empty() {
            let big = self.docs.doc(doc).is_some_and(|d| d.len_chars() > 1 << 20);
            self.docs.remove(doc);
            if self.backed_up.contains_key(&doc) {
                self.backup_soon();
            }
            self.conflict_prompted.remove(&doc);
            if big {
                self.request_trim();
            }
        }
    }

    /// A new untitled editor with `text` (e.g. language server output).
    pub(crate) fn new_untitled_with(&mut self, text: &str) {
        let doc = Document::from_text(text);
        let id = self.docs.insert(doc);
        let gid = self.layout.active;
        self.add_view(gid, View::new(id));
        self.focus_changed(Focus::Editor);
    }

    pub(crate) fn new_untitled(&mut self) {
        let doc = Document::new_untitled(self.default_eol(None));
        let id = self.docs.insert(doc);
        let gid = self.layout.active;
        self.add_view(gid, View::new(id));
        self.focus_changed(Focus::Editor);
    }

    pub(crate) fn open_file_from_input(&mut self, text: &str) -> Result<(), String> {
        let path = yace_workspace::paths::expand_tilde(text);
        if path.is_dir() {
            return Err("That is a folder; use Open Folder or Add Folder to Workspace".into());
        }
        self.open_file(&path, false);
        Ok(())
    }

    // ------------------------------------------------------------------
    // Saving

    /// Save one document, applying save-time settings first.
    pub fn save_doc(&mut self, doc: DocId, force: bool) -> SaveOutcome {
        let Some(path) = self.docs.doc(doc).map(|d| d.path().map(Path::to_path_buf)) else {
            return SaveOutcome::Failed;
        };
        let Some(path) = path else {
            self.prompt_save_as(doc);
            return SaveOutcome::NeedsPath;
        };
        if !force && self.docs.doc(doc).is_some_and(|d| d.disk_state == DiskState::Conflicted) {
            self.confirm_overwrite(doc, &path);
            return SaveOutcome::Conflict;
        }
        self.apply_save_transforms(doc, &path);
        let result = self.docs.doc_mut(doc).map(|d| d.save(None, force));
        match result {
            Some(Ok(())) => {
                self.after_save(doc, &path);
                self.backup_soon();
                SaveOutcome::Saved
            }
            Some(Err(yace_buffer::io::SaveError::Conflict { .. })) => {
                self.confirm_overwrite(doc, &path);
                SaveOutcome::Conflict
            }
            Some(Err(e)) => {
                self.notify(Severity::Error, e.to_string());
                SaveOutcome::Failed
            }
            None => SaveOutcome::Failed,
        }
    }

    fn apply_save_transforms(&mut self, doc: DocId, path: &Path) {
        let s = self.settings_for(Some(path)).files.clone();
        if let Some(d) = self.docs.doc(doc) {
            if d.large_file {
                return;
            }
            if s.trim_trailing_whitespace {
                let cs = ops::trim_trailing_whitespace(d.rope());
                self.apply_doc_changes(doc, cs);
            }
        }
        if s.insert_final_newline
            && let Some(d) = self.docs.doc(doc)
        {
            let cs = ops::ensure_final_newline(d.rope(), d.eol());
            self.apply_doc_changes(doc, cs);
        }
    }

    fn after_save(&mut self, doc: DocId, path: &Path) {
        self.conflict_prompted.remove(&doc);
        self.plugin_event("documentSaved", serde_json::json!({ "doc": doc, "path": path.display().to_string() }));
        if self.settings.git.auto_refresh
            && let Some((repo, _)) = self.git_repo_of(path)
        {
            self.git_schedule(repo, super::git::STATUS_DEBOUNCE);
        }
        self.lsp_did_save(doc);
        if let Some(parent) = path.parent() {
            self.explorer.refresh_dir(parent);
        }
        let key = path_key(path);
        if self.on_config_files_changed(std::slice::from_ref(&path.to_path_buf())) {
            // Settings, keybindings or a theme: applied already.
        } else if self.workspace.file().is_some_and(|f| path_key(f) == key) {
            self.notify(Severity::Info, "Workspace file saved; reopen the workspace to apply folder changes");
        }
        // Continue a pending close flow.
        if self.suppress_flow {
            if let Some(f) = self.close_flow.as_mut() {
                f.pending.retain(|d| *d != doc);
                f.saving = None;
            }
        } else if self.close_flow.as_ref().is_some_and(|f| f.pending.front() == Some(&doc) || f.saving == Some(doc)) {
            if let Some(f) = self.close_flow.as_mut() {
                f.pending.retain(|d| *d != doc);
                f.saving = None;
            }
            self.continue_close();
        }
    }

    fn confirm_overwrite(&mut self, doc: DocId, path: &Path) {
        self.open_overlay(Overlay::Confirm(Confirm {
            title: "File changed on disk".into(),
            message: vec![
                format!("{} has been modified outside YACE since it was opened.", path.display()),
                "Overwriting will replace the version on disk with yours.".into(),
            ],
            buttons: vec![
                ("Overwrite".into(), ConfirmAction::OverwriteSave(doc)),
                ("Cancel".into(), ConfirmAction::Nothing),
            ],
            selected: 1,
            cancel: ConfirmAction::Nothing,
        }));
    }

    pub(crate) fn prompt_save_as(&mut self, doc: DocId) {
        let current = self.docs.doc(doc).and_then(|d| d.path().map(|p| p.display().to_string()));
        let default = current.unwrap_or_else(|| {
            let dir = self
                .workspace
                .roots()
                .iter()
                .find(|r| r.is_available())
                .map(|r| r.path.clone())
                .unwrap_or_else(|| std::env::current_dir().unwrap_or_default());
            format!("{}/{}", dir.display(), self.docs.name(doc))
        });
        self.open_overlay(Overlay::Prompt(Prompt {
            title: "Save As".into(),
            hint: Some("Path of the file to write (Tab completes)".into()),
            input: LineInput::new(default),
            action: PromptAction::SaveAs { doc },
            path_completion: true,
            error: None,
        }));
    }

    pub(crate) fn save_as(&mut self, doc: DocId, text: &str) -> Result<(), String> {
        if text.is_empty() {
            return Err("Enter a file path".into());
        }
        let path = yace_workspace::paths::absolutize(&yace_workspace::paths::expand_tilde(text));
        if path.is_dir() {
            return Err("That path is a folder".into());
        }
        let same = self.docs.doc(doc).and_then(|d| d.path()).is_some_and(|p| p == path);
        if path.exists() && !same {
            self.open_overlay(Overlay::Confirm(Confirm {
                title: "Replace existing file?".into(),
                message: vec![format!("{} already exists.", path.display())],
                buttons: vec![
                    ("Replace".into(), ConfirmAction::OverwriteExisting { doc, path: path.clone() }),
                    ("Cancel".into(), ConfirmAction::Nothing),
                ],
                selected: 1,
                cancel: ConfirmAction::Nothing,
            }));
            return Ok(());
        }
        if let Some(parent) = path.parent()
            && !parent.exists()
        {
            self.open_overlay(Overlay::Confirm(Confirm {
                title: "Create folder?".into(),
                message: vec![format!("{} does not exist. Create it?", parent.display())],
                buttons: vec![
                    ("Create and Save".into(), ConfirmAction::CreateParentAndSave { doc, path: path.clone() }),
                    ("Cancel".into(), ConfirmAction::Nothing),
                ],
                selected: 0,
                cancel: ConfirmAction::Nothing,
            }));
            return Ok(());
        }
        self.save_as_confirmed(doc, &path);
        Ok(())
    }

    pub(crate) fn save_as_confirmed(&mut self, doc: DocId, path: &Path) {
        self.apply_save_transforms(doc, path);
        let result = self.docs.doc_mut(doc).map(|d| {
            let same = d.path() == Some(path);
            d.save(Some(path), !same)
        });
        match result {
            Some(Ok(())) => {
                self.docs.rekey(doc);
                self.after_save(doc, path);
            }
            Some(Err(e)) => self.notify(Severity::Error, e.to_string()),
            None => {}
        }
    }

    pub(crate) fn save_all(&mut self) {
        let dirty: Vec<DocId> =
            self.docs.ids().into_iter().filter(|d| self.docs.doc(*d).is_some_and(Document::is_dirty)).collect();
        for d in dirty {
            if self.save_doc(d, false) != SaveOutcome::Saved {
                break;
            }
        }
    }

    pub(crate) fn autosave_all(&mut self) {
        let ids: Vec<DocId> = self
            .docs
            .ids()
            .into_iter()
            .filter(|d| {
                self.docs
                    .doc(*d)
                    .is_some_and(|x| x.is_dirty() && x.path().is_some() && x.disk_state == DiskState::InSync)
            })
            .collect();
        for d in ids {
            self.save_doc(d, false);
        }
    }

    pub(crate) fn revert_active(&mut self) {
        if let Some((_, _, d)) = self.active() {
            self.reload_doc(d);
        }
    }

    pub(crate) fn reload_doc(&mut self, doc: DocId) {
        let sel = self
            .layout
            .views_of(doc)
            .first()
            .and_then(|(g, v)| self.layout.group(*g).map(|gr| gr.views[*v].sel.clone()))
            .unwrap_or_default();
        let res = self.docs.doc_mut(doc).map(|d| d.reload(&sel));
        match res {
            Some(Ok(_)) => {
                self.backup_soon();
                // Keep every view's cursor where it was (clamped) rather than
                // mapping it through the whole-document replacement.
                let len = self.docs.doc(doc).map(|d| d.len_chars()).unwrap_or(0);
                for v in self.layout.all_views_mut().filter(|v| v.doc == doc) {
                    v.sel = v.sel.clamp(len);
                }
            }
            Some(Err(e)) => self.notify(Severity::Error, e.to_string()),
            None => {}
        }
    }

    // ------------------------------------------------------------------
    // Closing and the unsaved-changes flow

    /// Close views of `docs` in group `gid`, asking about unsaved changes
    /// for documents that would lose their last view.
    pub(crate) fn close_views(&mut self, gid: GroupId, docs: Vec<DocId>) {
        let list: Vec<(GroupId, DocId)> = docs.iter().map(|d| (gid, *d)).collect();
        let losing: Vec<DocId> =
            docs.iter().copied().filter(|d| self.layout.views_of(*d).iter().all(|(g, _)| *g == gid)).collect();
        self.start_close(losing, AfterClose::CloseViews(list));
    }

    pub(crate) fn start_close(&mut self, docs: Vec<DocId>, after: AfterClose) {
        let pending: VecDeque<DocId> =
            docs.into_iter().filter(|d| self.docs.doc(*d).is_some_and(Document::is_dirty)).collect();
        self.close_flow = Some(CloseFlow { pending, after, saving: None });
        self.continue_close();
    }

    pub(crate) fn continue_close(&mut self) {
        let Some(flow) = self.close_flow.as_ref() else { return };
        if let Some(&doc) = flow.pending.front() {
            let name = self.docs.name(doc);
            let many = flow.pending.len() > 1;
            let mut buttons = vec![
                ("Save".to_string(), ConfirmAction::Close(CloseChoice::Save)),
                ("Don't Save".to_string(), ConfirmAction::Close(CloseChoice::DontSave)),
            ];
            if many {
                buttons.push(("All: Save".to_string(), ConfirmAction::Close(CloseChoice::SaveAll)));
            }
            buttons.push(("Cancel".to_string(), ConfirmAction::Close(CloseChoice::Cancel)));
            self.open_overlay(Overlay::Confirm(Confirm {
                title: "Unsaved changes".into(),
                message: vec![
                    format!("Do you want to save the changes you made to {name}?"),
                    "Your changes will be lost if you don't save them.".into(),
                ],
                buttons,
                selected: 0,
                cancel: ConfirmAction::Close(CloseChoice::Cancel),
            }));
            return;
        }
        let after = self.close_flow.take().map(|f| f.after);
        if let Some(after) = after {
            self.finish_close(after);
        }
    }

    pub(crate) fn on_close_choice(&mut self, choice: CloseChoice) {
        let Some(doc) = self.close_flow.as_ref().and_then(|f| f.pending.front().copied()) else {
            self.close_flow = None;
            return;
        };
        match choice {
            CloseChoice::Cancel => self.close_flow = None,
            CloseChoice::DontSave => {
                if let Some(f) = self.close_flow.as_mut() {
                    f.pending.pop_front();
                }
                self.continue_close();
            }
            CloseChoice::Save => match self.save_doc(doc, false) {
                // after_save advanced the flow.
                SaveOutcome::Saved => {}
                SaveOutcome::NeedsPath => {
                    if let Some(f) = self.close_flow.as_mut() {
                        f.saving = Some(doc);
                    }
                }
                // The overwrite confirmation continues via after_save.
                SaveOutcome::Conflict => {}
                SaveOutcome::Failed => self.close_flow = None,
            },
            CloseChoice::SaveAll => {
                self.suppress_flow = true;
                while let Some(doc) = self.close_flow.as_ref().and_then(|f| f.pending.front().copied()) {
                    match self.save_doc(doc, false) {
                        SaveOutcome::Saved => {}
                        SaveOutcome::NeedsPath => {
                            if let Some(f) = self.close_flow.as_mut() {
                                f.saving = Some(doc);
                            }
                            self.suppress_flow = false;
                            return;
                        }
                        SaveOutcome::Conflict => {
                            self.suppress_flow = false;
                            return;
                        }
                        SaveOutcome::Failed => {
                            self.suppress_flow = false;
                            self.close_flow = None;
                            return;
                        }
                    }
                }
                self.suppress_flow = false;
                self.continue_close();
            }
        }
    }

    fn finish_close(&mut self, after: AfterClose) {
        match after {
            AfterClose::Quit => {
                self.save_session();
                self.backups_close();
                self.kill_all_terminals();
                self.quit = true;
            }
            AfterClose::CloseViews(list) => {
                for (gid, doc) in list {
                    self.remove_view(gid, doc);
                }
            }
            AfterClose::CloseGroup(gid) => {
                let docs: Vec<DocId> =
                    self.layout.group(gid).map(|g| g.views.iter().map(|v| v.doc).collect()).unwrap_or_default();
                for d in docs {
                    self.remove_view(gid, d);
                }
                self.layout.close_group(gid);
            }
            AfterClose::SwitchWorkspace(target) => self.switch_workspace_now(target),
        }
    }

    /// Close every view of a document that has no unsaved changes
    /// (generated documents).
    pub(crate) fn close_doc_views(&mut self, doc: DocId) {
        for (g, _) in self.layout.views_of(doc) {
            self.remove_view(g, doc);
        }
    }

    fn remove_view(&mut self, gid: GroupId, doc: DocId) {
        let path = self.docs.doc(doc).and_then(|d| d.path().map(Path::to_path_buf));
        let Some(group) = self.layout.group_mut(gid) else { return };
        let Some(i) = group.find(doc) else { return };
        group.remove(i);
        if let Some(path) = path {
            self.closed_tabs.push(ClosedTab { path, group: gid });
            if self.closed_tabs.len() > 50 {
                self.closed_tabs.remove(0);
            }
        }
        self.release_doc_if_unused(doc);
        // Closing the last tab of a non-primary group closes the group.
        if self.layout.group(gid).is_some_and(|g| g.views.is_empty()) && self.layout.groups().len() > 1 {
            self.layout.close_group(gid);
        }
        if self.layout.active_group().views.is_empty() && self.layout.groups().len() == 1 {
            self.focus = Focus::Explorer;
        }
    }

    pub(crate) fn close_active_editor(&mut self) {
        if let Some((g, _, d)) = self.active() {
            self.close_views(g, vec![d]);
        } else if self.layout.groups().len() > 1 {
            let g = self.layout.active;
            self.layout.close_group(g);
        }
    }

    pub(crate) fn reopen_closed(&mut self) {
        while let Some(t) = self.closed_tabs.pop() {
            if t.path.exists() {
                if self.layout.group(t.group).is_some() {
                    self.layout.active = t.group;
                }
                self.open_file(&t.path, false);
                return;
            }
        }
        self.notify(Severity::Info, "No recently closed editors");
    }

    pub(crate) fn quit_requested(&mut self) {
        if self.confirm_running_terminals(AfterClose::Quit) {
            return;
        }
        let docs = self.docs.ids();
        self.start_close(docs, AfterClose::Quit);
    }

    // ------------------------------------------------------------------
    // External modification (spec §9.10)

    pub(crate) fn process_fs_changes(&mut self, paths: Vec<PathBuf>) {
        // Events arrive under the watched (canonical) directory; report them
        // under every path that directory is shown as (symlinked roots).
        let mut paths = paths;
        let mut extra = Vec::new();
        for p in &paths {
            for (canon, shown) in &self.watch_alias {
                if let Ok(rel) = p.strip_prefix(canon) {
                    extra.extend(shown.iter().filter(|s| *s != canon).map(|s| s.join(rel)));
                }
            }
        }
        paths.extend(extra);
        self.on_config_files_changed(&paths);
        self.git_paths_changed(&paths);
        let mut dirs: Vec<PathBuf> = Vec::new();
        for p in &paths {
            if let Some(parent) = p.parent() {
                dirs.push(parent.to_path_buf());
            }
            dirs.push(p.clone());
        }
        dirs.sort();
        dirs.dedup();
        for d in &dirs {
            self.explorer.refresh_dir(d);
        }
        self.index_fs_changes(&paths);
        self.search_queue_paths(paths.iter().cloned());
        self.check_open_docs_on_disk();
        self.needs_render = true;
    }

    /// Compare every open document with the disk (cheap metadata check).
    pub(crate) fn check_open_docs_on_disk(&mut self) {
        let ids = self.docs.ids();
        for id in ids {
            if self.docs.is_loading(id) {
                continue;
            }
            let Some(doc) = self.docs.doc(id) else { continue };
            let Some(path) = doc.path().map(Path::to_path_buf) else { continue };
            if doc.disk.is_none() && !path.exists() {
                continue; // never-saved new file
            }
            let exists = path.exists();
            if !exists {
                if doc.disk_state != DiskState::Deleted {
                    if let Some(d) = self.docs.doc_mut(id) {
                        d.disk_state = DiskState::Deleted;
                    }
                    self.notify(
                        Severity::Warning,
                        format!("{} was deleted on disk; the editor keeps your copy", path.display()),
                    );
                }
                continue;
            }
            let changed = io::changed_on_disk(&path, doc.disk.as_ref()).unwrap_or(false);
            let was_deleted = doc.disk_state == DiskState::Deleted;
            if !changed && !was_deleted {
                continue;
            }
            if !doc.is_dirty() {
                self.reload_doc(id);
            } else if doc.disk_state != DiskState::Conflicted && !self.conflict_prompted.contains(&id) {
                self.conflict_prompted.insert(id);
                let name = self.docs.name(id);
                self.open_overlay(Overlay::Confirm(Confirm {
                    title: "File changed on disk".into(),
                    message: vec![
                        format!("{name} was changed outside YACE, and you have unsaved changes."),
                        format!("({})", path.display()),
                    ],
                    buttons: vec![
                        ("Keep mine".into(), ConfirmAction::KeepMine(id)),
                        ("Reload (discard my changes)".into(), ConfirmAction::ReloadFromDisk(id)),
                    ],
                    selected: 0,
                    cancel: ConfirmAction::KeepMine(id),
                }));
            }
        }
    }

    /// Paths of documents under `old` follow a rename/move to `new`.
    pub(crate) fn retarget_docs(&mut self, old: &Path, new: &Path) {
        let ids = self.docs.ids();
        for id in ids {
            let Some(p) = self.docs.doc(id).and_then(|d| d.path().map(Path::to_path_buf)) else { continue };
            if let Ok(rest) = p.strip_prefix(old) {
                let np = if rest.as_os_str().is_empty() { new.to_path_buf() } else { new.join(rest) };
                if let Some(d) = self.docs.doc_mut(id) {
                    d.set_path(Some(np));
                }
                self.docs.rekey(id);
            }
        }
    }

    pub(crate) fn emit_title(&mut self) {
        let title = format!("\x1b]2;{} — YACE\x07", self.workspace.name());
        self.emit(Effect::Write(title.into_bytes()));
    }
}
