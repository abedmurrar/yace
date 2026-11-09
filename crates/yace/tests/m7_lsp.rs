//! M7: language servers, end to end against a fake server that speaks real
//! JSON-RPC over stdio (`tests/fixtures/fake_lsp.py`).

mod common;

use std::{path::Path, time::Duration};

use common::{Harness, multi_root};
use yace_app::{
    cli::{Launch, Target},
    initial_workspace,
    overlay::Overlay,
};

const T: Duration = Duration::from_secs(10);

fn have_python() -> bool {
    yace_lsp::registry::which("python3").is_some()
}

fn fake() -> String {
    format!("{}/tests/fixtures/fake_lsp.py", env!("CARGO_MANIFEST_DIR"))
}

/// Config with the fake server for TypeScript; `extra_env` adds env vars.
fn config(log: &Path, extra_env: &str, extra: &str) -> String {
    format!(
        r#"[lsp]
disabled = ["typescript-language-server"]
idleShutdownSecs = 0
[[lsp.servers]]
name = "fake"
command = "{cmd}"
languages = ["typescript"]
env = {{ FAKE_LSP_LOG = "{log}"{extra_env} }}
settings = {{ fake = {{ answer = 42 }} }}
{extra}"#,
        cmd = fake(),
        log = log.display()
    )
}

fn open_folder(h: &mut Harness, dir: &Path) {
    initial_workspace(
        &mut h.app,
        &Launch { target: Target::Folder(dir.to_path_buf()), add_roots: vec![], goto: None, restore: false },
    );
    h.settle();
}

fn log_has(log: &Path, needle: &str) -> bool {
    std::fs::read_to_string(log).unwrap_or_default().contains(needle)
}

/// Wait until the active document's server is running.
fn wait_running(h: &mut Harness) -> bool {
    h.wait_for(T, |h| h.app.lsp_status().is_some_and(|s| s == "fake"))
}

struct Fixture {
    _dir: tempfile::TempDir,
    root: std::path::PathBuf,
    log: std::path::PathBuf,
}

fn fixture(files: &[(&str, &str)]) -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("proj");
    for (rel, text) in files {
        let p = root.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, text).unwrap();
    }
    let log = dir.path().join("lsp.log");
    Fixture { _dir: dir, root, log }
}

#[test]
fn lazy_start_sync_diagnostics_and_problems() {
    if !have_python() {
        return;
    }
    let f = fixture(&[("a.ts", "let a = 1; // TODO\n"), ("b.txt", "plain\n")]);
    let mut h = Harness::with_config(&config(&f.log, "", ""));
    open_folder(&mut h, &f.root);
    assert!(h.app.lsp.servers.is_empty(), "nothing starts before a file of its language opens");
    h.app.open_file(&f.root.join("b.txt"), false);
    h.settle();
    assert!(h.app.lsp.servers.is_empty(), "plain text has no server");
    h.app.open_file(&f.root.join("a.ts"), false);
    h.settle();
    assert!(wait_running(&mut h), "server started lazily");
    assert!(h.wait_for(T, |h| h.app.lsp.counts() == (0, 1)), "TODO warning published");
    assert!(log_has(&f.log, "\"textDocument/didOpen\""));
    assert!(h.wait_for(T, |_| log_has(&f.log, "\"answer\": 42")), "workspace/configuration answered from settings");
    let screen = h.screen();
    assert!(screen.contains("⚠ 1"), "status bar counts:\n{screen}");
    // Typing syncs incrementally; the server's copy matches exactly, so the
    // error it reports sits exactly where we typed it.
    h.key("ctrl+end");
    h.type_text("ERROR");
    assert!(h.wait_for(T, |h| h.app.lsp.counts() == (1, 1)));
    assert!(log_has(&f.log, "\"contentChanges\": [{\"range\""), "incremental didChange");
    let d = h.app.active().unwrap().2;
    let rope = h.app.docs.doc(d).unwrap().rope().clone();
    let err = h.app.doc_diagnostics(d).iter().find(|x| x.severity == 1).unwrap().chars.unwrap();
    assert_eq!(rope.slice(err.0..err.1).to_string(), "ERROR");
    // The message under the cursor shows in the status bar.
    h.key("left");
    assert!(h.screen().contains("ERROR found"), "{}", h.screen());
    // Problems picker lists both; Enter jumps.
    h.key("ctrl+shift+m");
    let Some(Overlay::Picker(p)) = h.app.overlays.last() else { panic!("problems picker") };
    assert_eq!(p.items.len(), 2);
    h.key("enter");
    assert!(h.app.overlays.is_empty());
    // F8 cycles through problems in the file.
    h.key("ctrl+home");
    h.key("f8");
    let (line, col) = h.cursor();
    assert_eq!((line, col), (0, 14), "first problem (TODO)");
}

