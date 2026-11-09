//! Leftovers batch 2, search and Quick Open: replace previews, live
//! search results, incremental Find, a Quick Open index kept live.

mod common;

use std::{
    path::Path,
    time::{Duration, Instant},
};

use common::Harness;
use yace_app::app::{SearchField, SidebarView};

fn open_folder(h: &mut Harness, root: &Path) {
    yace_app::initial_workspace(
        &mut h.app,
        &yace_app::cli::Launch {
            target: yace_app::cli::Target::Folder(root.to_path_buf()),
            add_roots: vec![],
            goto: None,
            restore: false,
        },
    );
    h.settle();
}

/// Use Quick Open once: from then on its index is kept live.
fn start_index(h: &mut Harness) {
    h.key("ctrl+p");
    h.key("escape");
}

fn files(root: &Path, list: &[(&str, &str)]) {
    for (rel, text) in list {
        let p = root.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, text).unwrap();
    }
}

/// Let debounces and live-update delays elapse.
fn tick(h: &mut Harness) {
    h.app.on_tick(Instant::now() + Duration::from_secs(1));
    h.settle();
}

fn search(h: &mut Harness, query: &str) {
    h.key("ctrl+shift+f");
    assert_eq!(h.app.sidebar_view, SidebarView::Search);
    h.key("ctrl+a");
    h.type_text(query);
    h.key("enter");
}

fn result_files(h: &Harness) -> Vec<(String, usize)> {
    h.app.search.files.iter().map(|f| (f.rel.clone(), f.matches.iter().map(|m| m.ranges.len()).sum())).collect()
}

/// With the replace field shown, each match row shows the old text struck
/// through and the new text; Enter opens a diff of the whole file that is
/// exactly what Replace All writes.
#[test]
fn replace_preview_inline_and_as_a_diff() {
    let d = tempfile::tempdir().unwrap();
    files(d.path(), &[("a.txt", "keep\nfoo one\nbar\nfoo two\n"), ("b.txt", "nothing\n")]);
    let mut h = Harness::new();
    open_folder(&mut h, d.path());
    h.key("ctrl+shift+h");
    h.type_text("foo");
    h.key("enter");
    assert_eq!(result_files(&h), vec![("a.txt".into(), 2)]);
    assert!(!h.screen().contains("foobaz"), "no preview text yet");
    h.app.search.field = SearchField::Replace;
    h.type_text("baz");
    let screen = h.screen();
    assert!(screen.contains("foobaz one"), "struck old text then new text:\n{screen}");
    assert!(screen.contains("foobaz two"));
    // Exclude the second match: its row shows no preview.
    h.app.search.field = SearchField::Results;
    h.app.search.selected = 3; // root, file, match 1, match 2
    h.key("delete");
    let screen = h.screen();
    assert!(screen.contains("foobaz one") && !screen.contains("foobaz two"), "{screen}");
    // Enter on the first match opens the Replace Preview diff on its line.
    h.app.search.selected = 2;
    h.key("enter");
    let text = h.active_text();
    assert_eq!(
        text, "--- a/a.txt\n+++ b/a.txt\n@@ -1,4 +1,4 @@\n keep\n-foo one\n+baz one\n bar\n foo two\n",
        "excluded match left alone"
    );
    assert!(h.app.docs.name(h.app.active().unwrap().2).contains("(Replace Preview)"));
    assert_eq!(h.cursor().0, 5, "cursor on the new line of the match");
    // Replace All writes exactly the previewed text.
    h.key("ctrl+shift+f");
    h.app.search.field = SearchField::Results;
    h.key("ctrl+alt+enter");
    h.key("enter");
    assert_eq!(std::fs::read_to_string(d.path().join("a.txt")).unwrap(), "keep\nbaz one\nbar\nfoo two\n");
}

