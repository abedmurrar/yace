//! Backups of unsaved documents, for recovery after a crash, a kill or a
//! closed terminal.
//!
//! Each process writes the dirty documents of its workspace to
//! `<state>/backups/<workspace hash>/<pid>/<key>.json` about a second after
//! an edit, and deletes them once they are saved or closed. A clean quit
//! removes the folder. A folder whose process no longer exists holds what a
//! YACE that ended abnormally hadn't saved: the next start reopens those
//! documents with their unsaved text. Nothing on disk is ever overwritten.

use std::{
    fs, io,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};

/// One unsaved document.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Backup {
    /// The file it edits; `None` for an untitled document.
    pub path: Option<PathBuf>,
    /// Its full unsaved text.
    pub text: String,
    /// Length and hash of the file on disk the edits started from (to tell
    /// whether it changed since).
    pub disk: Option<(u64, u64)>,
}

/// The folder of this process's backups for a workspace.
pub fn process_dir(state_dir: &Path, workspace: &str, pid: u32) -> PathBuf {
    workspace_dir(state_dir, workspace).join(pid.to_string())
}

fn workspace_dir(state_dir: &Path, workspace: &str) -> PathBuf {
    // FNV-1a, as the session files use.
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in workspace.as_bytes() {
        h ^= u64::from(*b);
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    state_dir.join("backups").join(format!("{h:016x}"))
}

/// Write one backup (only the owner can read it).
pub fn write(dir: &Path, key: &str, b: &Backup) -> io::Result<()> {
    use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
    fs::DirBuilder::new().recursive(true).mode(0o700).create(dir)?;
    let tmp = dir.join(format!(".{key}.tmp"));
    {
        let mut f = fs::OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).open(&tmp)?;
        serde_json::to_writer(&mut f, b).map_err(io::Error::other)?;
        f.sync_all()?;
    }
    fs::rename(&tmp, dir.join(format!("{key}.json")))
}

pub fn remove(dir: &Path, key: &str) {
    let _ = fs::remove_file(dir.join(format!("{key}.json")));
}

/// Remove a process's backups (clean quit, or once recovered).
pub fn clear(dir: &Path) {
    let _ = fs::remove_dir_all(dir);
}

/// Backup folders of `workspace` left by processes that no longer exist.
pub fn orphans(state_dir: &Path, workspace: &str) -> Vec<PathBuf> {
    let Ok(rd) = fs::read_dir(workspace_dir(state_dir, workspace)) else { return Vec::new() };
    let mut out: Vec<PathBuf> = rd
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .and_then(|n| n.parse::<u32>().ok())
                .is_some_and(|pid| pid != std::process::id() && !Path::new(&format!("/proc/{pid}")).exists())
        })
        .collect();
    out.sort();
    out
}

/// The backups in a folder (unreadable ones are skipped).
pub fn read_all(dir: &Path) -> Vec<Backup> {
    let Ok(rd) = fs::read_dir(dir) else { return Vec::new() };
    let mut files: Vec<PathBuf> =
        rd.flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|x| x == "json")).collect();
    files.sort();
    files.iter().filter_map(|p| serde_json::from_slice(&fs::read(p).ok()?).ok()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn write_read_orphans_clear() {
        let state = tempfile::tempdir().unwrap();
        let mine = process_dir(state.path(), "ws", std::process::id());
        let b = Backup { path: Some("/x/a.txt".into()), text: "unsaved\n".into(), disk: Some((3, 42)) };
        write(&mine, "1", &b).unwrap();
        write(&mine, "2", &Backup { path: None, text: "draft".into(), disk: None }).unwrap();
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(fs::metadata(mine.join("1.json")).unwrap().permissions().mode() & 0o777, 0o600);
        assert_eq!(fs::metadata(&mine).unwrap().permissions().mode() & 0o777, 0o700);
        assert_eq!(read_all(&mine).len(), 2);
        // A live process's backups (ours) are not orphans; a dead one's are.
        assert!(orphans(state.path(), "ws").is_empty());
        let dead = process_dir(state.path(), "ws", u32::MAX - 1);
        write(&dead, "1", &b).unwrap();
        assert_eq!(orphans(state.path(), "ws"), vec![dead.clone()]);
        assert_eq!(read_all(&dead), vec![b]);
        assert!(orphans(state.path(), "other").is_empty(), "per workspace");
        remove(&mine, "2");
        assert_eq!(read_all(&mine).len(), 1);
        clear(&dead);
        assert!(orphans(state.path(), "ws").is_empty());
    }
}