#[test]
fn completion_filter_accept_snippet_and_auto_import() {
    if !have_python() {
        return;
    }
    let f = fixture(&[("a.ts", "first\n")]);
    let mut h = Harness::with_config(&config(&f.log, "", ""));
    open_folder(&mut h, &f.root);
    h.app.open_file(&f.root.join("a.ts"), false);
    h.settle();
    assert!(wait_running(&mut h));
    h.key("ctrl+end");
    h.type_text("x.");
    // '.' is a trigger character.
    assert!(h.wait_for(T, |h| h.app.completion.is_some()), "completion on trigger char");
    assert_eq!(h.app.completion.as_ref().unwrap().shown.len(), 3);
    assert!(h.screen().contains("alpha"));
    // Typing filters locally.
    h.type_text("al");
    let c = h.app.completion.as_ref().expect("still open");
    assert_eq!(c.shown.len(), 1, "only alpha matches 'al'");
    h.key("enter");
    assert!(h.app.completion.is_none());
    assert_eq!(h.active_text(), "first\nx.alpha");
    // Snippet: placeholder text selected.
    h.type_text(" ");
    h.command("editor.action.triggerSuggest");
    assert!(h.wait_for(T, |h| h.app.completion.is_some()));
    h.type_text("be");
    h.key("tab");
    assert_eq!(h.active_text(), "first\nx.alpha beta(x)");
    let (g, v, _) = h.app.active().unwrap();
    let sel = h.app.layout.group(g).unwrap().views[v].sel.primary();
    assert_eq!(sel.to() - sel.from(), 1, "x is selected");
    // Auto-import arrives through completionItem/resolve.
    h.key("end");
    h.type_text(" ");
    h.command("editor.action.triggerSuggest");
    assert!(h.wait_for(T, |h| h.app.completion.is_some()));
    h.type_text("gam");
    h.key("enter");
    assert!(h.wait_for(T, |h| h.active_text().starts_with("import gamma;\n")), "{}", h.active_text());
    assert!(h.active_text().ends_with("beta(x) gamma"));
    // Escape closes the list without touching the text.
    h.type_text(" ");
    h.command("editor.action.triggerSuggest");
    assert!(h.wait_for(T, |h| h.app.completion.is_some()));
    let before = h.active_text();
    h.key("escape");
    assert!(h.app.completion.is_none());
    assert_eq!(h.active_text(), before);
}

#[test]
fn hover_signature_help_and_stale_answers() {
    if !have_python() {
        return;
    }
    let f = fixture(&[("a.ts", "value\n")]);
    let mut h = Harness::with_config(&config(&f.log, "", ""));
    open_folder(&mut h, &f.root);
    h.app.open_file(&f.root.join("a.ts"), false);
    h.settle();
    assert!(wait_running(&mut h));
    h.key("ctrl+k ctrl+i");
    assert!(h.wait_for(T, |h| h.app.hover.is_some()));
    assert_eq!(h.app.hover.as_ref().unwrap().lines, ["let value:  number", "", "hover for value", "- item"]);
    let screen = h.screen();
    for row in ["let value:  number", "hover for value", "- item"] {
        assert!(screen.lines().any(|l| l.contains(row)), "{row:?} on its own row:\n{screen}");
    }
    h.key("escape");
    assert!(h.app.hover.is_none());
    // Without the enhanced keyboard protocol Ctrl+I arrives as Tab.
    h.app.keyboard_enhanced = false;
    h.key("ctrl+k");
    h.key("tab");
    assert!(h.wait_for(T, |h| h.app.hover.is_some()), "Ctrl+K Tab shows the hover");
    assert_eq!(h.active_text(), "value\n", "the Tab was not typed");
    h.key("escape");
    h.app.keyboard_enhanced = true;
    // Signature help on '(' and ',' with the active parameter.
    h.key("ctrl+end");
    h.type_text("f(");
    assert!(h.wait_for(T, |h| h.app.signature.is_some()));
    assert!(h.screen().contains("f(a: int, b: str)"));
    assert_eq!(h.app.signature.as_ref().unwrap().active, Some((2, 8)));
    h.type_text("1,");
    assert!(h.wait_for(T, |h| h.app.signature.as_ref().is_some_and(|s| s.active == Some((10, 16)))));
    h.type_text(")");
    assert!(h.app.signature.is_none());
    // An answer for an older version of the document is dropped.
    h.app.execute("editor.action.showHover", None);
    h.app.type_text_for_tests("z");
    std::thread::sleep(Duration::from_millis(300));
    h.settle();
    assert!(h.app.hover.is_none(), "stale hover ignored");
}

