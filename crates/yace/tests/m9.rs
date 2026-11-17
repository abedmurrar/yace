//! M9: Git integration (spec §15, §4.7, §8.5, §28.5) against real
//! repositories made with the installed git.

mod common;

use std::{
    path::{Path, PathBuf},
    process::Command,
    time::Duration,
};

use common::Harness;
use ratatui::{crossterm::event::KeyModifiers, style::Color};

fn git(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .args(args)
        .current_dir(dir)
        .env("GIT_AUTHOR_NAME", "T")
        .env("GIT_AUTHOR_EMAIL", "t@example.com")
        .env("GIT_COMMITTER_NAME", "T")
        .env("GIT_COMMITTER_EMAIL", "t@example.com")
        .output()
        .unwrap();
    assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8_lossy(&out.stdout).into_owned()
}

/// A repository with one commit on `branch`.
fn repo(branch: &str, files: &[(&str, &str)]) -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    git(d.path(), &["init", "-q", "-b", branch]);
    git(d.path(), &["config", "user.name", "T"]);
    git(d.path(), &["config", "user.email", "t@example.com"]);
    git(d.path(), &["config", "commit.gpgsign", "false"]);
    for (f, text) in files {
        let p = d.path().join(f);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, text).unwrap();
    }
    git(d.path(), &["add", "-A"]);
    git(d.path(), &["commit", "-q", "-m", "first"]);
    d
}

/// A workspace file listing `roots` (unrelated folders).
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
    settle_git(h);
    ws
}

/// Let due git work run (debounces elapse in "virtual" time).
fn settle_git(h: &mut Harness) {
    for _ in 0..6 {
        h.app.on_tick(std::time::Instant::now() + Duration::from_secs(1));
        h.settle();
    }
}

fn status_line(h: &mut Harness) -> String {
    h.screen().lines().last().unwrap_or("").to_string()
}

fn fg_of(h: &mut Harness, needle: &str) -> Option<Color> {
    let buf = h.render();
    for y in 0..buf.area.height {
        let row: String = (0..buf.area.width).map(|x| buf[(x, y)].symbol().to_string()).collect();
        if let Some(b) = row.find(needle) {
            return Some(buf[(row[..b].chars().count() as u16, y)].fg);
        }
    }
    None
}

/// An untracked file has no base to diff against: no git deadline may stay
/// pending, or the event loop wakes up forever (100% CPU, v0.7.0 bug).
#[test]
fn untracked_files_leave_no_pending_deadline() {
    let r = repo("main", &[("tracked.txt", "a\n")]);
    std::fs::write(r.path().join("untracked.txt"), "new\n").unwrap();
    std::fs::write(r.path().join(".gitignore"), "ignored.txt\n").unwrap();
    std::fs::write(r.path().join("ignored.txt"), "secret\n").unwrap();
    let mut h = Harness::new();
    let _ws = open_ws(&mut h, &[r.path()]);
    for f in ["untracked.txt", "ignored.txt", "tracked.txt"] {
        h.app.open_file(&r.path().join(f), false);
        h.settle();
        settle_git(&mut h);
        h.type_text("x");
        settle_git(&mut h);
        let now = std::time::Instant::now();
        assert!(
            h.app.git_deadline_for_tests().is_none_or(|d| d > now),
            "{f}: a git deadline in the past keeps the loop spinning"
        );
    }
}