/// Results follow edits to open files and changes on disk without a new
/// search; exclusions survive unrelated edits.
#[test]
fn results_update_live_after_edits_and_disk_changes() {
    let d = tempfile::tempdir().unwrap();
    files(
        d.path(),
        &[("a.txt", "needle 1\nneedle 2\n"), ("b.txt", "needle\n"), ("c.txt", "hay\n"), (".gitignore", "ignored/\n")],
    );
    let mut h = Harness::new();
    open_folder(&mut h, d.path());
    search(&mut h, "needle");
    assert_eq!(result_files(&h), vec![("a.txt".into(), 2), ("b.txt".into(), 1)]);
    let seq = h.app.search.seq;
    // Exclude a.txt's second match.
    h.app.search.files[0].excluded.insert(1);

    // Edit an open file: c.txt now matches (from the unsaved buffer).
    h.app.open_file(&d.path().join("c.txt"), false);
    h.type_text("needle ");
    tick(&mut h);
    assert_eq!(h.app.search.seq, seq, "no new search, only an update");
    assert_eq!(result_files(&h), vec![("a.txt".into(), 2), ("b.txt".into(), 1), ("c.txt".into(), 1)]);
    assert!(h.app.search.files[2].from_buffer);
    // Reverting it: back to the disk content, no match.
    h.command("workbench.action.files.revert");
    tick(&mut h);
    assert_eq!(result_files(&h), vec![("a.txt".into(), 2), ("b.txt".into(), 1)]);

    // Disk: change, create, delete, and an ignored file.
    std::fs::write(d.path().join("a.txt"), "new first line\nneedle 1\nneedle 2\n").unwrap();
    std::fs::write(d.path().join("new.txt"), "a needle\n").unwrap();
    std::fs::remove_file(d.path().join("b.txt")).unwrap();
    files(d.path(), &[("ignored/x.txt", "needle\n")]);
    h.fs_changed(vec![
        d.path().join("a.txt"),
        d.path().join("new.txt"),
        d.path().join("b.txt"),
        d.path().join("ignored/x.txt"),
    ]);
    assert_eq!(h.app.search.seq, seq);
    assert_eq!(result_files(&h), vec![("a.txt".into(), 2), ("new.txt".into(), 1)]);
    let a = &h.app.search.files[0];
    assert_eq!(a.matches[0].line, 1, "line numbers follow the change");
    assert_eq!(a.excluded.iter().copied().collect::<Vec<_>>(), vec![1], "exclusion kept for the same match");
    assert!(h.app.search_status().contains("3 results in 2 files"), "{}", h.app.search_status());
}

/// Find in file follows edits without searching the whole file again:
/// matches move with the text and only the edited lines are searched.
#[test]
fn find_in_file_updates_only_edited_lines() {
    let d = tempfile::tempdir().unwrap();
    let text: String = (0..10_000).map(|i| format!("row {i} needle\n")).collect();
    files(d.path(), &[("big.txt", &text)]);
    let mut h = Harness::new();
    open_folder(&mut h, d.path());
    h.app.open_file(&d.path().join("big.txt"), false);
    h.key("ctrl+f");
    h.type_text("needle");
    assert_eq!(h.app.find.matches.len(), 10_000);
    let jobs = h.find_jobs;
    // Edit with the bar still open: add a match on line 5000, break one on
    // line 10, add a line, then undo the last edit.
    h.app.focus = yace_app::app::Focus::Editor;
    let goto = |h: &mut Harness, line: usize, column: usize| {
        h.app.execute("workbench.action.gotoLine", Some(&serde_json::json!({ "line": line, "column": column })));
        h.settle();
    };
    goto(&mut h, 5001, 1);
    h.type_text("needle ");
    goto(&mut h, 11, 8);
    h.key("delete delete delete");
    h.key("enter");
    h.key("ctrl+z");
    h.type_text("x\ny");
    let doc = h.app.docs.doc(h.app.active().unwrap().2).unwrap().text();
    let re = yace_search::find::build_regex("needle", Default::default()).unwrap();
    let expected = yace_search::find::find_in_text(&doc, &re, None).matches;
    assert_eq!(h.app.find.matches, expected);
    assert_eq!(h.app.find.matches.len(), 10_000, "one added, one broken");
    assert_eq!(h.find_jobs, jobs, "no whole-file search after the edits");
}

fn indexed(h: &Harness) -> Vec<String> {
    let key = h.app.workspace.roots()[0].id.0;
    h.engine.index.root(key).map(|r| r.files.iter().map(|f| f.to_string()).collect()).unwrap_or_default()
}