#[test]
fn definition_across_roots_references_and_rename() {
    if !have_python() {
        return;
    }
    let m = multi_root();
    let dir = tempfile::tempdir().unwrap();
    let log = dir.path().join("lsp.log");
    // Definition: a file in another root.
    let target = m.web.join("src/index.ts");
    let env = format!(", FAKE_LSP_DEF = \"{}:0:13\"", target.display());
    let mut h = Harness::with_config(&config(&log, &env, ""));
    initial_workspace(
        &mut h.app,
        &Launch { target: Target::WorkspaceFile(m.ws_file.clone()), add_roots: vec![], goto: None, restore: false },
    );
    h.settle();
    std::fs::write(m.api.join("src/jobs.service.ts"), "class Jobs {}\nconst j = new Jobs();\n").unwrap();
    std::fs::write(m.api.join("src/other.ts"), "import { Jobs } from './jobs.service';\n").unwrap();
    h.app.open_file(&m.api.join("src/jobs.service.ts"), false);
    h.settle();
    assert!(wait_running(&mut h));
    // One server per root.
    h.app.open_file(&m.web.join("next.config.ts"), false);
    h.settle();
    assert!(h.wait_for(T, |h| h.app.lsp.servers.len() == 2));
    let roots: Vec<_> = h.app.lsp.servers.iter().map(|s| s.root.clone()).collect();
    assert!(roots.contains(&m.api) && roots.contains(&m.web), "{roots:?}");
    h.app.open_file(&m.api.join("src/jobs.service.ts"), false);
    h.settle();
    h.key("ctrl+home");
    h.key("f12");
    assert!(
        h.wait_for(T, |h| h.app.active().and_then(|(_, _, d)| h.app.docs.doc(d)?.path().map(|p| p.to_path_buf()))
            == Some(target.clone())),
        "jumped into the other root"
    );
    assert_eq!(h.cursor(), (0, 13));
    // Back to where we were (navigation history).
    h.command("workbench.action.navigateBack");
    assert!(h.app.docs.doc(h.app.active().unwrap().2).unwrap().path().unwrap().ends_with("jobs.service.ts"));
    // References: a picker with every occurrence (open and closed files).
    h.key("ctrl+home");
    h.key("right right right right right right right"); // on "Jobs"
    h.key("shift+f12");
    assert!(h.wait_for(T, |h| matches!(h.app.overlays.last(), Some(Overlay::Picker(_)))));
    let Some(Overlay::Picker(p)) = h.app.overlays.last() else { unreachable!() };
    assert_eq!(p.items.len(), 3, "{:?}", p.items.iter().map(|i| &i.detail).collect::<Vec<_>>());
    h.key("escape");
    // Rename: open document edited (undoable), closed file written to disk.
    h.key("f2");
    let Some(Overlay::Prompt(pr)) = h.app.overlays.last() else { panic!("rename prompt") };
    assert_eq!(pr.input.text, "Jobs");
    h.type_text("Tasks");
    h.key("enter");
    assert!(h.wait_for(T, |h| h.active_text().contains("class Tasks")));
    assert!(h.active_text().contains("new Tasks()"));
    assert!(h.app.docs.doc(h.app.active().unwrap().2).unwrap().is_dirty(), "left unsaved");
    assert!(std::fs::read_to_string(m.api.join("src/other.ts")).unwrap().contains("{ Tasks }"));
    h.key("ctrl+z");
    assert!(h.active_text().contains("class Jobs"), "one undo reverts the rename in the editor");
}

