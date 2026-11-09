//! Crash recovery: unsaved documents are backed up a second after an edit
//! (`crate::backup`) and reopened, still unsaved, after YACE ended without
//! a clean quit (a crash, a kill, a closed terminal).

use std::{
    collections::HashSet,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

use yace_buffer::{Change, ChangeSet, Document, Rope};

use super::{App, Effect, Severity};
use crate::{
    backup,
    editor::{DocId, View},
};

/// How long after an edit its backup is written (at most once per second
/// while typing).
pub const BACKUP_DELAY: Duration = Duration::from_secs(1);

/// One document to back up: its key, path, text and the disk state its
/// edits started from.
#[derive(Debug, Clone)]
pub struct BackupWrite {
    pub key: String,
    pub path: Option<PathBuf>,
    pub rope: Rope,
    pub disk: Option<(u64, u64)>,
}

/// Backups to write and delete, in order, on the backup worker.
#[derive(Debug, Clone)]
pub struct BackupJob {
    pub dir: PathBuf,
    pub writes: Vec<BackupWrite>,
    pub removes: Vec<String>,
    /// Remove the whole folder afterwards (clean quit).
    pub clear: bool,
}

impl BackupJob {
    pub fn run(self) {
        for key in &self.removes {
            backup::remove(&self.dir, key);
        }
        for w in self.writes {
            let b = backup::Backup { path: w.path, text: w.rope.to_string(), disk: w.disk };
            if let Err(e) = backup::write(&self.dir, &w.key, &b) {
                tracing::warn!("could not back up unsaved changes to {}: {e}", self.dir.display());
            }
        }
        if self.clear {
            backup::clear(&self.dir);
        }
    }
}

impl App {
    /// The workspace backups are filed under: its identity, or one shared
    /// name for windows without a folder (`yace file.js`).
    fn backup_workspace(&self) -> String {
        if self.workspace.roots().is_empty() { "no-folder".into() } else { self.workspace.identity() }
    }

    /// Write backups soon (after an edit, a save, a close).
    pub(crate) fn backup_soon(&mut self) {
        if self.backup_deadline.is_none() {
            self.backup_deadline = Some(Instant::now() + BACKUP_DELAY);
        }
    }

    /// Bring the backups in line with the documents: write the unsaved
    /// ones that changed since, delete the ones saved or closed.
    pub(crate) fn sync_backups(&mut self) {
        self.backup_deadline = None;
        let Some(dir) = self.backup_dir.clone() else { return };
        let mut writes = Vec::new();
        let mut keep = HashSet::new();
        for d in self.docs.ids() {
            if self.docs.get(d).is_none_or(|e| e.loading) {
                continue;
            }
            let Some(doc) = self.docs.doc(d) else { continue };
            if !doc.is_dirty() {
                continue;
            }
            keep.insert(d);
            if self.backed_up.get(&d) == Some(&doc.version()) {
                continue;
            }
            self.backed_up.insert(d, doc.version());
            let disk = doc.disk.as_ref().map(|s| (s.len, s.hash));
            writes.push(BackupWrite {
                key: d.to_string(),
                path: doc.path().map(Path::to_path_buf),
                rope: doc.rope().clone(),
                disk,
            });
        }
        let removes: Vec<String> = self.backed_up.keys().filter(|d| !keep.contains(*d)).map(DocId::to_string).collect();
        self.backed_up.retain(|d, _| keep.contains(d));
        if !writes.is_empty() || !removes.is_empty() {
            self.effects.push(Effect::Backups(BackupJob { dir, writes, removes, clear: false }));
        }
    }

    /// The workspace closes cleanly (quit or switch, after the unsaved
    /// changes prompt): its backups aren't needed any more.
    pub(crate) fn backups_close(&mut self) {
        self.backup_deadline = None;
        self.backed_up.clear();
        if let Some(dir) = self.backup_dir.take() {
            self.effects.push(Effect::Backups(BackupJob { dir, writes: Vec::new(), removes: Vec::new(), clear: true }));
        }
    }

    /// When a workspace opens: reopen what a YACE that ended abnormally
    /// left unsaved in it. Files come back with the unsaved text as an
    /// undoable edit (undo shows the file as it is on disk); the disk is
    /// never written. Untitled documents come back untitled.
    pub(crate) fn recover_backups(&mut self) {
        let workspace = self.backup_workspace();
        self.backup_dir = Some(backup::process_dir(&self.paths.state_dir, &workspace, std::process::id()));
        let mut restored = 0;
        let mut changed: Vec<String> = Vec::new();
        for dir in backup::orphans(&self.paths.state_dir, &workspace) {
            for b in backup::read_all(&dir) {
                match &b.path {
                    Some(p) if p.is_file() => {
                        let Some(d) = self.doc_for_recovery(p) else {
                            // Unreadable now: keep the text in an untitled tab.
                            self.restore_untitled(&b.text);
                            restored += 1;
                            continue;
                        };
                        let rope = self.docs.doc(d).map(|x| x.rope().clone()).unwrap_or_default();
                        if rope == b.text.as_str() {
                            continue;
                        }
                        let now = yace_buffer::io::current_snapshot(p).ok().flatten().map(|s| (s.len, s.hash));
                        if b.disk.is_some() && now != b.disk {
                            changed.push(p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default());
                        }
                        let all = rope.len_chars();
                        self.apply_doc_changes(d, ChangeSet::new(vec![Change::replace(0, all, b.text.clone())], all));
                        restored += 1;
                    }
                    // Deleted since, or never saved.
                    _ => {
                        self.restore_untitled(&b.text);
                        restored += 1;
                    }
                }
            }
            backup::clear(&dir);
        }
        if restored == 0 {
            return;
        }
        let mut msg = format!(
            "Restored {restored} unsaved {} from when YACE last closed unexpectedly",
            if restored == 1 { "file" } else { "files" }
        );
        if !changed.is_empty() {
            msg.push_str(&format!(
                ". Changed on disk since: {} (your unsaved text is shown; Undo shows the file on disk)",
                changed.join(", ")
            ));
        }
        self.notify_sticky(Severity::Info, msg);
        // Back them up again under this process.
        self.sync_documents();
        self.sync_backups();
    }

    /// A new untitled tab holding `text` as an unsaved edit (so quitting
    /// asks about it, and it is backed up again).
    fn restore_untitled(&mut self, text: &str) {
        self.new_untitled();
        if let Some((_, _, d)) = self.active() {
            self.apply_doc_changes(d, ChangeSet::new(vec![Change::replace(0, 0, text.to_string())], 0));
        }
    }

    /// The open document for `path`, or the file opened in a new tab (read
    /// now, even if it's big, so the backup text can replace it).
    fn doc_for_recovery(&mut self, path: &Path) -> Option<DocId> {
        if let Some(d) = self.docs.find_by_path(path)
            && self.docs.get(d).is_some_and(|e| !e.loading)
        {
            return Some(d);
        }
        let doc = Document::open(path, self.default_eol(Some(path))).ok()?;
        let id = self.docs.insert(doc);
        let gid = self.layout.active;
        self.add_view(gid, View::new(id));
        Some(id)
    }
}