#[test]
fn each_root_has_its_own_repository_branch_and_decorations() {
    let api = repo("main", &[("src/index.ts", "export const a = 1;\n"), ("README.md", "api\n")]);
    let web = repo("develop", &[("app/page.tsx", "export default 1;\n")]);
    let plain = tempfile::tempdir().unwrap();
    std::fs::write(plain.path().join("notes.txt"), "no repo here\n").unwrap();
    std::fs::write(api.path().join("src/index.ts"), "export const a = 2;\n").unwrap();
    std::fs::write(api.path().join("new.ts"), "x\n").unwrap();
    let mut h = Harness::new();
    let _ws = open_ws(&mut h, &[api.path(), web.path(), plain.path()]);
    assert_eq!(h.app.git.repos.len(), 2, "two repositories; the plain folder has none");

    // Status bar follows the active file's repository.
    h.app.open_file(&api.path().join("src/index.ts"), false);
    settle_git(&mut h);
    let line = status_line(&mut h);
    assert!(line.contains("main*"), "{line}");
    h.app.open_file(&web.path().join("app/page.tsx"), false);
    settle_git(&mut h);
    let line = status_line(&mut h);
    assert!(line.contains("develop") && !line.contains("develop*"), "{line}");
    h.app.open_file(&plain.path().join("notes.txt"), false);
    settle_git(&mut h);

    // Explorer: decorations only where they belong.
    h.key("ctrl+shift+e");
    let t = h.app.theme.clone();
    h.app.explorer.selected = 0;
    let screen = h.screen();
    assert!(screen.contains("new.ts"), "{screen}");
    assert_eq!(fg_of(&mut h, "new.ts"), Some(t.git_added), "untracked");
    assert_eq!(
        h.app.git_decoration(&api.path().join("src")),
        Some(yace_git::Kind::Modified),
        "folder takes its contents' state"
    );
    assert_eq!(h.app.git_decoration(&api.path().join("README.md")), None);
    assert_eq!(h.app.git_decoration(&web.path().join("app/page.tsx")), None, "the other repo is clean");
    assert_eq!(h.app.git_decoration(&plain.path().join("notes.txt")), None);

    // Opening the workspace ran only read-only git commands.
    let log = h.app.git.git.log();
    assert!(!log.is_empty());
    assert!(log.iter().all(|e| !e.mutating), "{:?}", log.iter().map(|e| e.line()).collect::<Vec<_>>());
    assert!(log.iter().all(|e| ["rev-parse", "status", "show"].contains(&e.args[0].as_str())));
}

#[test]
fn gutter_shows_changes_while_typing_and_next_change_jumps() {
    let r = repo("main", &[("a.txt", "one\ntwo\nthree\nfour\nfive\n")]);
    let mut h = Harness::new();
    let _ws = open_ws(&mut h, &[r.path()]);
    h.app.open_file(&r.path().join("a.txt"), false);
    settle_git(&mut h);
    let d = h.app.active().unwrap().2;
    assert!(h.app.git_base(d).is_some(), "base fetched from the index");
    assert!(h.app.git_hunks(d).is_empty());
    // Modify line 2 and add a line after line 4 (unsaved).
    h.key("ctrl+home");
    h.key("down");
    h.key("end");
    h.type_text("!");
    h.key("down");
    h.key("down");
    h.key("end");
    h.type_text("\nnew");
    settle_git(&mut h);
    let kinds: Vec<_> = h.app.git_hunks(d).iter().map(|x| (x.kind, x.lines.clone())).collect();
    assert_eq!(kinds, vec![(yace_git::HunkKind::Modified, 1..2), (yace_git::HunkKind::Added, 4..5)]);
    let buf = h.render();
    let gl = yace_app::ui::workbench_layout(&h.app).groups[0].clone();
    let x = gl.content.x + gl.gutter - 1;
    let y1 = gl.content.y + 1;
    assert_eq!(buf[(x, y1)].symbol(), "▎");
    assert_eq!(buf[(x, y1)].fg, h.app.theme.git_modified);
    assert_eq!(buf[(x, gl.content.y + 4)].fg, h.app.theme.git_added);
    assert_eq!(buf[(x, gl.content.y)].symbol(), " ", "unchanged line");
    // Next / previous change.
    h.key("ctrl+home");
    h.key("alt+f5");
    assert_eq!(h.cursor().0, 1);
    h.key("alt+f5");
    assert_eq!(h.cursor().0, 4);
    h.key("alt+f5");
    assert_eq!(h.cursor().0, 1, "wraps");
    h.key("shift+alt+f5");
    assert_eq!(h.cursor().0, 4);
    // Show the change under the cursor.
    h.key("ctrl+home");
    h.key("down");
    h.command("yace.git.showChange");
    assert!(h.screen().contains("- two"), "{}", h.screen());
}

