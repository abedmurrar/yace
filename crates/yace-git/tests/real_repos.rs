//! Against real repositories made with the installed git (skipped without git).

use std::{path::Path, process::Command};

use yace_git::{Git, Kind, detect, ops, status};

fn git_ok() -> bool {
    Command::new("git").arg("--version").output().is_ok()
}

fn sh(dir: &Path, args: &[&str]) {
    let st = Command::new("git")
        .args(args)
        .current_dir(dir)
        .env("GIT_AUTHOR_NAME", "T")
        .env("GIT_AUTHOR_EMAIL", "t@example.com")
        .env("GIT_COMMITTER_NAME", "T")
        .env("GIT_COMMITTER_EMAIL", "t@example.com")
        .output()
        .unwrap();
    assert!(st.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&st.stderr));
}

fn init(dir: &Path) {
    sh(dir, &["init", "-q", "-b", "main"]);
    sh(dir, &["config", "user.name", "T"]);
    sh(dir, &["config", "user.email", "t@example.com"]);
    sh(dir, &["config", "commit.gpgsign", "false"]);
}

#[test]
fn detect_status_ops_and_branches() {
    if !git_ok() {
        return;
    }
    let d = tempfile::tempdir().unwrap();
    let r = d.path();
    let g = Git::default();
    assert_eq!(detect(&g, r).unwrap(), None, "not a repo yet");
    init(r);
    std::fs::create_dir_all(r.join("src/deep")).unwrap();
    std::fs::write(r.join("src/a.rs"), "one\ntwo\n").unwrap();
    std::fs::write(r.join("gone.txt"), "x\n").unwrap();
    std::fs::write(r.join("old name.txt"), "rename me\n").unwrap();
    std::fs::write(r.join(".gitignore"), "target/\n").unwrap();
    sh(r, &["add", "-A"]);
    sh(r, &["commit", "-q", "-m", "first"]);
    let info = detect(&g, &r.join("src/deep")).unwrap().expect("subfolder is in the repo");
    assert_eq!(info.toplevel.canonicalize().unwrap(), r.canonicalize().unwrap());
    assert!(info.git_dir.ends_with(".git"));

    std::fs::write(r.join("src/a.rs"), "one\nTWO\n").unwrap();
    std::fs::remove_file(r.join("gone.txt")).unwrap();
    std::fs::write(r.join("staged ü.rs"), "s\n").unwrap();
    sh(r, &["add", "staged ü.rs"]);
    sh(r, &["mv", "old name.txt", "new name.txt"]);
    std::fs::create_dir_all(r.join("target/debug")).unwrap();
    std::fs::write(r.join("target/debug/x"), "").unwrap();
    std::fs::create_dir_all(r.join("fresh")).unwrap();
    std::fs::write(r.join("fresh/u.txt"), "").unwrap();
    let st = status(&g, r).unwrap();
    assert_eq!(st.branch.as_deref(), Some("main"));
    let find = |p: &str| st.files.iter().find(|f| f.path == p).unwrap_or_else(|| panic!("{p}: {:?}", st.files));
    assert_eq!(find("src/a.rs").kind(), Kind::Modified);
    assert!(find("src/a.rs").unstaged());
    assert_eq!(find("gone.txt").kind(), Kind::Deleted);
    assert_eq!(find("staged ü.rs").kind(), Kind::Added);
    assert!(find("staged ü.rs").staged());
    assert_eq!(find("new name.txt").kind(), Kind::Renamed);
    assert_eq!(find("new name.txt").orig.as_deref(), Some("old name.txt"));
    assert_eq!(find("fresh/").kind(), Kind::Untracked, "untracked folders are collapsed");
    assert_eq!(find("target/").kind(), Kind::Ignored, "ignored folders are collapsed");

    // Bases and diffs.
    assert_eq!(ops::base_text(&g, r, "src/a.rs", true).unwrap().as_deref(), Some("one\ntwo\n"));
    assert_eq!(ops::base_text(&g, r, "nope.rs", true).unwrap(), None);
    let d = ops::diff_text(&g, r, "src/a.rs", false).unwrap();
    assert!(d.contains("-two\n+TWO\n"), "{d}");
    let u = ops::untracked_diff(&g, r, "fresh/u.txt").unwrap();
    assert!(u.contains("new file mode"), "{u}");

    // Stage, unstage, commit.
    ops::stage(&g, r, &["src/a.rs"]).unwrap();
    assert!(status(&g, r).unwrap().files.iter().any(|f| f.path == "src/a.rs" && f.staged() && !f.unstaged()));
    ops::unstage(&g, r, &["src/a.rs"], true).unwrap();
    assert!(status(&g, r).unwrap().files.iter().any(|f| f.path == "src/a.rs" && !f.staged()));
    let first = ops::commit(&g, r, "second: with \"quotes\" and $HOME\n\nbody", false).unwrap();
    assert!(first.contains("second"), "{first}");
    let log = Command::new("git").args(["log", "-1", "--format=%B"]).current_dir(r).output().unwrap();
    assert_eq!(String::from_utf8_lossy(&log.stdout).trim(), "second: with \"quotes\" and $HOME\n\nbody");
    let e = ops::commit(&g, r, "nothing", false);
    assert!(e.is_err(), "nothing staged is an error with git's message");

    // Branches.
    ops::create_branch(&g, r, "feature/x").unwrap();
    assert_eq!(status(&g, r).unwrap().branch.as_deref(), Some("feature/x"));
    let bs = ops::branches(&g, r).unwrap();
    assert!(bs.iter().any(|b| b.name == "main" && !b.current));
    assert!(bs.iter().any(|b| b.name == "feature/x" && b.current));
    let main = bs.iter().find(|b| b.name == "main").unwrap().clone();
    ops::switch(&g, r, &main).unwrap();
    assert_eq!(status(&g, r).unwrap().branch.as_deref(), Some("main"));
    assert!(!ops::valid_branch_name(&g, r, "bad..name"));
    assert!(ops::valid_branch_name(&g, r, "good/name"));

    // Every read used --no-optional-locks; only the operations wrote.
    let log = g.log();
    assert!(
        log.iter().filter(|e| e.mutating).all(|e| ["add", "restore", "commit", "switch"].contains(&e.args[0].as_str()))
    );
}