#[test]
fn symbols_code_actions_and_format_on_save() {
    if !have_python() {
        return;
    }
    let f = fixture(&[
        ("a.ts", "class Box {}\ndef alpha() {}\ndef beta() {}   \nlet t = 'TODO';\n"),
        ("lib/util.ts", "def helper() {}\n"),
    ]);
    let mut h = Harness::with_config(&config(&f.log, "", "[editor]\nformatOnSave = true\n"));
    open_folder(&mut h, &f.root);
    h.app.open_file(&f.root.join("a.ts"), false);
    h.settle();
    assert!(wait_running(&mut h));
    // Document symbols via Ctrl+Shift+O (Quick Open "@").
    h.key("ctrl+shift+o");
    assert!(h.wait_for(T, |h| matches!(h.app.overlays.last(), Some(Overlay::Picker(p)) if p.items.len() == 3)));
    h.type_text("bet");
    let Some(Overlay::Picker(p)) = h.app.overlays.last() else { unreachable!() };
    assert_eq!(p.current().unwrap().label, "beta");
    h.key("enter");
    assert_eq!(h.cursor(), (2, 4));
    // Workspace symbols via Ctrl+T (Quick Open "#").
    h.key("ctrl+t");
    h.type_text("help");
    assert!(h.wait_for(T, |h| matches!(h.app.overlays.last(), Some(Overlay::Picker(p)) if p.items.len() == 1 && p.items[0].label == "helper")));
    h.key("enter");
    assert!(h.app.docs.doc(h.app.active().unwrap().2).unwrap().path().unwrap().ends_with("lib/util.ts"));
    // Code action on the TODO.
    h.app.open_file(&f.root.join("a.ts"), false);
    h.settle();
    h.key("ctrl+end");
    h.key("up");
    h.key("ctrl+.");
    assert!(h.wait_for(T, |h| matches!(h.app.overlays.last(), Some(Overlay::Picker(p)) if p.items.iter().any(|i| i.label == "Replace TODO with DONE"))));
    h.key("enter");
    assert!(h.active_text().contains("'DONE'"));
    // Format on save strips the trailing spaces before writing.
    h.key("ctrl+s");
    assert!(h.wait_for(T, |_| std::fs::read_to_string(f.root.join("a.ts")).unwrap().contains("def beta() {}\n")));
    assert!(!h.app.docs.doc(h.app.active().unwrap().2).unwrap().is_dirty());
}

#[test]
fn crash_restarts_and_missing_server_hint_once() {
    if !have_python() {
        return;
    }
    let f = fixture(&[("a.ts", "ok\n"), ("p.py", "x = 1\n"), ("q.py", "y = 2\n")]);
    let extra =
        "[[lsp.servers]]\nname = \"nope\"\ncommand = \"definitely-missing-yace-server\"\nlanguages = [\"python\"]\n";
    let mut cfg = config(&f.log, "", extra);
    cfg = cfg.replace(
        "disabled = [\"typescript-language-server\"]",
        "disabled = [\"typescript-language-server\", \"pyright\", \"pylsp\"]",
    );
    let mut h = Harness::with_config(&cfg);
    open_folder(&mut h, &f.root);
    h.app.open_file(&f.root.join("a.ts"), false);
    h.settle();
    assert!(wait_running(&mut h));
    let pid = h.app.lsp.servers[0].client.as_ref().unwrap().pid();
    h.key("ctrl+end");
    h.type_text("CRASH");
    assert!(
        h.wait_for(T, |h| h.app.notification_log.iter().any(|(_, m)| m.contains("stopped unexpectedly"))),
        "crash reported"
    );
    assert!(wait_running(&mut h), "restarted");
    let new_pid = h.app.lsp.servers[0].client.as_ref().unwrap().pid();
    assert_ne!(pid, new_pid);
    // A missing server: one hint per language, then silence.
    h.app.open_file(&f.root.join("p.py"), false);
    h.settle();
    h.app.open_file(&f.root.join("q.py"), false);
    h.settle();
    let hints = h.app.notification_log.iter().filter(|(_, m)| m.contains("No language server for Python")).count();
    assert_eq!(hints, 1);
    // The running servers report includes the server's memory.
    h.command("yace.lsp.showRunning");
    let Some(Overlay::Info { lines, .. }) = h.app.overlays.last() else { panic!("info overlay") };
    assert!(lines[0].starts_with("fake") && lines[0].contains("MiB"), "{lines:?}");
}

