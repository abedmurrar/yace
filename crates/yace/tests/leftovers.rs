//! Items left open by earlier milestones: canonical watches, typed command
//! arguments, Explorer file operations, file-type icons.

mod common;

use std::path::{Path, PathBuf};

use common::Harness;

/// Open a workspace file listing `roots`.
fn open_ws(h: &mut Harness, roots: &[&Path]) -> tempfile::TempDir {
    let ws = tempfile::tempdir().unwrap();
    let folders: Vec<String> = roots.iter().map(|r| format!("{{ \"path\": \"{}\" }}", r.display())).collect();
    let file = ws.path().join("w.code-workspace");
    std::fs::write(&file, format!("{{ \"folders\": [{}] }}", folders.join(","))).unwrap();
    yace_app::initial_workspace(
        &mut h.app,
        &yace_app::cli::Launch {
            target: yace_app::cli::Target::WorkspaceFile(file),
            add_roots: vec![],
            goto: None,
            restore: false,
        },
    );
    h.settle();
    ws
}

fn files(root: &Path, list: &[(&str, &str)]) {
    for (rel, text) in list {
        let p = root.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, text).unwrap();
    }
}

/// A root that is a symlink to another root's folder: one watch for the
/// real directory, and changes show up under both paths — also after one
/// of them stops needing the watch (spec §8.4).
#[test]
fn symlinked_folders_share_one_watch() {
    let d = tempfile::tempdir().unwrap();
    let proj = d.path().join("proj");
    files(&proj, &[("src/a.rs", "a\n")]);
    let link = d.path().join("src-link");
    std::os::unix::fs::symlink(proj.join("src"), &link).unwrap();
    let mut h = Harness::new();
    let _ws = open_ws(&mut h, &[&proj, &link]);
    let src = proj.join("src");
    h.select_explorer(&src);
    h.command("list.expand");
    let canon = std::fs::canonicalize(&src).unwrap();
    let watched: Vec<PathBuf> = h.app.watched_dirs().iter().cloned().collect();
    assert_eq!(watched.iter().filter(|p| **p == canon).count(), 1, "{watched:?}");
    assert!(!watched.contains(&link), "the symlink is not watched separately: {watched:?}");

    // An event on the real directory reaches both views of it.
    std::fs::write(canon.join("new.rs"), "").unwrap();
    h.fs_changed(vec![canon.join("new.rs")]);
    let labels = h.explorer_labels();
    assert_eq!(labels.iter().filter(|l| l.trim() == "new.rs").count(), 2, "{labels:#?}");

    // Collapse the folder in the first root: the link still gets events.
    h.select_explorer(&src);
    h.command("list.collapse");
    assert!(h.app.watched_dirs().contains(&canon));
    std::fs::write(canon.join("later.rs"), "").unwrap();
    h.fs_changed(vec![canon.join("later.rs")]);
    assert!(h.explorer_labels().iter().any(|l| l.trim() == "later.rs"), "{:#?}", h.explorer_labels());
}

fn overlay_input(h: &Harness) -> Option<String> {
    match h.app.overlays.last() {
        Some(yace_app::overlay::Overlay::Picker(p)) => Some(p.input.text.clone()),
        _ => None,
    }
}