#[test]
fn outside_changes_are_picked_up() {
    let r = repo("main", &[("a.txt", "one\n")]);
    let mut h = Harness::new();
    let _ws = open_ws(&mut h, &[r.path()]);
    h.app.open_file(&r.path().join("a.txt"), false);
    settle_git(&mut h);
    assert!(status_line(&mut h).contains("main"));
    // A branch created and checked out in a shell.
    git(r.path(), &["switch", "-q", "-c", "feature"]);
    let git_dir: PathBuf = r.path().join(".git");
    assert!(h.app.watched_dirs().contains(&git_dir.canonicalize().unwrap()) || h.app.watched_dirs().contains(&git_dir));
    h.fs_changed(vec![git_dir.join("HEAD")]);
    settle_git(&mut h);
    assert!(status_line(&mut h).contains("feature"), "{}", status_line(&mut h));
    // A commit from outside clears the dirty marker and updates the base.
    std::fs::write(r.path().join("a.txt"), "one\ntwo\n").unwrap();
    h.fs_changed(vec![r.path().join("a.txt")]);
    settle_git(&mut h);
    assert!(status_line(&mut h).contains("feature*"));
    git(r.path(), &["commit", "-qam", "outside"]);
    h.fs_changed(vec![git_dir.join("index")]);
    settle_git(&mut h);
    let line = status_line(&mut h);
    assert!(line.contains("feature") && !line.contains("feature*"), "{line}");
    let d = h.app.active().unwrap().2;
    h.command("workbench.action.files.revert");
    settle_git(&mut h);
    assert_eq!(h.app.git_base(d).as_deref(), Some("one\ntwo\n"), "base re-read after the index changed");
}

#[test]
fn missing_git_is_one_hint_and_nothing_else_breaks() {
    let r = repo("main", &[("a.txt", "one\n")]);
    let mut h = Harness::with_config("[git]\npath = \"/nonexistent/git\"\n");
    let _ws = open_ws(&mut h, &[r.path()]);
    let notes: Vec<String> = h.app.notifications.iter().map(|n| n.text.clone()).collect();
    assert_eq!(notes.iter().filter(|n| n.contains("git program was not found")).count(), 1, "{notes:?}");
    h.app.open_file(&r.path().join("a.txt"), false);
    settle_git(&mut h);
    assert!(!status_line(&mut h).contains("main"));
    h.type_text("x");
    assert!(h.active_text().starts_with('x'));
}

fn scm_lines(h: &mut Harness) -> Vec<String> {
    h.screen().lines().map(|l| l.chars().take(40).collect::<String>().trim_end().to_string()).collect()
}

fn select_row(h: &mut Harness, label: &str) {
    let rows = h.app.scm_rows();
    let i = rows
        .iter()
        .position(|r| match r {
            yace_app::app::ScmRow::File { path, .. } => path == label,
            yace_app::app::ScmRow::Group { group, .. } => group.title() == label,
            yace_app::app::ScmRow::Repo(i) => h.app.git.repos[*i].name == label,
        })
        .unwrap_or_else(|| panic!("{label} not in {rows:?}"));
    h.app.scm.selected = i;
}

