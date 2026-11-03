//! M4: Quick Open, command palette, find/replace, workspace search,
//! navigation history and async loading.

mod common;

use std::fs;

use common::{Harness, multi_root};
use yace_app::{
    app::{Focus, SearchField, SidebarView},
    cli::{Launch, Target},
    initial_workspace,
    overlay::Overlay,
};

fn open_ws(h: &mut Harness, ws: &std::path::Path) {
    let launch =
        Launch { target: Target::WorkspaceFile(ws.to_path_buf()), add_roots: vec![], goto: None, restore: false };
    initial_workspace(&mut h.app, &launch);
    h.settle();
}

fn picker_rows(h: &Harness) -> Vec<(String, String, String)> {
    match h.app.overlays.last() {
        Some(Overlay::Picker(p)) => p
            .filtered
            .iter()
            .map(|i| (p.items[*i].label.clone(), p.items[*i].detail.clone(), p.items[*i].hint.clone()))
            .collect(),
        _ => vec![],
    }
}

fn active_name(h: &Harness) -> String {
    h.app.docs.name(h.app.active().unwrap().2)
}

fn cursor(h: &Harness) -> (usize, usize) {
    let (g, v, d) = h.app.active().unwrap();
    let rope = h.app.docs.doc(d).unwrap().rope().clone();
    let head = h.app.layout.group(g).unwrap().views[v].sel.primary().head;
    let line = rope.char_to_line(head);
    (line, head - rope.line_to_char(line))
}

#[test]
fn quick_open_finds_files_in_every_root_and_shows_root_identity() {
    let m = multi_root();
    let mut h = Harness::new();
    open_ws(&mut h, &m.ws_file);
    h.key("ctrl+p");
    assert!(matches!(h.app.overlays.last(), Some(Overlay::Picker(_))));
    h.type_text("index");
    let rows = picker_rows(&h);
    let index_rows: Vec<_> = rows.iter().filter(|r| r.0 == "index.ts").collect();
    assert_eq!(index_rows.len(), 3, "{rows:?}");
    let roots: Vec<&str> = index_rows.iter().map(|r| r.2.as_str()).collect();
    for r in ["GLOBAL API", "LEARN APP", "CHAT API"] {
        assert!(roots.contains(&r), "{roots:?}");
    }
    assert!(index_rows.iter().all(|r| r.1 == "src/"));
    // Fuzzy: jobserv → jobs.service.ts
    h.key("ctrl+a");
    h.type_text("jobserv");
    assert_eq!(picker_rows(&h)[0].0, "jobs.service.ts");
    h.key("enter");
    assert_eq!(active_name(&h), "jobs.service.ts");
    assert!(h.app.overlays.is_empty());
    // Picked results rank first next time (recency).
    h.key("ctrl+p");
    assert_eq!(picker_rows(&h)[0].0, "jobs.service.ts", "recent files shown for an empty query");
}

#[test]
fn quick_open_shows_partial_results_while_indexing() {
    let m = multi_root();
    let mut h = Harness::new();
    h.defer_walks = true;
    open_ws(&mut h, &m.ws_file);
    h.key("ctrl+p");
    assert!(!h.pending_walks.is_empty(), "walks requested lazily on first Quick Open");
    h.type_text("readme");
    assert!(picker_rows(&h).is_empty());
    let chat = h.app.workspace.roots().iter().find(|r| r.display_name() == "Chat API").unwrap().id.0;
    h.feed_walk_batch(chat, &["README.md"]);
    let rows = picker_rows(&h);
    assert_eq!(rows.len(), 1, "results from a partial index: {rows:?}");
    assert!(matches!(h.app.overlays.last(), Some(Overlay::Picker(p)) if p.title.contains("indexing")));
}

#[test]
fn quick_open_line_and_column_modes() {
    let m = multi_root();
    fs::write(m.api.join("src/index.ts"), "line1\nline2 abcdef\nline3\nline4\nline5\n").unwrap();
    let mut h = Harness::new();
    open_ws(&mut h, &m.ws_file);
    h.key("ctrl+p");
    h.type_text("api index.ts:2:3");
    let rows = picker_rows(&h);
    assert_eq!(rows[0].2, "GLOBAL API", "{rows:?}");
    h.key("enter");
    assert_eq!(cursor(&h), (1, 2));
    h.key("ctrl+p");
    h.type_text(":5");
    h.key("enter");
    assert_eq!(cursor(&h).0, 4);
    // ? lists modes; > runs commands.
    h.key("ctrl+p");
    h.type_text("?");
    assert!(picker_rows(&h).iter().any(|r| r.0.contains("Show and Run Commands")));
    h.key("escape");
    h.key("ctrl+p");
    h.type_text(">toggle side bar");
    assert!(picker_rows(&h)[0].0.contains("Toggle Primary Side Bar"));
    h.key("enter");
    assert!(!h.app.sidebar_visible);
}

