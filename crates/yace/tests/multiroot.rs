//! Multi-root workspace and editing integration tests (spec §28.2, §28.4, §28.5).

mod common;

use std::fs;

use common::{Harness, multi_root};
use yace_app::{
    app::Focus,
    cli::{Launch, Target},
    initial_workspace,
    overlay::Overlay,
};

fn open_ws(h: &mut Harness, ws: &std::path::Path) {
    let launch =
        Launch { target: Target::WorkspaceFile(ws.to_path_buf()), add_roots: vec![], goto: None, restore: true };
    initial_workspace(&mut h.app, &launch);
    h.settle();
}

#[test]
fn every_root_appears_and_missing_root_does_not_block() {
    let m = multi_root();
    let mut h = Harness::new();
    open_ws(&mut h, &m.ws_file);
    let labels = h.explorer_labels();
    for name in ["Global API", "Learn App", "Chat API", "Launcher"] {
        assert!(labels.iter().any(|l| l == name), "{name} missing from {labels:?}");
    }
    // Roots are expanded and their children loaded independently.
    assert!(labels.iter().any(|l| l.trim() == "next.config.ts"));
    assert!(labels.iter().any(|l| l.trim() == "README.md"), "relative root resolved against the workspace file");
    let screen = h.screen();
    assert!(screen.contains("GLOBAL API"));
    assert!(screen.contains("LAUNCHER"));
    assert!(screen.contains("missing") || screen.contains("not found"), "{screen}");
    // Missing root reported as a notification.
    assert!(h.app.notification_log.iter().any(|(_, t)| t.contains("Launcher")));
}

#[test]
fn open_files_across_roots_with_disambiguated_tabs() {
    let m = multi_root();
    let mut h = Harness::new();
    open_ws(&mut h, &m.ws_file);
    h.app.open_file(&m.api.join("src/index.ts"), false);
    h.app.open_file(&m.web.join("src/index.ts"), false);
    h.settle();
    h.app.notifications.clear();
    let screen = h.screen();
    assert!(screen.contains("index.ts — Global API / src"), "{screen}");
    assert!(screen.contains("index.ts — Learn App / src"), "{screen}");
    assert_eq!(h.active_text(), "export const web = 2;\n");
    // Status bar shows the owning root.
    assert!(screen.contains("Learn App › src/index.ts"), "{screen}");
}

#[test]
fn edit_save_and_bytes_on_disk() {
    let m = multi_root();
    let mut h = Harness::new();
    open_ws(&mut h, &m.ws_file);
    let file = m.chat.join("src/index.ts");
    h.app.open_file(&file, false);
    h.key("ctrl+end");
    h.type_text("hello({");
    h.key("enter");
    h.type_text("x");
    h.key("ctrl+s");
    assert_eq!(fs::read_to_string(&file).unwrap(), "chat\nhello({\n    x\n})");
    let (_, _, d) = h.app.active().unwrap();
    assert!(!h.app.docs.doc(d).unwrap().is_dirty());
    // Undo restores, redo reapplies.
    h.key("ctrl+z");
    assert!(h.app.docs.doc(d).unwrap().is_dirty());
    h.key("ctrl+y");
    assert!(!h.app.docs.doc(d).unwrap().is_dirty());
}

#[test]
fn root_settings_apply_per_repository() {
    let m = multi_root();
    let mut h = Harness::new();
    open_ws(&mut h, &m.ws_file);
    h.app.open_file(&m.web.join("next.config.ts"), false);
    h.key("home tab");
    assert!(h.active_text().starts_with("  export"), "web uses tabSize 2 from .vscode/settings.json");
    h.app.open_file(&m.api.join("package.json"), false);
    h.key("home tab");
    assert!(h.active_text().starts_with("    {"), "api uses the default of 4");
}

#[test]
fn external_modification_reload_and_conflict() {
    let m = multi_root();
    let mut h = Harness::new();
    open_ws(&mut h, &m.ws_file);
    let file = m.api.join("src/jobs.service.ts");
    h.app.open_file(&file, false);
    // Clean buffer: reload silently.
    fs::write(&file, "class Jobs { run() {} }\n").unwrap();
    h.fs_changed(vec![file.clone()]);
    assert_eq!(h.active_text(), "class Jobs { run() {} }\n");
    // Dirty buffer: ask.
    h.type_text("// mine ");
    fs::write(&file, "external again\n").unwrap();
    h.fs_changed(vec![file.clone()]);
    assert!(matches!(h.app.overlays.last(), Some(Overlay::Confirm(c)) if c.title.contains("changed on disk")));
    h.key("enter"); // Keep mine (default)
    assert!(h.active_text().starts_with("// mine "), "{:?}", h.active_text());
    // Saving now warns about the conflict instead of silently overwriting.
    h.key("ctrl+s");
    assert!(matches!(h.app.overlays.last(), Some(Overlay::Confirm(c)) if c.title.contains("changed on disk")));
    h.key("escape");
    assert_eq!(fs::read_to_string(&file).unwrap(), "external again\n");
    // Deleted on disk keeps the buffer.
    fs::remove_file(&file).unwrap();
    h.fs_changed(vec![file.clone()]);
    assert!(h.active_text().starts_with("// mine "));
}