/// Commands take typed arguments and report why they did not run
/// (spec §19.1).
#[test]
fn commands_take_arguments_and_report_errors() {
    use serde_json::json;
    use yace_app::commands::CommandError;
    let d = tempfile::tempdir().unwrap();
    files(d.path(), &[("a.txt", "one\ntwo\nthree\n")]);
    let mut h = Harness::new();
    let _ws = open_ws(&mut h, &[d.path()]);

    // Errors.
    assert!(matches!(h.app.run_command("no.such.command", None), Err(CommandError::Unknown(_))));
    assert!(matches!(
        h.app.run_command("editor.action.formatDocument", None),
        Err(CommandError::Unavailable(m)) if m.contains("no editor")
    ));
    assert!(matches!(h.app.run_command("type", Some(&json!({ "text": "x" }))), Err(CommandError::Unavailable(_))));
    let bad = h.app.run_command("type", Some(&json!({ "txt": 1 })));
    assert!(matches!(&bad, Err(CommandError::InvalidArgs(m)) if m.contains(r#"{ "text": string }"#)), "{bad:?}");
    // `execute` turns an error into one notification naming the command.
    h.app.notifications.clear();
    h.app.execute("workbench.action.gotoLine", Some(&json!({ "lin": 2 })));
    let n: Vec<String> = h.app.notifications.iter().map(|n| n.text.clone()).collect();
    assert!(n.len() == 1 && n[0].contains("Go to Line/Column") && n[0].contains("expected arguments"), "{n:?}");

    // Open a file with a position, type, go to a line, insert a snippet.
    let a = d.path().join("a.txt");
    h.app
        .run_command("vscode.open", Some(&json!({ "path": a.display().to_string(), "line": 2, "column": 2 })))
        .unwrap();
    h.settle();
    assert_eq!(h.cursor(), (1, 1));
    h.app.run_command("type", Some(&json!({ "text": "X" }))).unwrap();
    assert_eq!(h.active_text(), "one\ntXwo\nthree\n");
    h.app.run_command("workbench.action.gotoLine", Some(&json!(3))).unwrap();
    assert_eq!(h.cursor(), (2, 0));
    h.app.run_command("workbench.action.gotoLine", Some(&json!({ "line": 1, "column": 4 }))).unwrap();
    assert_eq!(h.cursor(), (0, 3));
    h.app.run_command("editor.action.insertSnippet", Some(&json!({ "snippet": "(${1:arg})$0" }))).unwrap();
    assert_eq!(h.active_text(), "one(arg)\ntXwo\nthree\n");
    let (g, v, _) = h.app.active().unwrap();
    let r = h.app.layout.group(g).unwrap().views[v].sel.primary();
    assert_eq!((r.from(), r.to()), (4, 7), "the first placeholder is selected");
    assert!(matches!(
        h.app.run_command("workbench.action.files.openFile", Some(&json!("missing.txt"))),
        Err(CommandError::Failed(_))
    ));

    // Quick Open with a prefix; no terminal for sendSequence.
    h.app.run_command("workbench.action.quickOpen", Some(&json!(">"))).unwrap();
    assert_eq!(overlay_input(&h).as_deref(), Some(">"));
    h.key("escape");
    assert!(matches!(
        h.app.run_command("workbench.action.terminal.sendSequence", Some(&json!({ "text": "ls\n" }))),
        Err(CommandError::Unavailable(_))
    ));

    // A key binding with arguments, from keybindings.toml.
    let kb = h.paths().keybindings();
    std::fs::create_dir_all(kb.parent().unwrap()).unwrap();
    std::fs::write(
        &kb,
        "[[bindings]]\nkey = \"ctrl+alt+j\"\ncommand = \"type\"\nargs = { text = \"!\" }\n\
         [[bindings]]\nkey = \"ctrl+alt+k\"\ncommand = \"type\"\nargs = { wrong = 1 }\n",
    )
    .unwrap();
    h.fs_changed(vec![kb.clone()]);
    h.key("ctrl+alt+j");
    assert!(h.active_text().contains('!'), "{}", h.active_text());
    h.app.notifications.clear();
    h.key("ctrl+alt+k");
    let n: Vec<String> = h.app.notifications.iter().map(|n| n.text.clone()).collect();
    assert!(n.iter().any(|m| m.contains("keybindings.toml")), "{n:?}");
}

#[test]
fn copy_names_follow_vs_code() {
    use yace_app::app::explorer_ops::unique_destination;
    let d = tempfile::tempdir().unwrap();
    let p = d.path();
    files(p, &[("a.txt", ""), ("Makefile", ""), (".env", ""), ("dir/x", ""), ("a.tar.gz", "")]);
    assert_eq!(unique_destination(p, "new.txt"), p.join("new.txt"));
    assert_eq!(unique_destination(p, "a.txt"), p.join("a copy.txt"));
    std::fs::write(p.join("a copy.txt"), "").unwrap();
    assert_eq!(unique_destination(p, "a.txt"), p.join("a copy 2.txt"));
    assert_eq!(unique_destination(p, "Makefile"), p.join("Makefile copy"));
    assert_eq!(unique_destination(p, ".env"), p.join(".env copy"));
    assert_eq!(unique_destination(p, "dir"), p.join("dir copy"));
    assert_eq!(unique_destination(p, "a.tar.gz"), p.join("a.tar copy.gz"));
}

/// Explorer copy / cut / paste, Open to the Side, Reveal (spec §8.2).
#[test]
fn explorer_copy_cut_paste_beside_and_reveal() {
    let a = tempfile::tempdir().unwrap();
    let b = tempfile::tempdir().unwrap();
    files(a.path(), &[("src/a.txt", "A\n"), ("sub/inner/deep.txt", "deep\n"), ("move.txt", "move me\n")]);
    files(b.path(), &[("readme.md", "b\n")]);
    std::os::unix::fs::symlink("deep.txt", a.path().join("sub/inner/link")).unwrap();
    let mut h = Harness::new();
    let _ws = open_ws(&mut h, &[a.path(), b.path()]);
    h.app.focus_explorer();

    // Copy a file and paste it into its own folder twice.
    let src_a = a.path().join("src/a.txt");
    h.select_explorer(&src_a);
    h.key("ctrl+c");
    h.key("ctrl+v");
    h.key("ctrl+v");
    assert_eq!(std::fs::read_to_string(a.path().join("src/a copy.txt")).unwrap(), "A\n");
    assert!(a.path().join("src/a copy 2.txt").exists());
    assert!(h.explorer_labels().iter().any(|l| l.trim() == "a copy 2.txt"), "{:#?}", h.explorer_labels());

    // Copy a folder (with a symlink inside) into the other root.
    let sub = a.path().join("sub");
    h.select_explorer(&sub);
    h.key("ctrl+c");
    h.select_explorer(b.path());
    h.key("ctrl+v");
    assert_eq!(std::fs::read_to_string(b.path().join("sub/inner/deep.txt")).unwrap(), "deep\n");
    let link = b.path().join("sub/inner/link");
    assert!(std::fs::symlink_metadata(&link).unwrap().file_type().is_symlink(), "symlinks are copied as links");
    // …but never into itself.
    h.select_explorer(&a.path().join("sub/inner"));
    h.app.notifications.clear();
    h.key("ctrl+v");
    assert!(h.app.notifications.iter().any(|n| n.text.contains("into itself")));
    assert!(!a.path().join("sub/inner/sub").exists());

    // Cut a file that is open, paste it into the other root: the editor follows.
    let mv = a.path().join("move.txt");
    h.app.open_file(&mv, false);
    h.settle();
    h.app.focus_explorer();
    h.select_explorer(&mv);
    h.key("ctrl+x");
    assert!(h.app.is_cut(&mv));
    h.select_explorer(&b.path().join("readme.md"));
    h.key("ctrl+v");
    let moved = b.path().join("move.txt");
    assert!(moved.exists() && !mv.exists());
    let (_, _, d) = h.app.active().unwrap();
    assert_eq!(h.app.docs.doc(d).unwrap().path(), Some(moved.as_path()));
    assert!(h.app.file_clipboard.is_none(), "a cut is pasted once");

    // Ctrl+Enter opens beside, in a new group to the right.
    h.app.focus_explorer();
    h.select_explorer(&src_a);
    let groups = h.app.layout.groups().len();
    h.key("ctrl+enter");
    assert_eq!(h.app.layout.groups().len(), groups + 1);
    let (_, _, d) = h.app.active().unwrap();
    assert_eq!(h.app.docs.doc(d).unwrap().path(), Some(src_a.as_path()));

    // Reveal opens the containing folder, never the file.
    h.app.focus_explorer();
    h.select_explorer(&src_a);
    h.key("shift+alt+r");
    assert_eq!(h.opened, vec![a.path().join("src")]);
}

/// Ctrl+Enter in Quick Open opens the file beside.
#[test]
fn quick_open_ctrl_enter_opens_beside() {
    let d = tempfile::tempdir().unwrap();
    files(d.path(), &[("one.rs", "1\n"), ("two.rs", "2\n")]);
    let mut h = Harness::new();
    let _ws = open_ws(&mut h, &[d.path()]);
    h.app.open_file(&d.path().join("one.rs"), false);
    h.settle();
    h.app.execute("workbench.action.quickOpen", None);
    h.settle();
    h.type_text("two");
    h.settle();
    h.key("ctrl+enter");
    assert_eq!(h.app.layout.groups().len(), 2);
    assert!(h.active_text().starts_with('2'));
}