#[test]
fn command_palette_is_fuzzy_and_remembers_recent() {
    let mut h = Harness::new();
    h.key("ctrl+shift+p");
    h.type_text("splt rght");
    assert_eq!(picker_rows(&h)[0].0, "View: Split Editor Right");
    h.key("enter");
    h.key("ctrl+shift+p");
    let rows = picker_rows(&h);
    assert_eq!(rows[0].0, "View: Split Editor Right", "recently used first");
    assert!(rows[0].2.contains("recently used"));
}

#[test]
fn find_widget_counts_navigates_replaces_and_multicursors() {
    let d = tempfile::tempdir().unwrap();
    let f = d.path().join("a.txt");
    fs::write(&f, "foo bar foo\nFoo baz foo\n").unwrap();
    let mut h = Harness::new();
    initial_workspace(
        &mut h.app,
        &Launch {
            target: Target::File { path: f.clone(), line: None, col: None },
            add_roots: vec![],
            goto: None,
            restore: false,
        },
    );
    h.settle();
    h.key("ctrl+f");
    assert_eq!(h.app.focus, Focus::Find);
    h.type_text("foo");
    assert_eq!(h.app.find.matches.len(), 4, "case-insensitive by default");
    assert_eq!(h.app.find.status(), "1 of 4");
    h.key("alt+c");
    assert_eq!(h.app.find.matches.len(), 3);
    h.key("enter");
    assert_eq!(h.app.find.status(), "2 of 3");
    h.key("shift+enter shift+enter");
    assert_eq!(h.app.find.status(), "3 of 3", "previous wraps around");
    // Replace all is a single undo step.
    h.key("ctrl+h");
    h.type_text("qux");
    h.key("ctrl+alt+enter");
    assert_eq!(h.active_text(), "qux bar qux\nFoo baz qux\n");
    h.key("escape");
    h.key("ctrl+z");
    assert_eq!(h.active_text(), "foo bar foo\nFoo baz foo\n");
    // Alt+Enter: a cursor per match.
    h.key("ctrl+f");
    h.key("alt+enter");
    let (g, v, _) = h.app.active().unwrap();
    assert_eq!(h.app.layout.group(g).unwrap().views[v].sel.len(), 3);
    assert!(!h.app.find.visible);
    // Regex with capture groups and replace one.
    h.key("escape ctrl+home");
    h.key("ctrl+h");
    assert_eq!(h.app.find.field, yace_app::app::FindField::Replace, "existing query: Ctrl+H focuses Replace");
    h.key("tab");
    h.key("ctrl+a");
    h.type_text("(b)a(r|z)");
    h.key("alt+r");
    h.key("tab");
    h.key("ctrl+a");
    h.type_text("$2$1");
    h.key("ctrl+shift+1");
    assert_eq!(h.active_text(), "foo rb foo\nFoo baz foo\n");
    assert_eq!(h.app.find.matches.len(), 1, "matches refreshed after the edit");
}