#[test]
fn conflicts_ahead_behind_and_detached() {
    if !git_ok() {
        return;
    }
    let remote = tempfile::tempdir().unwrap();
    init(remote.path());
    std::fs::write(remote.path().join("f.txt"), "base\n").unwrap();
    sh(remote.path(), &["add", "-A"]);
    sh(remote.path(), &["commit", "-q", "-m", "base"]);
    let clone = tempfile::tempdir().unwrap();
    let c = clone.path().join("c");
    sh(clone.path(), &["clone", "-q", remote.path().to_str().unwrap(), "c"]);
    sh(&c, &["config", "user.name", "T"]);
    sh(&c, &["config", "user.email", "t@example.com"]);
    // Diverge: one commit each side.
    std::fs::write(remote.path().join("f.txt"), "theirs\n").unwrap();
    sh(remote.path(), &["commit", "-q", "-am", "theirs"]);
    std::fs::write(c.join("f.txt"), "ours\n").unwrap();
    sh(&c, &["commit", "-q", "-am", "ours"]);
    sh(&c, &["fetch", "-q"]);
    let g = Git::default();
    let st = status(&g, &c).unwrap();
    assert_eq!((st.ahead, st.behind), (1, 1));
    assert_eq!(st.upstream.as_deref(), Some("origin/main"));
    // A real merge conflict.
    let _ = Command::new("git").args(["merge", "-q", "origin/main"]).current_dir(&c).output();
    let st = status(&g, &c).unwrap();
    assert_eq!(st.conflicts(), 1);
    assert_eq!(st.files[0].kind(), Kind::Conflicted);
    sh(&c, &["merge", "--abort"]);
    // Detached HEAD.
    sh(&c, &["checkout", "-q", "--detach", "HEAD"]);
    let st = status(&g, &c).unwrap();
    assert_eq!(st.branch, None);
    assert!(st.label().starts_with('('));
    // A missing program.
    let missing = Git::new("/nonexistent/git");
    assert!(matches!(detect(&missing, &c), Err(yace_git::GitError::NotFound(_))));
}