#[test]
fn completion_filtering_stays_fast_with_large_lists() {
    if !have_python() {
        return;
    }
    let f = fixture(&[("a.ts", "\n")]);
    let mut h = Harness::with_config(&config(&f.log, ", FAKE_LSP_ITEMS = \"5000\"", ""));
    open_folder(&mut h, &f.root);
    h.app.open_file(&f.root.join("a.ts"), false);
    h.settle();
    assert!(wait_running(&mut h));
    h.command("editor.action.triggerSuggest");
    assert!(h.wait_for(T, |h| h.app.completion.as_ref().is_some_and(|c| c.items.len() == 5003)));
    // Each keystroke re-filters 5,003 items locally.
    let mut worst = Duration::ZERO;
    for ch in "itemva".chars() {
        let t0 = std::time::Instant::now();
        h.app.type_text_for_tests(&ch.to_string());
        h.app.refilter_for_tests();
        worst = worst.max(t0.elapsed());
    }
    let c = h.app.completion.as_ref().expect("still open");
    assert!(!c.shown.is_empty());
    eprintln!("worst keystroke with 5,003 completion items: {worst:?}");
    // Generous bound so debug builds pass; release numbers are in TODO.md.
    assert!(worst < Duration::from_millis(if cfg!(debug_assertions) { 400 } else { 16 }), "{worst:?}");
}

#[test]
fn a_server_that_exits_during_startup_is_explained_not_restarted() {
    let dir = tempfile::tempdir().unwrap();
    let bin = dir.path().join("broken-server");
    std::fs::write(&bin, "#!/bin/sh\necho \"error: Unknown binary 'broken-server'\" >&2\nexit 1\n").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    std::fs::write(dir.path().join("a.ts"), "x\n").unwrap();
    let cfg = format!(
        "[lsp]\ndisabled = [\"typescript-language-server\"]\n[[lsp.servers]]\nname = \"broken\"\ncommand = \"{}\"\nlanguages = [\"typescript\"]\n",
        bin.display()
    );
    let mut h = Harness::with_config(&cfg);
    open_folder(&mut h, dir.path());
    h.app.open_file(&dir.path().join("a.ts"), false);
    h.settle();
    assert!(h.wait_for(T, |h| h.app.notification_log.iter().any(|(_, m)| m.contains("broken could not start"))));
    let msg = h.app.notification_log.iter().find(|(_, m)| m.contains("could not start")).unwrap().1.clone();
    assert!(msg.contains("Unknown binary"), "stderr explains why: {msg}");
    std::thread::sleep(Duration::from_millis(200));
    h.settle();
    assert!(!h.app.notification_log.iter().any(|(_, m)| m.contains("Restarting")), "no restart loop");
    assert_eq!(h.app.lsp_status().as_deref(), Some("broken (stopped)"));
}