#[test]
fn workspace_search_scopes_groups_opens_and_replaces() {
    let m = multi_root();
    let mut h = Harness::new();
    open_ws(&mut h, &m.ws_file);
    h.key("ctrl+shift+f");
    assert_eq!(h.app.sidebar_view, SidebarView::Search);
    h.type_text("export");
    h.key("enter");
    let roots: Vec<u32> = h.app.search.files.iter().map(|f| f.root).collect();
    assert!(roots.len() >= 3, "{roots:?}");
    let order: Vec<u32> = h.app.workspace.roots().iter().map(|r| r.id.0).collect();
    let mut sorted = roots.clone();
    sorted.sort_by_key(|r| order.iter().position(|o| o == r));
    assert_eq!(roots, sorted, "grouped by root in workspace order");
    assert!(h.screen().contains("GLOBAL API"));
    // Scope to one root.
    h.command("workbench.action.search.toggleQueryDetails");
    h.app.search.field = SearchField::Include;
    h.type_text("./Learn App");
    h.key("enter");
    let learn = h.app.workspace.roots().iter().find(|r| r.display_name() == "Learn App").unwrap().id.0;
    assert!(!h.app.search.files.is_empty());
    assert!(h.app.search.files.iter().all(|f| f.root == learn));
    // Open a result at the right line.
    h.app.search.field = SearchField::Results;
    h.app.search.selected = 2; // root, file, first match
    h.key("enter");
    assert_eq!(h.app.focus, Focus::Editor);
    assert_eq!(cursor(&h).0, 0);

    // Replace across roots: open+dirty buffer is edited (not saved), closed files are written.
    h.app.search.include = Default::default();
    let api_index = m.api.join("src/index.ts");
    h.app.open_file(&api_index, false);
    h.type_text("// dirty\n");
    h.key("ctrl+shift+h");
    h.key("ctrl+a");
    h.type_text("export const");
    h.key("enter");
    h.app.search.field = SearchField::Replace;
    h.type_text("const");
    h.app.search.field = SearchField::Results;
    h.key("ctrl+alt+enter");
    assert!(matches!(h.app.overlays.last(), Some(Overlay::Confirm(c)) if c.title == "Replace in files"));
    h.key("enter");
    let d = h.app.docs.find_by_path(&api_index).unwrap();
    let doc = h.app.docs.doc(d).unwrap();
    assert_eq!(doc.text(), "// dirty\nconst api = 1;\n");
    assert!(doc.is_dirty(), "open buffer changed but not saved");
    assert_eq!(fs::read_to_string(&api_index).unwrap(), "export const api = 1;\n", "disk untouched for the open file");
    assert_eq!(fs::read_to_string(m.web.join("src/index.ts")).unwrap(), "const web = 2;\n", "closed file written");
}

#[test]
fn search_respects_gitignore_and_dirty_buffers() {
    let m = multi_root();
    fs::write(m.api.join(".gitignore"), "dist/\n").unwrap();
    fs::create_dir_all(m.api.join("dist")).unwrap();
    fs::write(m.api.join("dist/out.js"), "needle\n").unwrap();
    let mut h = Harness::new();
    open_ws(&mut h, &m.ws_file);
    h.app.open_file(&m.chat.join("README.md"), false);
    h.type_text("needle ");
    h.key("ctrl+shift+f");
    h.type_text("needle");
    h.key("enter");
    let rels: Vec<&str> = h.app.search.files.iter().map(|f| f.rel.as_str()).collect();
    assert_eq!(rels, vec!["README.md"], "ignored dist/ skipped; unsaved buffer content searched");
    assert!(h.app.search.files[0].from_buffer);
}

#[test]
fn navigation_back_and_forward_across_roots() {
    let m = multi_root();
    let mut h = Harness::new();
    open_ws(&mut h, &m.ws_file);
    h.app.open_file(&m.api.join("package.json"), false);
    h.key("ctrl+p");
    h.type_text("next.config");
    h.key("enter");
    assert_eq!(active_name(&h), "next.config.ts");
    h.key("alt+left");
    assert_eq!(active_name(&h), "package.json");
    h.key("alt+right");
    assert_eq!(active_name(&h), "next.config.ts");
}

#[test]
fn large_files_load_in_the_background() {
    let d = tempfile::tempdir().unwrap();
    let f = d.path().join("big.log");
    let content = "0123456789abcdef\n".repeat(100_000);
    fs::write(&f, &content).unwrap();
    let mut h = Harness::new();
    h.defer_loads = true;
    open_folder(&mut h, d.path());
    h.app.open_file(&f, false);
    h.settle();
    let (_, _, doc) = h.app.active().unwrap();
    assert!(h.app.docs.is_loading(doc));
    assert!(h.screen().contains("Loading big.log"));
    h.type_text("x");
    assert_eq!(h.app.docs.doc(doc).unwrap().len_chars(), 0, "read-only while loading");
    h.app.goto_line_col(41, 3);
    for ev in std::mem::take(&mut h.pending_loads) {
        h.app.handle(ev);
    }
    assert!(!h.app.docs.is_loading(doc));
    assert_eq!(h.app.docs.doc(doc).unwrap().len_chars(), content.chars().count());
    assert_eq!(cursor(&h), (41, 3), "requested position applied after load");
}

fn open_folder(h: &mut Harness, dir: &std::path::Path) {
    let launch = Launch { target: Target::Folder(dir.to_path_buf()), add_roots: vec![], goto: None, restore: false };
    initial_workspace(&mut h.app, &launch);
    h.settle();
}