#[test]
fn source_control_view_stage_commit_and_branches() {
    let r = repo("main", &[("a.txt", "one\n"), ("b.txt", "bee\n")]);
    let other = repo("trunk", &[("x.txt", "x\n")]);
    std::fs::write(r.path().join("a.txt"), "one\ntwo\n").unwrap();
    std::fs::write(r.path().join("new.txt"), "new\n").unwrap();
    let mut h = Harness::new();
    let _ws = open_ws(&mut h, &[r.path(), other.path()]);
    h.key("ctrl+shift+g");
    settle_git(&mut h);
    let lines = scm_lines(&mut h);
    let text = lines.join("\n");
    assert!(text.contains("SOURCE CONTROL (2)"), "{text}");
    assert!(text.contains("Changes 1") && text.contains("Untracked 1"), "{text}");
    assert!(text.contains("trunk"), "both repositories listed: {text}");

    // Enter on a changed file opens a read-only, highlighted diff.
    select_row(&mut h, "a.txt");
    h.key("enter");
    settle_git(&mut h);
    let d = h.app.active().unwrap().2;
    assert!(h.app.docs.is_read_only(d));
    assert!(h.active_text().contains("+two"), "{}", h.active_text());
    assert_eq!(h.app.syntax.language(d).id, "diff");
    assert!(h.screen().contains("a.txt (Working Tree)"));
    assert_eq!(fg_of(&mut h, "+two"), Some(h.app.theme.git_added));
    h.type_text("x");
    assert!(!h.active_text().starts_with('x'), "read-only");
    // Enter on the added line jumps to it in the file.
    h.key("ctrl+end");
    h.key("up");
    h.key("enter");
    assert!(h.app.active().is_some_and(|(_, _, d)| !h.app.docs.is_read_only(d)));
    assert_eq!(h.cursor().0, 1);

    // Stage from the view: s on the file, then u to unstage, then s again.
    h.key("ctrl+shift+g");
    select_row(&mut h, "a.txt");
    h.key("s");
    settle_git(&mut h);
    assert!(scm_lines(&mut h).join("\n").contains("Staged Changes 1"));
    select_row(&mut h, "Staged Changes");
    h.key("u");
    settle_git(&mut h);
    assert!(!scm_lines(&mut h).join("\n").contains("Staged Changes"));
    select_row(&mut h, "a.txt");
    h.key("s");
    settle_git(&mut h);

    // Commit: c, type the message, Enter.
    h.key("c");
    assert!(h.screen().contains("Commit staged changes to"), "{}", h.screen());
    h.key("enter");
    assert!(h.screen().contains("Enter a commit message"));
    h.type_text("add two");
    h.key("enter");
    settle_git(&mut h);
    assert_eq!(git(r.path(), &["log", "-1", "--format=%s"]).trim(), "add two");
    let text = scm_lines(&mut h).join("\n");
    assert!(!text.contains("Staged Changes") && text.contains("Untracked 1"), "{text}");

    // Smart commit: nothing staged but a tracked change → offer to commit all.
    std::fs::write(r.path().join("b.txt"), "bee2\n").unwrap();
    h.fs_changed(vec![r.path().join("b.txt")]);
    settle_git(&mut h);
    select_row(&mut h, "b.txt");
    h.key("c");
    assert!(h.screen().contains("no staged changes"), "{}", h.screen());
    h.key("y");
    h.type_text("all of it");
    h.key("enter");
    settle_git(&mut h);
    assert_eq!(git(r.path(), &["log", "-1", "--format=%s"]).trim(), "all of it");

    // Branches: b → picker → create.
    select_row(&mut h, "new.txt");
    h.key("b");
    settle_git(&mut h);
    assert!(h.screen().contains("Switch Branch in"), "{}", h.screen());
    h.key("enter"); // "+ Create new branch…"
    h.type_text("feature/scm");
    h.key("enter");
    settle_git(&mut h);
    assert_eq!(git(r.path(), &["branch", "--show-current"]).trim(), "feature/scm");
    assert_eq!(git(other.path(), &["branch", "--show-current"]).trim(), "trunk", "the other repository is untouched");
    // Switch back to main through the picker.
    h.key("b");
    settle_git(&mut h);
    h.type_text("main");
    h.key("enter");
    settle_git(&mut h);
    assert_eq!(git(r.path(), &["branch", "--show-current"]).trim(), "main");

    // A failing operation shows git's message.
    h.key("b");
    settle_git(&mut h);
    h.key("enter");
    h.type_text("main");
    h.key("enter");
    settle_git(&mut h);
    let notes: Vec<String> = h.app.notifications.iter().map(|n| n.text.clone()).collect();
    assert!(notes.iter().any(|n| n.contains("failed") && n.contains("already exists")), "{notes:?}");

    // Show Output lists the operations.
    h.command("git.showOutput");
    let out = h.active_text();
    assert!(out.contains("git commit -F -") && out.contains("git switch -c feature/scm"), "{out}");
}