#[test]
fn removing_a_root_keeps_files_and_updates_workspace_file() {
    let m = multi_root();
    let mut h = Harness::new();
    open_ws(&mut h, &m.ws_file);
    h.app.focus_explorer();
    let idx = h.app.explorer.rows.iter().position(|r| r.label == "Global API").unwrap();
    h.app.explorer.selected = idx;
    h.command("workbench.action.removeRootFolder");
    assert!(
        matches!(h.app.overlays.last(), Some(Overlay::Confirm(c)) if c.message.iter().any(|l| l.contains("NOT deleted")))
    );
    h.key("enter");
    assert!(m.api.join("src/index.ts").exists(), "files on disk untouched");
    let saved = fs::read_to_string(&m.ws_file).unwrap();
    assert!(!saved.contains("Global API"));
    assert!(saved.starts_with("// Cursor workspace"), "comments preserved");
    assert!(saved.contains("\"launch\""), "unknown fields preserved");
    assert!(saved.contains("// relative to this file"));
}

#[test]
fn add_folder_from_unrelated_location() {
    let m = multi_root();
    let extra = tempfile::tempdir().unwrap();
    fs::write(extra.path().join("main.rs"), "fn main() {}\n").unwrap();
    let mut h = Harness::new();
    open_ws(&mut h, &m.ws_file);
    h.command("workbench.action.addRootFolder");
    if let Some(Overlay::Prompt(p)) = h.app.overlays.last_mut() {
        p.input = yace_app::overlay::LineInput::new(extra.path().display().to_string());
    }
    h.key("enter");
    assert!(h.app.overlays.is_empty());
    assert_eq!(h.app.workspace.roots().len(), 5);
    assert!(h.explorer_labels().iter().any(|l| l.trim() == "main.rs"));
    assert!(fs::read_to_string(&m.ws_file).unwrap().contains(&extra.path().display().to_string()));
}

#[test]
fn session_restores_tabs_cursor_split_and_expansion() {
    let m = multi_root();
    let state = tempfile::tempdir().unwrap();
    let state_path = state.path().to_path_buf();
    {
        let mut h = Harness::with_state(state);
        open_ws(&mut h, &m.ws_file);
        h.app.open_file(&m.api.join("src/jobs.service.ts"), false);
        h.key("ctrl+end");
        h.command("workbench.action.splitEditor");
        h.app.open_file(&m.web.join("next.config.ts"), false);
        h.key("right right");
        h.select_explorer(&m.chat.join("src/index.ts"));
        h.app.save_session();
        std::mem::forget(h.state); // keep the dir for the second run
    }
    let state = tempfile::TempDir::new_in(state_path.parent().unwrap()).unwrap();
    // Reuse the same state directory contents.
    let _ = fs::remove_dir_all(state.path());
    copy_dir(&state_path, state.path());
    let mut h = Harness::with_state(state);
    open_ws(&mut h, &m.ws_file);
    let order = h.app.layout.order();
    assert_eq!(order.len(), 2, "split restored");
    let g1 = h.app.layout.group(order[0]).unwrap();
    let v = g1.active_view().unwrap();
    assert_eq!(h.app.docs.name(v.doc), "jobs.service.ts");
    assert_eq!(v.sel.primary().head, "class Jobs {}\n".chars().count(), "cursor restored");
    let g2 = h.app.layout.group(order[1]).unwrap();
    assert!(g2.views.iter().any(|v| h.app.docs.name(v.doc) == "next.config.ts"));
    assert!(h.explorer_labels().iter().any(|l| l.trim() == "index.ts"), "expansion restored");
    let _ = fs::remove_dir_all(&state_path);
}

fn copy_dir(from: &std::path::Path, to: &std::path::Path) {
    fs::create_dir_all(to).unwrap();
    for e in fs::read_dir(from).unwrap() {
        let e = e.unwrap();
        let p = to.join(e.file_name());
        if e.path().is_dir() {
            copy_dir(&e.path(), &p);
        } else {
            fs::copy(e.path(), p).unwrap();
        }
    }
}