/// Once Quick Open is used, the index walks every root, watches every
/// folder it indexed, and follows new folders, new files and deletions
/// from file-system events alone, keeping ignore rules.
#[test]
fn quick_open_index_stays_live_without_rewalks() {
    let d = tempfile::tempdir().unwrap();
    files(d.path(), &[("src/main.rs", "fn main() {}\n"), (".gitignore", "node_modules/\n"), ("README.md", "hi\n")]);
    std::fs::create_dir_all(d.path().join("node_modules/dep")).unwrap();
    let mut h = Harness::new();
    open_folder(&mut h, d.path());
    assert_eq!(h.walks_run, 0, "nothing indexed (or watched) until Quick Open is used");
    assert!(!h.app.all_roots_live_for_tests());
    start_index(&mut h);
    let walks = h.walks_run;
    assert_eq!(walks, 1);
    assert!(h.app.all_roots_live_for_tests());
    let root = d.path().canonicalize().unwrap();
    assert!(h.app.watched_dirs().contains(&root.join("src")), "every indexed folder is watched");
    assert!(!h.app.watched_dirs().contains(&root.join("node_modules")), "ignored folders are not");

    // A new tree, reported only as its top folder.
    files(d.path(), &[("a/b/c/new.txt", "x\n"), ("a/b/other.rs", "y\n")]);
    h.fs_changed(vec![d.path().join("a")]);
    assert!(indexed(&h).contains(&"a/b/c/new.txt".to_string()), "{:?}", indexed(&h));
    assert!(h.app.watched_dirs().contains(&root.join("a/b/c")));
    // A new file, an ignored one, and one under an ignored folder.
    files(d.path(), &[("a/b/c/later.txt", ""), ("node_modules/dep/x.js", ""), ("src/debug.log", "")]);
    std::fs::write(d.path().join(".gitignore"), "node_modules/\n").unwrap();
    h.fs_changed(vec![d.path().join("a/b/c/later.txt"), d.path().join("node_modules/dep/x.js")]);
    let list = indexed(&h);
    assert!(list.contains(&"a/b/c/later.txt".to_string()));
    assert!(!list.iter().any(|f| f.starts_with("node_modules")), "{list:?}");
    let mut sorted = list.clone();
    sorted.sort();
    assert_eq!(list, sorted, "the list stays sorted");
    // Deleting a folder drops its files and its watches.
    std::fs::remove_dir_all(d.path().join("a/b")).unwrap();
    h.fs_changed(vec![d.path().join("a/b")]);
    assert!(!indexed(&h).iter().any(|f| f.starts_with("a/b")));
    assert!(!h.app.watched_dirs().contains(&root.join("a/b/c")));
    assert_eq!(h.walks_run, walks, "no re-walk for any of it");
    // Quick Open, even later, doesn't re-walk a live root.
    h.key("ctrl+p");
    h.type_text("main");
    h.key("escape");
    assert_eq!(h.walks_run, walks);
    // Changing an ignore file re-walks (which files count changed).
    h.fs_changed(vec![d.path().join(".gitignore")]);
    assert_eq!(h.walks_run, walks + 1);
    // Lost events: everything is walked again.
    h.app.handle(yace_app::app::AppEvent::FsRescan);
    h.settle();
    assert_eq!(h.walks_run, walks + 2);
}

/// A root with more folders than the watch budget isn't watched folder by
/// folder: it keeps the re-walk when Quick Open opens, with one notice.
#[test]
fn over_the_watch_budget_falls_back_to_rewalks() {
    let d = tempfile::tempdir().unwrap();
    files(d.path(), &[("a/1.txt", ""), ("b/2.txt", ""), ("c/3.txt", ""), ("d/4.txt", "")]);
    let mut h = Harness::new();
    h.app.set_watch_budget_for_tests(3);
    open_folder(&mut h, d.path());
    start_index(&mut h);
    assert!(!h.app.all_roots_live_for_tests());
    let root = d.path().canonicalize().unwrap();
    assert!(!h.app.watched_dirs().contains(&root.join("c")));
    let notices: Vec<String> = h.app.notification_log.iter().map(|(_, t)| t.clone()).collect();
    assert_eq!(notices.iter().filter(|t| t.contains("more than YACE can watch")).count(), 1, "{notices:?}");
}

/// Typing a longer literal query searches only the files that matched the
/// shorter one (the results are complete and kept live).
#[test]
fn a_longer_query_searches_only_previous_matches() {
    let d = tempfile::tempdir().unwrap();
    let mut list: Vec<(String, String)> =
        (0..50).map(|i| (format!("f{i}.txt"), "nothing here\n".to_string())).collect();
    list.push(("hit1.txt".into(), "needle\n".into()));
    list.push(("hit2.txt".into(), "need\n".into()));
    let refs: Vec<(&str, &str)> = list.iter().map(|(a, b)| (a.as_str(), b.as_str())).collect();
    files(d.path(), &refs);
    let mut h = Harness::new();
    open_folder(&mut h, d.path());
    start_index(&mut h);
    search(&mut h, "need");
    assert_eq!(h.last_search_only, Some(None), "first search walks everything");
    assert_eq!(h.app.search.files.len(), 2);
    h.type_text("le");
    tick(&mut h);
    assert_eq!(h.last_search_only, Some(Some(2)), "only the two earlier matches");
    assert_eq!(result_files(&h), vec![("hit1.txt".into(), 1)]);
    // A file that started matching meanwhile is found too (live updates).
    std::fs::write(d.path().join("f3.txt"), "needle\n").unwrap();
    h.fs_changed(vec![d.path().join("f3.txt")]);
    h.type_text("x");
    tick(&mut h);
    h.key("backspace");
    tick(&mut h);
    assert_eq!(h.last_search_only, Some(None), "a shorter query walks again");
    assert_eq!(result_files(&h), vec![("f3.txt".into(), 1), ("hit1.txt".into(), 1)]);
    // Regex and whole-word queries always walk.
    h.key("alt+r");
    tick(&mut h);
    h.type_text("x");
    tick(&mut h);
    assert_eq!(h.last_search_only, Some(None));
}
