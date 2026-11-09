//! Crash recovery: unsaved documents are backed up and reopened after YACE
//! ended without a clean quit.

mod common;

use std::{
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

use common::Harness;
use yace_app::{
    cli::{Launch, Target},
    initial_workspace,
};

fn open_folder(h: &mut Harness, dir: &Path) {
    initial_workspace(
        &mut h.app,
        &Launch { target: Target::Folder(dir.to_path_buf()), add_roots: vec![], goto: None, restore: false },
    );
    h.settle();
}

/// Run the backup timer now.
fn back_up(h: &mut Harness) {
    h.app.on_tick(Instant::now() + Duration::from_secs(2));
    h.settle();
}

fn backups_root(h: &Harness) -> PathBuf {
    h.paths().state_dir.join("backups")
}

/// Every backup folder (one per workspace and process).
fn backup_dirs(h: &Harness) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for ws in std::fs::read_dir(backups_root(h)).into_iter().flatten().flatten() {
        out.extend(std::fs::read_dir(ws.path()).into_iter().flatten().flatten().map(|e| e.path()));
    }
    out
}

fn backup_files(h: &Harness) -> usize {
    backup_dirs(h).iter().map(|d| std::fs::read_dir(d).map(|r| r.count()).unwrap_or(0)).sum()
}

/// Make the backups look like they belong to a process that has died (this
/// one is still alive), then start a new YACE on the same state.
fn crash_and_restart(h: Harness, root: &Path, as_pid: u32) -> Harness {
    for d in backup_dirs(&h) {
        std::fs::rename(&d, d.with_file_name(as_pid.to_string())).unwrap();
    }
    let Harness { state, .. } = h;
    let mut h = Harness::with_state(state);
    open_folder(&mut h, root);
    h
}

fn notes(h: &Harness) -> String {
    h.app.notifications.iter().map(|n| n.text.clone()).collect::<Vec<_>>().join("\n")
}

fn texts(h: &Harness) -> Vec<(Option<String>, String, bool)> {
    let mut v: Vec<_> = h
        .app
        .docs
        .ids()
        .into_iter()
        .filter_map(|d| h.app.docs.doc(d))
        .map(|d| (d.path().map(|p| p.file_name().unwrap().to_string_lossy().into_owned()), d.text(), d.is_dirty()))
        .collect();
    v.sort();
    v
}

const DEAD: u32 = u32::MAX - 7;

#[test]
fn unsaved_files_and_untitled_documents_come_back_after_a_crash() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("proj");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("a.txt"), "one\n").unwrap();
    std::fs::write(root.join("b.txt"), "saved\n").unwrap();
    let mut h = Harness::new();
    open_folder(&mut h, &root);
    h.app.open_file(&root.join("a.txt"), false);
    h.settle();
    h.type_text("two ");
    h.app.execute("workbench.action.files.newUntitledFile", None);
    h.type_text("draft");
    h.app.open_file(&root.join("b.txt"), false);
    h.settle();
    assert_eq!(backup_files(&h), 0, "nothing is written right away");
    back_up(&mut h);
    assert_eq!(backup_files(&h), 2, "the two unsaved documents; b.txt is saved");

    let mut h = crash_and_restart(h, &root, DEAD);
    assert!(notes(&h).contains("Restored 2 unsaved files"), "{}", notes(&h));
    assert_eq!(
        texts(&h),
        vec![(None, "draft".into(), true), (Some("a.txt".into()), "two one\n".into(), true)],
        "a.txt has its unsaved text, the untitled draft is back"
    );
    assert_eq!(std::fs::read_to_string(root.join("a.txt")).unwrap(), "one\n", "the disk is untouched");
    assert!(backup_dirs(&h).iter().all(|d| d.file_name().unwrap() == std::process::id().to_string().as_str()));
    assert_eq!(backup_files(&h), 2, "backed up again under this process");

    // Undo shows the file as it is on disk.
    h.app.open_file(&root.join("a.txt"), false);
    h.settle();
    h.key("ctrl+z");
    assert_eq!(h.active_text(), "one\n");
    h.key("ctrl+y");
    // Saving removes its backup.
    h.key("ctrl+s");
    h.settle();
    back_up(&mut h);
    assert_eq!(backup_files(&h), 1);
    assert_eq!(std::fs::read_to_string(root.join("a.txt")).unwrap(), "two one\n");
}

#[test]
fn a_file_changed_on_disk_since_is_flagged_and_left_alone() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("proj");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("a.txt"), "one\n").unwrap();
    let mut h = Harness::new();
    open_folder(&mut h, &root);
    h.app.open_file(&root.join("a.txt"), false);
    h.settle();
    h.type_text("mine ");
    back_up(&mut h);
    std::fs::write(root.join("a.txt"), "changed elsewhere\n").unwrap();
    let h = crash_and_restart(h, &root, DEAD);
    assert!(notes(&h).contains("Changed on disk since: a.txt"), "{}", notes(&h));
    assert_eq!(texts(&h), vec![(Some("a.txt".into()), "mine one\n".into(), true)]);
    assert_eq!(std::fs::read_to_string(root.join("a.txt")).unwrap(), "changed elsewhere\n");
}

#[test]
fn a_clean_quit_or_dont_save_leaves_nothing_and_live_windows_keep_theirs() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("proj");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("a.txt"), "one\n").unwrap();
    let mut h = Harness::new();
    open_folder(&mut h, &root);
    h.app.open_file(&root.join("a.txt"), false);
    h.settle();
    h.type_text("x");
    back_up(&mut h);
    assert_eq!(backup_files(&h), 1);
    // Close the tab with "Don't Save": its backup goes.
    h.key("ctrl+w");
    h.key("d"); // "Don't Save"
    h.settle();
    back_up(&mut h);
    assert_eq!(backup_files(&h), 0);

    // Quit after "Don't Save": the folder goes too.
    h.app.open_file(&root.join("a.txt"), false);
    h.settle();
    h.type_text("y");
    back_up(&mut h);
    assert_eq!(backup_files(&h), 1);
    h.key("ctrl+q");
    h.key("d");
    h.settle();
    assert!(h.app.quit);
    assert!(backup_dirs(&h).is_empty(), "{:?}", backup_dirs(&h));

    // Backups of a window that is still open (pid 1 is always alive) are
    // not taken over.
    let mut h = Harness::new();
    open_folder(&mut h, &root);
    h.app.open_file(&root.join("a.txt"), false);
    h.settle();
    h.type_text("z");
    back_up(&mut h);
    let h = crash_and_restart(h, &root, 1);
    assert!(!notes(&h).contains("Restored"), "{}", notes(&h));
    assert!(texts(&h).iter().all(|(_, _, dirty)| !dirty));
}
