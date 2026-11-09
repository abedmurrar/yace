//! Several language servers per file: a plugin's server (`[[lsp_servers]]`)
//! next to a language's main server. Both are copies of the fake server
//! (`tests/fixtures/fake_lsp.py`): the main one reports TODO and ERROR, the
//! plugin's one ("lint") reports LINT.

mod common;

use std::{
    path::{Path, PathBuf},
    time::Duration,
};

use common::Harness;
use yace_app::{
    cli::{Launch, Target},
    initial_workspace,
    overlay::Overlay,
};
use yace_plugin_host::store;

const T: Duration = Duration::from_secs(10);
const PLUGIN: &str = "dev.test.lint";

fn have_python() -> bool {
    yace_lsp::registry::which("python3").is_some()
}

fn fake() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/fake_lsp.py")
}

struct Fixture {
    _dir: tempfile::TempDir,
    root: PathBuf,
    main_log: PathBuf,
    lint_log: PathBuf,
}

/// A project with `a.ts`, the plugin's program in `node_modules/.bin`, and
/// `lint.json` (the plugin's root file) unless `with_root_file` is false.
fn fixture(text: &str, with_root_file: bool) -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("proj");
    std::fs::create_dir_all(root.join("node_modules/.bin")).unwrap();
    std::os::unix::fs::symlink(fake(), root.join("node_modules/.bin/fake-lint")).unwrap();
    std::fs::write(root.join("a.ts"), text).unwrap();
    std::fs::write(root.join("notes.txt"), "LINT here\n").unwrap();
    if with_root_file {
        std::fs::write(root.join("lint.json"), "{}").unwrap();
    }
    Fixture { main_log: dir.path().join("main.log"), lint_log: dir.path().join("lint.log"), _dir: dir, root }
}

/// User config: the fake server is TypeScript's main server.
fn config(f: &Fixture, extra: &str) -> String {
    format!(
        r#"[lsp]
disabled = ["typescript-language-server", "typescript"]
idleShutdownSecs = 0
[[lsp.servers]]
name = "fake"
command = "{cmd}"
languages = ["typescript"]
env = {{ FAKE_LSP_LOG = "{log}" }}
{extra}"#,
        cmd = fake().display(),
        log = f.main_log.display()
    )
}

/// Install a data-only plugin whose language server is the fake one under
/// the name "lint", for TypeScript and plain text.
fn install_plugin(h: &mut Harness, f: &Fixture) {
    let src = tempfile::tempdir().unwrap();
    std::fs::write(
        src.path().join("yace-plugin.toml"),
        format!(
            r#"id = "{PLUGIN}"
name = "Lint"
version = "1.0.0"

[[lsp_servers]]
name = "lint"
command = "fake-lint"
local_bin = true
languages = ["typescript", "plaintext"]
root_files = ["lint.json"]
env = {{ FAKE_LSP_LOG = "{log}", FAKE_LSP_NAME = "lint", FAKE_LSP_MATCH = "LINT" }}
settings = {{ lint = {{ folder = "${{workspaceFolder}}", name = "${{workspaceFolderName}}" }} }}
install_hint = "npm i -D fake-lint"
"#,
            log = f.lint_log.display()
        ),
    )
    .unwrap();
    let plugins = h.paths().config_dir.join("plugins");
    let plan = store::plan_install(src.path(), &plugins, yace_app::app::App::yace_version()).unwrap();
    let summary = yace_app::plugin_cli::trust_summary(&plan).join("\n");
    assert!(summary.contains("Starts a language server: fake-lint (from the project's node_modules/.bin"), "{summary}");
    assert!(summary.contains("in folders with lint.json") && !summary.contains("no code runs"), "{summary}");
    store::install(&plan, &plugins).unwrap();
    h.app.discover_plugins();
    h.settle();
}

fn open_folder(h: &mut Harness, dir: &Path) {
    initial_workspace(
        &mut h.app,
        &Launch { target: Target::Folder(dir.to_path_buf()), add_roots: vec![], goto: None, restore: false },
    );
    h.settle();
}

fn log(p: &Path) -> String {
    std::fs::read_to_string(p).unwrap_or_default()
}