/// Real server smoke test (typescript-language-server). Runs when
/// `YACE_TEST_TSLS` points at the server binary; `YACE_TEST_NODE_DIR` adds
/// Node.js to its PATH if needed.
#[test]
fn real_typescript_language_server() {
    let Ok(server) = std::env::var("YACE_TEST_TSLS") else { return };
    let path = match std::env::var("YACE_TEST_NODE_DIR") {
        Ok(d) => format!("{d}:{}", std::env::var("PATH").unwrap_or_default()),
        Err(_) => std::env::var("PATH").unwrap_or_default(),
    };
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("app");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("a.ts"), "export function greet(name: string): string {\n  return name;\n}\n").unwrap();
    std::fs::write(root.join("b.ts"), "import { greet } from './a';\ngreet(1);\n").unwrap();
    // YACE_TEST_TSLS_ARGS: e.g. "--lsp --stdio" for TypeScript 7's native server.
    let args: Vec<String> = std::env::var("YACE_TEST_TSLS_ARGS")
        .unwrap_or_else(|_| "--stdio".into())
        .split_whitespace()
        .map(|a| format!("\"{a}\""))
        .collect();
    let args = args.join(", ");
    let cfg = format!(
        "[lsp]\n[[lsp.servers]]\nname = \"typescript-language-server\"\ncommand = \"{server}\"\nargs = [{args}]\nlanguages = [\"typescript\"]\nenv = {{ PATH = \"{path}\" }}\n"
    );
    let mut h = Harness::with_config(&cfg);
    open_folder(&mut h, &root);
    h.app.open_file(&root.join("b.ts"), false);
    h.settle();
    let t = Duration::from_secs(60);
    assert!(h.wait_for(t, |h| h.app.lsp.counts().0 >= 1), "type error reported");
    let d = h.app.active().unwrap().2;
    assert!(h.app.doc_diagnostics(d).iter().any(|x| x.message.contains("not assignable")));
    // Go to definition into a.ts.
    h.key("down");
    h.key("home");
    h.key("f12");
    assert!(h.wait_for(t, |h| {
        h.app.active().and_then(|(_, _, d)| h.app.docs.doc(d)?.path().map(|p| p.ends_with("a.ts"))).unwrap_or(false)
    }));
    // Completion while typing an identifier (quick suggestions).
    h.app.open_file(&root.join("b.ts"), false);
    h.settle();
    h.key("ctrl+end");
    h.type_text("gre");
    h.command("editor.action.triggerSuggest");
    assert!(h.wait_for(t, |h| {
        h.app.completion.as_ref().is_some_and(|c| c.shown.iter().any(|i| c.items[*i].label == "greet"))
    }));
    h.command("yace.lsp.showRunning");
    if let Some(Overlay::Info { lines, .. }) = h.app.overlays.last() {
        eprintln!("{}", lines.join("\n"));
    }
    if std::env::var("YACE_TEST_HOLD").is_ok() {
        let pid = h.app.lsp.servers[0].client.as_ref().unwrap().pid();
        let out = std::process::Command::new("ps").args(["-eo", "pid,pgid,rss,args"]).output().unwrap();
        for l in String::from_utf8_lossy(&out.stdout).lines() {
            if l.split_whitespace().nth(1) == Some(&pid.to_string()) {
                eprintln!("PS {l}");
            }
        }
    }
}

#[test]
fn notification_history_is_searchable_and_copyable() {
    let mut h = Harness::new();
    h.app.notify_sticky(
        yace_app::app::Severity::Info,
        "No language server for TypeScript found. For completion and navigation run: npm install -g typescript typescript-language-server",
    );
    h.app.notify(yace_app::app::Severity::Info, "something else");
    h.key("ctrl+k ctrl+n");
    let Some(Overlay::Picker(p)) = h.app.overlays.last() else { panic!("history picker") };
    assert_eq!(p.items.len(), 2);
    h.type_text("typescript");
    h.key("enter");
    assert!(h.app.clipboard.get().text.contains("npm install -g typescript typescript-language-server"));
    // The whole history can also be opened as text.
    h.command("yace.notificationsInEditor");
    assert!(h.active_text().contains("[info] No language server for TypeScript"));
}

#[test]
fn pull_diagnostics_like_typescript_7() {
    if !have_python() {
        return;
    }
    let f = fixture(&[("a.ts", "let a = 1; // TODO\n")]);
    let mut h = Harness::with_config(&config(&f.log, ", FAKE_LSP_PULL = \"1\"", ""));
    open_folder(&mut h, &f.root);
    h.app.open_file(&f.root.join("a.ts"), false);
    h.settle();
    assert!(wait_running(&mut h));
    assert!(h.wait_for(T, |h| h.app.lsp.counts() == (0, 1)), "pulled after open");
    assert!(log_has(&f.log, "\"textDocument/diagnostic\""), "YACE pulled");
    h.key("ctrl+end");
    h.type_text("ERROR");
    assert!(h.wait_for(T, |h| h.app.lsp.counts() == (1, 1)), "pulled again after the edit");
    h.key("ctrl+a");
    h.type_text("clean");
    assert!(h.wait_for(T, |h| h.app.lsp.counts() == (0, 0)), "problems go away");
}