#[test]
fn keybindings_route_by_focus() {
    let m = multi_root();
    let mut h = Harness::new();
    open_ws(&mut h, &m.ws_file);
    h.app.focus_explorer();
    h.app.explorer.selected = 0;
    h.key("down");
    assert_eq!(h.app.explorer.selected, 1, "Down moves the explorer selection");
    h.app.open_file(&m.api.join("package.json"), false);
    assert_eq!(h.app.focus, Focus::Editor);
    let before = h.app.explorer.selected;
    h.key("end");
    assert_eq!(h.app.explorer.selected, before, "editor keys don't touch the explorer");
    let (_, v, _) = h.app.active().unwrap();
    assert_eq!(h.app.layout.active_group().views[v].sel.primary().head, 2);
    // Chords
    h.key("ctrl+k");
    assert!(h.app.resolver.is_pending());
    h.key("ctrl+m");
    assert!(h.app.layout.maximized, "Ctrl+K Ctrl+M toggles maximize");
    // Command palette opens and runs commands.
    h.key("ctrl+shift+p");
    assert!(matches!(h.app.overlays.last(), Some(Overlay::Picker(_))));
    h.type_text("toggle primary side bar");
    h.key("enter");
    assert!(!h.app.sidebar_visible);
}

#[test]
fn quit_with_unsaved_changes_asks_then_discards() {
    let m = multi_root();
    let mut h = Harness::new();
    open_ws(&mut h, &m.ws_file);
    let f = m.api.join("package.json");
    h.app.open_file(&f, false);
    h.type_text("x");
    h.key("ctrl+q");
    assert!(!h.app.quit);
    assert!(matches!(h.app.overlays.last(), Some(Overlay::Confirm(c)) if c.title == "Unsaved changes"));
    h.key("escape");
    assert!(!h.app.quit, "cancel keeps running");
    h.key("ctrl+q");
    h.key("d"); // Don't Save accelerator
    assert!(h.app.quit);
    assert_eq!(fs::read_to_string(&f).unwrap(), "{}\n");
}

#[test]
fn explorer_file_operations() {
    let m = multi_root();
    let mut h = Harness::new();
    open_ws(&mut h, &m.ws_file);
    h.app.focus_explorer();
    h.select_explorer(&m.chat.join("src/index.ts"));
    h.command("explorer.newFile");
    h.type_text("nested/new.ts");
    h.key("enter");
    let created = m.chat.join("src/nested/new.ts");
    assert!(created.exists());
    assert_eq!(h.app.docs.name(h.app.active().unwrap().2), "new.ts");
    h.type_text("const x = 1;");
    h.key("ctrl+s");
    // Rename the open file: the tab follows.
    h.app.focus_explorer();
    h.select_explorer(&created);
    h.command("renameFile");
    h.key("ctrl+a");
    h.type_text("renamed.ts");
    h.key("enter");
    let renamed = m.chat.join("src/nested/renamed.ts");
    assert!(renamed.exists() && !created.exists());
    assert_eq!(fs::read_to_string(&renamed).unwrap(), "const x = 1;");
    let d = h.app.docs.find_by_path(&renamed).expect("doc retargeted");
    assert_eq!(h.app.docs.name(d), "renamed.ts");
    // Roots can't be deleted from disk: Delete on a root offers removal.
    h.app.explorer.selected = 0;
    h.command("deleteFile");
    assert!(matches!(h.app.overlays.last(), Some(Overlay::Confirm(c)) if c.title.contains("Remove folder")));
}

#[test]
fn multi_cursor_and_clipboard_in_app() {
    let dir = tempfile::tempdir().unwrap();
    let f = dir.path().join("a.txt");
    fs::write(&f, "foo bar\nfoo baz\nfoo qux\n").unwrap();
    let mut h = Harness::new();
    let launch = Launch {
        target: Target::File { path: f.clone(), line: None, col: None },
        add_roots: vec![],
        goto: None,
        restore: false,
    };
    initial_workspace(&mut h.app, &launch);
    h.settle();
    h.key("ctrl+d ctrl+d ctrl+d");
    let (_, v, _) = h.app.active().unwrap();
    assert_eq!(h.app.layout.active_group().views[v].sel.len(), 3);
    h.type_text("x");
    assert_eq!(h.active_text(), "x bar\nx baz\nx qux\n");
    h.key("escape");
    h.key("ctrl+home ctrl+c ctrl+v");
    assert_eq!(h.active_text(), "x bar\nx bar\nx baz\nx qux\n", "line copy/paste with no selection");
    h.key("alt+down");
    assert_eq!(h.active_text(), "x bar\nx baz\nx bar\nx qux\n");
    h.key("ctrl+/");
    assert_eq!(h.active_text(), "x bar\nx baz\n// x bar\nx qux\n");
}

#[test]
fn narrow_and_tiny_terminals_do_not_panic() {
    let m = multi_root();
    let mut h = Harness::new();
    open_ws(&mut h, &m.ws_file);
    h.app.open_file(&m.api.join("src/index.ts"), false);
    for (w, hgt) in [(1, 1), (10, 3), (30, 8), (49, 20), (200, 60)] {
        h.app.resize(w, hgt);
        h.term = ratatui::Terminal::new(ratatui::backend::TestBackend::new(w, hgt)).unwrap();
        let _ = h.screen();
    }
}