/// (source, message) of the active document's diagnostics.
fn diags(h: &Harness) -> Vec<(String, String)> {
    let d = h.app.active().unwrap().2;
    h.app.doc_diagnostics(d).iter().map(|x| (x.source.clone().unwrap_or_default(), x.message.clone())).collect()
}

fn status(h: &Harness) -> String {
    h.app.lsp_status().unwrap_or_default()
}

#[test]
fn a_plugin_server_runs_next_to_the_main_one() {
    if !have_python() {
        return;
    }
    let f = fixture("TODO LINT\n", true);
    let mut h = Harness::with_config(&config(&f, "[editor]\ndefaultFormatter = \"lint\"\n"));
    install_plugin(&mut h, &f);
    open_folder(&mut h, &f.root);
    h.app.open_file(&f.root.join("a.ts"), false);
    h.settle();
    assert!(h.wait_for(T, |h| status(h) == "fake + lint"), "{:?}", h.app.lsp_status());

    // Both report; Problems lists both sources.
    assert!(h.wait_for(T, |h| diags(h).len() == 2), "{:?}", diags(&h));
    let mut got = diags(&h);
    got.sort();
    assert_eq!(got, [("fake".into(), "TODO found".into()), ("lint".into(), "LINT found".into())]);
    h.key("ctrl+shift+m");
    let screen = h.screen();
    assert!(screen.contains("TODO found") && screen.contains("LINT found"), "{screen}");
    h.key("escape");

    // ${workspaceFolder} was filled in the configuration it was given.
    let root = f.root.display().to_string();
    assert!(h.wait_for(T, |_| log(&f.lint_log).contains(&format!("\"folder\": \"{root}\""))), "{}", log(&f.lint_log));
    assert!(log(&f.lint_log).contains("\"name\": \"proj\""));

    // Edits reach both; a server's new report replaces only its own.
    h.key("ctrl+end");
    h.type_text("ERROR\n");
    assert!(h.wait_for(T, |h| diags(h).len() == 3), "{:?}", diags(&h));
    assert!(log(&f.main_log).contains("didChange") && log(&f.lint_log).contains("didChange"), "didChange to both");
    h.key("ctrl+z");
    assert!(h.wait_for(T, |h| diags(h).len() == 2), "{:?}", diags(&h));

    // Hover asks only the main server.
    h.key("ctrl+home");
    h.key("ctrl+k ctrl+i");
    assert!(h.wait_for(T, |h| h.app.hover.is_some()));
    h.key("escape");
    assert!(log(&f.main_log).contains("textDocument/hover") && !log(&f.lint_log).contains("textDocument/hover"));

    // Code actions from both servers in one picker, each with its own
    // diagnostics (and their `data`) as context; the chosen one runs on
    // its server.
    h.key("ctrl+.");
    assert!(
        h.wait_for(T, |h| matches!(h.app.overlays.last(),
        Some(Overlay::Picker(p)) if p.items.len() == 2)),
        "{}",
        h.screen()
    );
    let Some(Overlay::Picker(p)) = h.app.overlays.last() else { unreachable!() };
    let labels: Vec<(String, String)> = p.items.iter().map(|i| (i.label.clone(), i.hint.clone())).collect();
    assert!(labels.contains(&("Replace TODO with DONE".into(), "quickfix · fake".into())), "{labels:?}");
    assert!(labels.contains(&("lint: replace LINT".into(), "quickfix · lint".into())), "{labels:?}");
    h.type_text("lint:");
    h.key("enter");
    assert!(h.wait_for(T, |h| h.active_text().starts_with("TODO DONE")), "{}", h.active_text());

    // editor.defaultFormatter = "lint": the plugin's server formats.
    h.command("editor.action.formatDocument");
    assert!(h.wait_for(T, |h| h.active_text().starts_with("// lint\n")), "{}", h.active_text());

    // Disabling the plugin stops its server and removes its diagnostics.
    h.app.execute("yace.extensions.disable", None);
    h.type_text("Lint");
    h.key("enter");
    h.settle();
    assert!(h.wait_for(T, |h| status(h) == "fake"), "{:?}", h.app.lsp_status());
    assert!(!h.app.lsp.servers.iter().any(|s| s.def.name == "lint"));
    assert!(diags(&h).iter().all(|(src, _)| src == "fake"), "{:?}", diags(&h));
    // Enabling it again starts it for the open file.
    h.app.execute("yace.extensions.enable", None);
    h.type_text("Lint");
    h.key("enter");
    h.settle();
    assert!(h.wait_for(T, |h| status(h) == "fake + lint"), "{:?}", h.app.lsp_status());
}