/// TypeScript 7 found in the project's node_modules and used with no
/// configuration (`tsc --lsp --stdio`, native binary, pull diagnostics).
/// Runs when `YACE_TEST_TS7` points at a TypeScript ≥ 7 package directory.
#[test]
fn real_typescript_7_builtin() {
    let Ok(pkg) = std::env::var("YACE_TEST_TS7") else { return };
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("app");
    std::fs::create_dir_all(root.join("node_modules")).unwrap();
    std::os::unix::fs::symlink(&pkg, root.join("node_modules/typescript")).unwrap();
    std::fs::write(
        root.join("a.ts"),
        "/** Says hello. */\nexport function greet(name: string): string {\n  return name;\n}\n",
    )
    .unwrap();
    std::fs::write(root.join("b.ts"), "import { greet } from './a';\ngreet(1);\n").unwrap();
    let mut h = Harness::new();
    open_folder(&mut h, &root);
    h.app.open_file(&root.join("b.ts"), false);
    h.settle();
    let t = Duration::from_secs(60);
    assert!(h.wait_for(t, |h| h.app.lsp_status().is_some_and(|s| s == "typescript")), "{:?}", h.app.lsp_status());
    let program = h.app.lsp.servers[0].program.clone();
    eprintln!("program: {}", program.display());
    assert!(h.wait_for(t, |h| h.app.lsp.counts().0 >= 1), "type error pulled");
    let d = h.app.active().unwrap().2;
    assert!(h.app.doc_diagnostics(d).iter().any(|x| x.message.contains("not assignable")));
    h.key("down");
    h.key("home");
    h.key("f12");
    assert!(h.wait_for(t, |h| {
        h.app.active().and_then(|(_, _, d)| h.app.docs.doc(d)?.path().map(|p| p.ends_with("a.ts"))).unwrap_or(false)
    }));
    // Hover: the signature and the docs on separate rows (TypeScript 7's
    // plain text glued them together).
    h.key("ctrl+k ctrl+i");
    assert!(h.wait_for(t, |h| h.app.hover.is_some()), "hover");
    let lines = h.app.hover.as_ref().unwrap().lines.clone();
    eprintln!("hover: {lines:?}");
    assert!(lines[0].contains("greet(name: string): string") && !lines[0].contains("Says"), "{lines:?}");
    assert!(lines.iter().any(|l| l == "Says hello."), "{lines:?}");
    h.key("escape");
    h.app.open_file(&root.join("b.ts"), false);
    h.settle();
    h.key("ctrl+end");
    h.type_text("gre");
    h.command("editor.action.triggerSuggest");
    assert!(h.wait_for(t, |h| {
        h.app.completion.as_ref().is_some_and(|c| c.shown.iter().any(|i| c.items[*i].label == "greet"))
    }));
    h.command("yace.lsp.showRunning");
    if let Some(Overlay::Info { lines, .. }) = h.app.overlays.last() {
        eprintln!("{}", lines.join("\n"));
    }
}

/// Opens a real multi-root workspace (`YACE_TEST_REAL_WS`), one TypeScript
/// file per folder, and reports which server each folder got. Read-only:
/// nothing is edited or saved.
#[test]
fn real_workspace_servers_per_folder() {
    let Ok(ws) = std::env::var("YACE_TEST_REAL_WS") else { return };
    let mut h = Harness::with_config("[lsp]\nidleShutdownSecs = 0\n");
    initial_workspace(
        &mut h.app,
        &Launch { target: Target::WorkspaceFile(ws.into()), add_roots: vec![], goto: None, restore: false },
    );
    h.settle();
    let roots: Vec<std::path::PathBuf> = h.app.workspace.roots().iter().map(|r| r.path.clone()).collect();
    for root in &roots {
        let file = walk_first_ts(root);
        let Some(file) = file else {
            eprintln!("{}: no .ts file", root.display());
            continue;
        };
        h.app.open_file(&file, false);
        h.settle();
    }
    h.wait_for(Duration::from_secs(60), |h| {
        h.app.lsp.servers.iter().all(|s| s.status != yace_app::lsp::Status::Starting)
    });
    std::thread::sleep(Duration::from_secs(3));
    h.settle();
    h.command("yace.lsp.showRunning");
    if let Some(Overlay::Info { lines, .. }) = h.app.overlays.last() {
        eprintln!("{}", lines.join("\n"));
    }
    for (_, m) in &h.app.notification_log {
        eprintln!("note: {m}");
    }
}