#[test]
fn commands_ask_which_repository_when_ambiguous() {
    let a = repo("main", &[("a.txt", "a\n")]);
    let b = repo("main", &[("b.txt", "b\n")]);
    let mut h = Harness::new();
    let _ws = open_ws(&mut h, &[a.path(), b.path()]);
    h.command("git.checkout");
    assert!(h.screen().contains("Choose a Repository"), "{}", h.screen());
    h.key("escape");
    // With a file open, its repository is used.
    h.app.open_file(&b.path().join("b.txt"), false);
    h.settle();
    h.command("git.checkout");
    settle_git(&mut h);
    let screen = h.screen();
    assert!(screen.contains("Switch Branch in") && !screen.contains("Choose a Repository"), "{screen}");
}

/// Clicking the branch in the status bar opens Checkout to… for the
/// repository the label names, without asking which one.
#[test]
fn clicking_the_status_bar_branch_checks_out_in_that_repository() {
    let a = repo("main", &[("a.txt", "a\n")]);
    let b = repo("trunk", &[("b.txt", "b\n")]);
    git(b.path(), &["branch", "-q", "only-in-b"]);
    let mut h = Harness::new();
    let _ws = open_ws(&mut h, &[a.path(), b.path()]);
    let click_branch = |h: &mut Harness| {
        let y = h.render().area.height - 1;
        h.click(1, y, KeyModifiers::NONE);
        settle_git(h);
        h.screen()
    };
    assert!(status_line(&mut h).contains("main"), "{}", status_line(&mut h));
    let screen = click_branch(&mut h);
    assert!(screen.contains("Switch Branch in") && !screen.contains("Choose a Repository"), "{screen}");
    assert!(!screen.contains("only-in-b"), "{screen}");
    h.key("escape");
    // With b's file open the label shows b's branch, and so does the picker.
    h.app.open_file(&b.path().join("b.txt"), false);
    h.settle();
    settle_git(&mut h);
    assert!(status_line(&mut h).contains("trunk"), "{}", status_line(&mut h));
    let screen = click_branch(&mut h);
    assert!(screen.contains("only-in-b") && !screen.contains("Choose a Repository"), "{screen}");
    // Clicking elsewhere in the status bar does nothing.
    h.key("escape");
    let y = h.render().area.height - 1;
    h.click(60, y, KeyModifiers::NONE);
    settle_git(&mut h);
    assert!(!h.screen().contains("Switch Branch in"));
}

/// The diff tab in the active group, if one is open.
fn diff_tab(h: &Harness) -> Option<u32> {
    h.app.docs.ids().into_iter().find(|d| h.app.diff_of(*d).is_some())
}

/// The row of the screen showing both `a` and `b`.
fn row_with(h: &mut Harness, a: &str, b: &str) -> Option<String> {
    h.screen().lines().find(|l| l.contains(a) && l.contains(b)).map(String::from)
}