#[test]
fn a_plugin_server_needs_its_root_file_and_can_be_the_only_one() {
    if !have_python() {
        return;
    }
    // No lint.json: only the main server.
    let f = fixture("TODO LINT\n", false);
    let mut h = Harness::with_config(&config(&f, ""));
    install_plugin(&mut h, &f);
    open_folder(&mut h, &f.root);
    h.app.open_file(&f.root.join("a.ts"), false);
    h.settle();
    assert!(h.wait_for(T, |h| status(h) == "fake"));
    std::thread::sleep(Duration::from_millis(300));
    h.settle();
    assert_eq!(status(&h), "fake");
    assert!(!f.lint_log.exists(), "never started");

    // Plain text has no main server: the plugin's is the only one.
    let f = fixture("TODO LINT\n", true);
    let mut h = Harness::with_config(&config(&f, ""));
    install_plugin(&mut h, &f);
    open_folder(&mut h, &f.root);
    h.app.open_file(&f.root.join("notes.txt"), false);
    h.settle();
    assert!(h.wait_for(T, |h| status(h) == "lint"), "{:?}", h.app.lsp_status());
    assert!(h.wait_for(T, |h| diags(h) == [("lint".to_string(), "LINT found".to_string())]), "{:?}", diags(&h));
    // Uninstalling removes it and what it reported.
    h.app.execute("yace.extensions.uninstall", None);
    h.type_text("Lint");
    h.key("enter");
    h.key("u"); // "Uninstall" accelerator
    h.settle();
    assert!(h.wait_for(T, |h| h.app.lsp_status().is_none()), "{:?}", h.app.lsp_status());
    assert!(h.app.lsp.diagnostics.is_empty());
}

#[test]
fn safe_mode_starts_no_plugin_server() {
    if !have_python() {
        return;
    }
    let f = fixture("TODO LINT\n", true);
    let mut h = Harness::with_config(&config(&f, ""));
    install_plugin(&mut h, &f);
    let state = std::mem::replace(&mut h.state, tempfile::tempdir().unwrap());
    let mut h = Harness::with_state_mode(state, true);
    h.app.discover_plugins();
    h.settle();
    open_folder(&mut h, &f.root);
    h.app.open_file(&f.root.join("a.ts"), false);
    h.settle();
    assert!(h.wait_for(T, |h| status(h) == "fake"));
    std::thread::sleep(Duration::from_millis(300));
    h.settle();
    assert_eq!(status(&h), "fake");
    assert!(!f.lint_log.exists());
}

/// Install a real plugin folder (its `yace` requirement relaxed, so it
/// installs into this build whatever its version).
fn install_real(h: &mut Harness, plugin: &Path) {
    let staged = tempfile::tempdir().unwrap();
    let text = std::fs::read_to_string(plugin.join("yace-plugin.toml")).unwrap();
    let text: String = text
        .lines()
        .map(|l| if l.starts_with("yace = ") { "yace = \"*\"\n".to_string() } else { format!("{l}\n") })
        .collect();
    std::fs::write(staged.path().join("yace-plugin.toml"), text).unwrap();
    let plugins = h.paths().config_dir.join("plugins");
    let plan = store::plan_install(staged.path(), &plugins, yace_app::app::App::yace_version()).unwrap();
    store::install(&plan, &plugins).unwrap();
    h.app.discover_plugins();
    h.settle();
}

/// `VAR=<plugin folder>:<project folder>`; the project is copied so the
/// test can edit it.
fn real_setup(var: &str, config: &str, file: &str) -> Option<(Harness, tempfile::TempDir)> {
    let v = std::env::var(var).ok()?;
    let (plugin, project) = v.split_once(':')?;
    let copy = tempfile::tempdir().unwrap();
    let st = std::process::Command::new("cp").arg("-a").arg(format!("{project}/.")).arg(copy.path()).status().unwrap();
    assert!(st.success());
    let mut h = Harness::with_config(config);
    install_real(&mut h, Path::new(plugin));
    open_folder(&mut h, copy.path());
    h.app.open_file(&copy.path().join(file), false);
    h.settle();
    Some((h, copy))
}