fn walk_first_ts(dir: &Path) -> Option<std::path::PathBuf> {
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        let mut entries: Vec<_> = std::fs::read_dir(&d).ok()?.flatten().map(|e| e.path()).collect();
        entries.sort();
        for p in &entries {
            if p.is_file() && p.extension().is_some_and(|e| e == "ts") && !p.to_string_lossy().ends_with(".d.ts") {
                return Some(p.clone());
            }
        }
        for p in entries.into_iter().rev() {
            let name = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            if p.is_dir() && !name.starts_with('.') && name != "node_modules" && name != "dist" && name != "build" {
                stack.push(p);
            }
        }
    }
    None
}

/// Memory per step on a real workspace (`YACE_TEST_REAL_WS`): which
/// feature makes YACE's own RSS grow. Read-only.
#[test]
fn real_workspace_memory_steps() {
    let Ok(ws) = std::env::var("YACE_TEST_REAL_WS") else { return };
    let rss = || -> f64 {
        std::fs::read_to_string("/proc/self/status")
            .unwrap()
            .lines()
            .find(|l| l.starts_with("VmRSS"))
            .and_then(|l| l.split_whitespace().nth(1)?.parse::<f64>().ok())
            .unwrap_or(0.0)
            / 1024.0
    };
    let report = |step: &str| eprintln!("MEM {step:<40} {:6.1} MiB", rss());
    report("start");
    let mut h = Harness::with_config("[lsp]\nidleShutdownSecs = 0\n");
    initial_workspace(
        &mut h.app,
        &Launch { target: Target::WorkspaceFile(ws.into()), add_roots: vec![], goto: None, restore: false },
    );
    h.settle();
    report("workspace open");
    let root = h.app.workspace.roots()[0].path.clone();
    let file = walk_first_ts(&root).unwrap();
    h.app.open_file(&file, false);
    h.settle();
    report("first .ts open (parse)");
    let t = Duration::from_secs(90);
    h.wait_for(t, |h| h.app.lsp_status().is_some_and(|s| s == "typescript"));
    std::thread::sleep(Duration::from_secs(5));
    h.settle();
    report("server running + diagnostics");
    h.key("ctrl+end");
    h.type_text("\nconst zz = ");
    h.command("editor.action.triggerSuggest");
    h.wait_for(t, |h| h.app.completion.is_some());
    let n = h.app.completion.as_ref().map(|c| c.items.len()).unwrap_or(0);
    report(&format!("completion open ({n} items)"));
    h.key("escape");
    h.key("ctrl+z");
    report("completion closed");
    yace_app::mem::trim();
    report("after malloc_trim");
}

/// The details panel beside the completion list: the selected item's
/// documentation, asked for with `completionItem/resolve` when the list
/// didn't include it; hidden when there's no room, toggled from the
/// palette (or Ctrl+Space with the enhanced keyboard protocol).
#[test]
fn completion_details_panel() {
    if !have_python() {
        return;
    }
    let f = fixture(&[("a.ts", "first\n")]);
    let mut h = Harness::with_config(&config(&f.log, "", ""));
    open_folder(&mut h, &f.root);
    h.app.open_file(&f.root.join("a.ts"), false);
    h.settle();
    assert!(wait_running(&mut h));
    h.key("ctrl+end");
    h.type_text("x.gam");
    assert!(h.wait_for(T, |h| h.app.completion.is_some()));
    assert!(h.wait_for(T, |h| h.screen().contains("Gamma docs")), "docs resolved and shown:\n{}", h.screen());
    // Too narrow for a panel beside the list: hidden, list still shown.
    let narrow = h.render_at(40, 20);
    let text: String = (0..20).map(|y| (0..40).map(|x| narrow[(x, y)].symbol()).collect::<String>() + "\n").collect();
    assert!(text.contains("gamma") && !text.contains("Gamma docs"), "{text}");
    // Toggle off and on.
    h.key("ctrl+space");
    assert!(!h.screen().contains("Gamma docs"), "Ctrl+Space hides the details");
    h.command("toggleSuggestionDetails");
    assert!(h.screen().contains("Gamma docs"));
}