#[test]
fn open_changes_shows_both_versions_side_by_side() {
    let r = repo("main", &[("a.txt", "one\ntwo\nthree\nfour\n")]);
    std::fs::write(r.path().join("a.txt"), "one\nTWO\nthree\nfour\nfive\n").unwrap();
    let mut h = Harness::new();
    let _ws = open_ws(&mut h, &[r.path()]);
    h.command("workbench.action.toggleSidebarVisibility");
    h.app.open_file(&r.path().join("a.txt"), false);
    h.settle();
    settle_git(&mut h);
    let docs_before = h.app.docs.ids().len();
    h.command("git.openChange");
    settle_git(&mut h);
    let tab = diff_tab(&h).expect("a side-by-side diff tab");
    let screen = h.screen();
    assert!(screen.contains("a.txt (Index ↔ Working Tree)"), "{screen}");
    // Aligned rows: the changed line beside its new version, the added
    // line beside filler, equal lines beside each other.
    assert!(row_with(&mut h, "two", "TWO").is_some(), "{screen}");
    assert!(row_with(&mut h, "three", "three").is_some());
    let five = row_with(&mut h, "five", "five").map(|_| ()).or_else(|| {
        h.screen().lines().find(|l| l.contains("five")).map(|l| assert!(l.contains('╱'), "filler beside: {l}"))
    });
    assert!(five.is_some(), "{}", h.screen());
    // Colours: the changed text is tinted more strongly than its line.
    let buf = h.render();
    let (mut two, mut big) = (None, None);
    for y in 0..buf.area.height {
        let row: String = (0..buf.area.width).map(|x| buf[(x, y)].symbol().to_string()).collect();
        if let (Some(a), Some(b)) = (row.find("two"), row.find("TWO")) {
            two = Some((row[..a].chars().count() as u16, y));
            big = Some((row[..b].chars().count() as u16, y));
        }
    }
    let ((tx, ty), (bx, by)) = (two.unwrap(), big.unwrap());
    assert_ne!(buf[(tx, ty)].bg, buf[(tx - 3, ty)].bg, "changed text vs the rest of its line");
    assert_ne!(buf[(tx, ty)].bg, buf[(bx, by)].bg, "removed vs added");
    // The cursor starts on the first change; the next change is "five".
    assert_eq!(h.app.diff_of(tab).unwrap().cursor, 1);
    h.key("alt+f5");
    assert_eq!(h.app.diff_of(tab).unwrap().cursor, 4);
    h.key("alt+f5");
    assert_eq!(h.app.diff_of(tab).unwrap().cursor, 1, "wraps around");
    h.key("down");
    h.key("down");
    assert_eq!(h.app.diff_of(tab).unwrap().cursor, 3);
    // Editing the open file updates the diff (after a pause).
    h.key("enter");
    assert_eq!(h.cursor(), (3, 0), "Enter opened the file at that line");
    h.key("ctrl+home");
    h.type_text("zero\n");
    h.app.on_tick(std::time::Instant::now() + Duration::from_secs(1));
    h.app.on_tick(std::time::Instant::now() + Duration::from_secs(2));
    let rows = &h.app.diff_of(tab).unwrap().rows;
    assert_eq!(rows.len(), 6, "{rows:?}");
    assert_eq!(rows[0].left, None, "the new first line is added");
    // Closing the diff tab drops the hidden version it opened.
    h.key("ctrl+z");
    h.key("ctrl+pagedown");
    assert_eq!(h.app.active().map(|(_, _, d)| d), Some(tab));
    h.key("ctrl+w");
    h.settle();
    assert!(diff_tab(&h).is_none());
    assert_eq!(h.app.docs.ids().len(), docs_before, "only the file's own document is left");
}