fn picker_items(h: &Harness) -> Vec<(String, String)> {
    match h.app.overlays.last() {
        Some(Overlay::Picker(p)) => p.items.iter().map(|i| (i.label.clone(), i.hint.clone())).collect(),
        _ => Vec::new(),
    }
}

/// Real Biome (`YACE_TEST_BIOME=<yace-biome folder>:<project with
/// biome.json and node_modules/.bin/biome>`): problems, a quick fix and
/// formatting through `editor.defaultFormatter`.
#[test]
fn real_biome() {
    let cfg = "[lsp]\nidleShutdownSecs = 0\n[editor]\ndefaultFormatter = \"biome\"\n";
    let Some((mut h, _copy)) = real_setup("YACE_TEST_BIOME", cfg, "a.ts") else { return };
    let t = Duration::from_secs(60);
    assert!(h.wait_for(t, |h| status(h).contains("biome")), "{:?}", h.app.lsp_status());
    eprintln!("servers: {}", status(&h));
    assert!(h.wait_for(t, |h| diags(h).iter().any(|(s, _)| s == "biome")), "{:?}", diags(&h));
    eprintln!("diagnostics: {:?}", diags(&h));
    // Quick fix for `==` on line 2.
    h.key("ctrl+home");
    h.key("down");
    h.key("end");
    h.key("ctrl+.");
    assert!(
        h.wait_for(t, |h| !picker_items(h).is_empty()),
        "{}\n{}",
        h.screen(),
        h.app.notifications.iter().map(|n| n.text.clone()).collect::<Vec<_>>().join("\n")
    );
    let items = picker_items(&h);
    eprintln!("code actions: {items:?}");
    let k = items.iter().position(|(l, _)| l.contains("fix for noDoubleEquals")).expect("the fix for ==");
    for _ in 0..k {
        h.key("down");
    }
    h.key("enter");
    assert!(h.wait_for(t, |h| h.active_text().contains("x === 1")), "{}", h.active_text());
    // Biome formats (its server, not the main one).
    h.command("editor.action.formatDocument");
    assert!(h.wait_for(t, |h| h.active_text().contains("export const y = 2")), "{}", h.active_text());
}

/// Real ESLint (`YACE_TEST_ESLINT=<yace-eslint folder>:<project with
/// eslint.config.* and ESLint installed>`; vscode-eslint-language-server on
/// PATH or in the project): problems and a quick fix.
#[test]
fn real_eslint() {
    let cfg = "[lsp]\nidleShutdownSecs = 0\n";
    let Some((mut h, _copy)) = real_setup("YACE_TEST_ESLINT", cfg, "a.js") else { return };
    let t = Duration::from_secs(60);
    assert!(h.wait_for(t, |h| status(h).contains("eslint")), "{:?}", h.app.lsp_status());
    eprintln!("servers: {}", status(&h));
    assert!(
        h.wait_for(t, |h| diags(h).iter().filter(|(s, _)| s == "eslint").count() >= 2),
        "{:?}\n{}",
        diags(&h),
        h.app.notifications.iter().map(|n| n.text.clone()).collect::<Vec<_>>().join("\n")
    );
    eprintln!("diagnostics: {:?}", diags(&h));
    // Quick fix on the debugger line: disable the rule for that line.
    h.key("ctrl+home");
    h.key("down");
    h.key("ctrl+.");
    assert!(h.wait_for(t, |h| !picker_items(h).is_empty()), "{}", h.screen());
    let items = picker_items(&h);
    eprintln!("code actions: {items:?}");
    let k = items.iter().position(|(l, _)| l.contains("Disable no-debugger for this line")).expect("disable action");
    for _ in 0..k {
        h.key("down");
    }
    h.key("enter");
    assert!(
        h.wait_for(t, |h| h.active_text().contains("// eslint-disable-next-line no-debugger")),
        "{}",
        h.active_text()
    );
    // The problem goes away once ESLint re-checks.
    assert!(h.wait_for(t, |h| !diags(h).iter().any(|(_, m)| m.contains("debugger"))), "{:?}", diags(&h));
}