#[test]
fn staged_untracked_and_conflicts_open_side_by_side() {
    let r = repo("main", &[("a.txt", "base\n")]);
    // A conflict on a.txt: main and other both change its line.
    git(r.path(), &["checkout", "-q", "-b", "other"]);
    std::fs::write(r.path().join("a.txt"), "theirs\n").unwrap();
    git(r.path(), &["commit", "-qam", "other"]);
    git(r.path(), &["checkout", "-q", "main"]);
    std::fs::write(r.path().join("a.txt"), "ours\n").unwrap();
    git(r.path(), &["commit", "-qam", "main"]);
    let _ = Command::new("git").args(["merge", "-q", "other"]).current_dir(r.path()).output();
    std::fs::write(r.path().join("b.txt"), "new\nfile\n").unwrap();
    std::fs::write(r.path().join("c.txt"), "staged\n").unwrap();
    git(r.path(), &["add", "c.txt"]);
    let mut h = Harness::new();
    let _ws = open_ws(&mut h, &[r.path()]);
    // A narrow sidebar leaves the editor wide enough for two panes.
    h.app.sidebar_width = 12;
    h.command("workbench.view.scm");
    settle_git(&mut h);
    for (label, title, a, b) in [
        ("a.txt", "a.txt (Ours ↔ Theirs)", "ours", "theirs"),
        ("b.txt", "b.txt (Empty ↔ Working Tree)", "file", "file"),
        ("c.txt", "c.txt (HEAD ↔ Index)", "staged", "staged"),
    ] {
        h.command("workbench.view.scm");
        select_row(&mut h, label);
        h.key("enter");
        settle_git(&mut h);
        let screen = h.screen();
        assert!(screen.contains(title), "{label}: {screen}");
        assert!(row_with(&mut h, a, b).is_some() || screen.contains(b), "{label}: {screen}");
    }
}

#[test]
fn a_narrow_group_gets_the_unified_diff() {
    let r = repo("main", &[("a.txt", "one\n")]);
    std::fs::write(r.path().join("a.txt"), "uno\n").unwrap();
    let mut h = Harness::new();
    let _ws = open_ws(&mut h, &[r.path()]);
    h.app.open_file(&r.path().join("a.txt"), false);
    h.settle();
    settle_git(&mut h);
    h.command("git.openChange");
    settle_git(&mut h);
    assert!(diff_tab(&h).is_none());
    assert!(h.active_text().contains("@@"), "{}", h.active_text());
}

/// The Task Runner plugin subscribes to `gitStatusChanged`: switching
/// branches (where package.json differs) updates its view.
#[test]
fn plugins_receive_git_status_changes() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let st =
        Command::new(env!("CARGO")).args(["build", "-p", "yace-plugin-tasks"]).current_dir(&root).status().unwrap();
    assert!(st.success());
    let target = std::env::var_os("CARGO_TARGET_DIR").map(PathBuf::from).unwrap_or_else(|| root.join("target"));
    let staged = tempfile::tempdir().unwrap();
    std::fs::copy(root.join("plugins/tasks/yace-plugin.toml"), staged.path().join("yace-plugin.toml")).unwrap();
    std::fs::create_dir_all(staged.path().join("lib")).unwrap();
    std::fs::copy(target.join("debug/libyace_plugin_tasks.so"), staged.path().join("lib/libyace_plugin_tasks.so"))
        .unwrap();

    let r = repo("main", &[("package.json", r#"{"scripts": {"build": "x"}}"#)]);
    git(r.path(), &["switch", "-q", "-c", "other"]);
    std::fs::write(r.path().join("package.json"), r#"{"scripts": {"build": "x", "deploy": "y"}}"#).unwrap();
    git(r.path(), &["commit", "-qam", "deploy script"]);
    git(r.path(), &["switch", "-q", "main"]);

    let mut h = Harness::new();
    let plugins = h.paths().config_dir.join("plugins");
    let plan =
        yace_plugin_host::store::plan_install(staged.path(), &plugins, yace_app::app::App::yace_version()).unwrap();
    yace_plugin_host::store::install(&plan, &plugins).unwrap();
    h.app.discover_plugins();
    h.settle();
    let _ws = open_ws(&mut h, &[r.path()]);
    assert!(h.wait_for(Duration::from_secs(10), |h| h.screen().contains("1 tasks")), "{}", h.screen());
    // Switch branches outside YACE; the .git watch triggers a status refresh.
    git(r.path(), &["switch", "-q", "other"]);
    h.fs_changed(vec![r.path().join(".git/HEAD")]);
    settle_git(&mut h);
    assert!(h.wait_for(Duration::from_secs(10), |h| h.screen().contains("2 tasks")), "{}", h.screen());
}
