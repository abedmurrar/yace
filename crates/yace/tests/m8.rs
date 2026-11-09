//! M8: native plugins (spec §18, tests §28.6) with real plugin libraries
//! built from `plugins/*`.

mod common;

use std::{
    path::{Path, PathBuf},
    sync::OnceLock,
    time::Duration,
};

use common::Harness;
use yace_app::{
    cli::{Launch, Target},
    initial_workspace,
    plugins::PluginState,
};
use yace_plugin_host::store;

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize().unwrap()
}

/// Build the example plugins once per test run.
fn built_lib(name: &str) -> PathBuf {
    static BUILT: OnceLock<PathBuf> = OnceLock::new();
    let dir = BUILT.get_or_init(|| {
        let root = workspace_root();
        let st = std::process::Command::new(env!("CARGO"))
            .args(["build", "-p", "yace-plugin-hello", "-p", "yace-plugin-tasks", "-p", "yace-plugin-test-misbehave"])
            .current_dir(&root)
            .status()
            .unwrap();
        assert!(st.success(), "building the example plugins failed");
        let target = std::env::var_os("CARGO_TARGET_DIR").map(PathBuf::from).unwrap_or_else(|| root.join("target"));
        target.join("debug")
    });
    dir.join(format!("lib{name}.so"))
}

/// A plugin folder ready to install: manifest + library under `lib/`.
fn stage(crate_dir: &str, lib: &str) -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    let src = workspace_root().join("plugins").join(crate_dir);
    std::fs::copy(src.join("yace-plugin.toml"), d.path().join("yace-plugin.toml")).unwrap();
    std::fs::create_dir_all(d.path().join("lib")).unwrap();
    std::fs::copy(built_lib(lib), d.path().join("lib").join(format!("lib{lib}.so"))).unwrap();
    d
}

fn install(h: &mut Harness, staged: &Path) {
    let plugins = h.paths().config_dir.join("plugins");
    let plan = store::plan_install(staged, &plugins, yace_app::app::App::yace_version()).unwrap();
    store::install(&plan, &plugins).unwrap();
    h.app.discover_plugins();
    h.settle();
}

fn state(h: &Harness, id: &str) -> PluginState {
    let i = h.app.plugins.find(id).unwrap_or_else(|| panic!("{id} not found"));
    h.app.plugins.entries[i].state.clone()
}

fn log(h: &Harness, id: &str) -> String {
    let i = h.app.plugins.find(id).unwrap();
    h.app.plugins.entries[i].log.iter().cloned().collect::<Vec<_>>().join("\n")
}

fn notes(h: &Harness) -> String {
    h.app.notifications.iter().map(|n| n.text.clone()).collect::<Vec<_>>().join("\n")
}

fn open_folder(h: &mut Harness, dir: &Path) {
    initial_workspace(
        &mut h.app,
        &Launch { target: Target::Folder(dir.to_path_buf()), add_roots: vec![], goto: None, restore: false },
    );
    h.settle();
}

#[test]
fn install_through_the_ui_then_lazy_activation_commands_settings_events_jobs() {
    let ws = tempfile::tempdir().unwrap();
    std::fs::write(ws.path().join("notes.txt"), "one two three\n").unwrap();
    let mut h = Harness::with_config("[plugin.\"dev.yace.hello\"]\ngreeting = \"Hi\"\n");
    open_folder(&mut h, ws.path());
    let staged = stage("hello", "yace_plugin_hello");

    // Install from a directory: the confirmation says what it is.
    h.command("yace.extensions.installFromDirectory");
    h.type_text(&staged.path().display().to_string());
    h.key("enter");
    let screen = h.screen();
    assert!(screen.contains("Install Plugin") && screen.contains("NOT sandboxed"), "{screen}");
    assert!(screen.contains("Hello 1.0.0 (dev.yace.hello)"), "{screen}");
    h.key("i"); // "Install" accelerator
    h.settle();
    assert_eq!(state(&h, "dev.yace.hello"), PluginState::Inactive, "contributions only, no code yet");
    let i = h.app.plugins.find("dev.yace.hello").unwrap();
    assert!(h.app.plugins.entries[i].loaded.is_none());

    // The command is in the palette before the plugin loads.
    h.key("ctrl+shift+p");
    h.type_text("say hello");
    assert!(h.screen().contains("Hello: Say Hello"));
    h.key("enter");
    assert_eq!(state(&h, "dev.yace.hello"), PluginState::Active, "activated by its command");
    assert!(notes(&h).contains("Hello: Hi! (1 workspace folder)"), "{}", notes(&h));

    // Its keybinding works too.
    h.app.notifications.clear();
    h.key("ctrl+alt+h");
    assert!(notes(&h).contains("Hi!"));

    // Status item, updated by the documentSaved event.
    assert!(h.screen().contains("💾 0"), "{}", h.screen());
    h.app.open_file(&ws.path().join("notes.txt"), false);
    h.settle();
    h.type_text("x");
    h.key("ctrl+s");
    h.settle();
    assert!(h.screen().contains("💾 1"), "{}", h.screen());
    // Clicking it runs its command (hello.greet).
    h.app.notifications.clear();
    let status = yace_app::ui::workbench_layout(&h.app).status;
    let cell = yace_app::ui::status_cells(&h.app, status).into_iter().find(|c| c.command == "hello.greet").unwrap();
    let row: String = (cell.x..cell.x + cell.w).map(|x| h.render()[(x, status.y)].symbol().to_string()).collect();
    assert!(row.contains("💾"), "the cell is where the item is drawn: {row:?}");
    h.click(cell.x + 1, status.y, ratatui::crossterm::event::KeyModifiers::NONE);
    assert!(notes(&h).contains("Hi!"), "{}", notes(&h));

    // Reading and editing the active document.
    h.app.notifications.clear();
    h.command("hello.wordCount");
    assert!(notes(&h).contains("3 words"), "{}", notes(&h));
    h.key("ctrl+home");
    h.command("hello.insertDate");
    let text = h.active_text();
    assert!(text.len() == "2025-01-01xone two three\n".len() && text.as_bytes()[4] == b'-', "{text}");
    h.key("ctrl+z");
    assert_eq!(h.active_text(), "xone two three\n", "a plugin edit is one undo step");

    // A background job posts its result back.
    h.app.notifications.clear();
    h.command("hello.slowCount");
    assert!(h.wait_for(Duration::from_secs(10), |h| notes(h).contains("counted 500000 odd numbers")), "{}", notes(&h));

    // An invalid setting value is reported and the default used.
    std::fs::write(h.paths().user_settings(), "[plugin.\"dev.yace.hello\"]\ngreeting = 3\n").unwrap();
    h.command("yace.reloadConfiguration");
    h.app.notifications.clear();
    h.command("hello.greet");
    assert!(notes(&h).contains("Hello: Hello!"), "{}", notes(&h));
    assert!(h.app.config_errors.iter().any(|e| e.message.contains("expected string")), "{:?}", h.app.config_errors);

    // Disable: deactivated, command gone; enable: back.
    h.command("yace.extensions.disable");
    h.type_text("hello");
    h.key("enter");
    h.settle();
    assert_eq!(state(&h, "dev.yace.hello"), PluginState::Disabled);
    assert!(h.app.commands.get("hello.greet").is_none());
    assert!(!h.screen().contains("💾"));
    h.command("yace.extensions.enable");
    h.type_text("hello");
    h.key("enter");
    h.settle();
    assert_eq!(state(&h, "dev.yace.hello"), PluginState::Inactive);
    assert!(h.app.commands.get("hello.greet").is_some());

    // Performance view lists it.
    h.command("hello.greet");
    h.command("yace.extensions.showPerformance");
    let screen = h.screen();
    assert!(screen.contains("Hello") && screen.contains("active"), "{screen}");
}

#[test]
fn tasks_plugin_view_quick_open_and_terminal() {
    let ws = tempfile::tempdir().unwrap();
    std::fs::write(ws.path().join("package.json"), r#"{"scripts": {"dev": "echo dev", "lint": "echo lint"}}"#).unwrap();
    let mut h = Harness::new();
    let staged = stage("tasks", "yace_plugin_tasks");
    install(&mut h, staged.path());
    assert_eq!(state(&h, "dev.yace.tasks"), PluginState::Inactive, "no package.json open yet");
    // Opening a folder with package.json activates it (workspaceContains).
    open_folder(&mut h, ws.path());
    assert_eq!(state(&h, "dev.yace.tasks"), PluginState::Active);
    assert!(h.wait_for(Duration::from_secs(10), |h| h.screen().contains("2 tasks")), "{}", h.screen());

    // The view, drawn by the editor.
    h.command("tasks.view.focus");
    let screen = h.screen();
    assert!(screen.contains("TASKS") && screen.contains("dev") && screen.contains("echo lint"), "{screen}");
    // Down to "dev", Enter: a terminal named after the task, in that folder.
    h.key("down");
    h.key("enter");
    h.settle();
    let t = h.app.panel.tabs.last().expect("a terminal was opened");
    assert_eq!(t.name, "npm dev");
    assert_eq!(t.startup.as_deref(), Some("npm run dev"));
    assert_eq!(t.cwd.canonicalize().unwrap(), ws.path().canonicalize().unwrap());

    // Quick Open provider `!`.
    h.key("ctrl+p");
    h.type_text("!lin");
    assert!(h.wait_for(Duration::from_secs(5), |h| h.screen().contains("Run Task")), "{}", h.screen());
    let screen = h.screen();
    assert!(screen.contains("lint") && !screen.contains("echo dev"), "{screen}");
    h.key("enter");
    h.settle();
    assert_eq!(h.app.panel.tabs.last().unwrap().startup.as_deref(), Some("npm run lint"));
    // `?` lists the provider.
    h.key("ctrl+p");
    h.type_text("?");
    assert!(h.screen().contains("!  Run Task"));
    h.key("escape");
    h.app.kill_all_terminals();
}

#[test]
#[allow(unsafe_code)] // set_var: only this test's fixture plugin reads it
fn malformed_calls_are_rejected_and_failures_contained() {
    let mut h = Harness::new();
    let staged = stage("test-misbehave", "yace_plugin_test_misbehave");
    install(&mut h, staged.path());
    h.command("misbehave.run");
    assert_eq!(state(&h, "dev.yace.test-misbehave"), PluginState::Active);
    let l = log(&h, "dev.yace.test-misbehave");
    for (name, status) in [
        ("null-string", 3),
        ("invalid-utf8", 3),
        ("huge-length", 3),
        ("null-out", 3),
        ("unknown-setting", 4),
        ("unknown-doc", 4),
        ("no-capability", 6),
        ("bad-json", 3),
        ("undeclared-status", 4),
        ("undeclared-view", 4),
        ("no-terminal-capability", 6),
        ("good-status", 0),
    ] {
        assert!(l.contains(&format!("{name}={status}")), "{name}: expected {status} in\n{l}");
    }
    assert!(l.contains("last-error=no setting `nope`"), "{l}");
    assert!(h.screen().contains(" ok"), "the good status item is shown");

    // A slow callback is reported once.
    h.command("misbehave.slow");
    assert!(notes(&h).contains("blocked the editor for"), "{}", notes(&h));
    // A failing command is reported, the plugin keeps running.
    h.app.notifications.clear();
    h.command("misbehave.fail");
    assert!(notes(&h).contains("misbehave.fail: plugin Misbehaving Test Plugin failed"), "{}", notes(&h));
    assert_eq!(state(&h, "dev.yace.test-misbehave"), PluginState::Active);

    // Broken entry points: each one is refused before any callback runs.
    for (mode, expect) in [
        ("abi", "does not support plugin ABI 1"),
        ("fail", "failed to initialise"),
        ("small-table", "table is too small"),
        ("wrong-abi", "answered with ABI 99"),
    ] {
        // SAFETY: only this test's plugin reads the variable, at init.
        unsafe { std::env::set_var("YACE_TEST_PLUGIN_MODE", mode) };
        let mut h = Harness::new();
        install(&mut h, staged.path());
        h.command("misbehave.run");
        match state(&h, "dev.yace.test-misbehave") {
            PluginState::Failed(why) => assert!(why.contains(expect), "{mode}: {why}"),
            s => panic!("{mode}: {s:?}"),
        }
        assert!(notes(&h).contains("was not loaded"), "{mode}: {}", notes(&h));
    }
    unsafe { std::env::remove_var("YACE_TEST_PLUGIN_MODE") };
}

#[test]
fn changed_untrusted_incompatible_and_invalid_plugins_are_not_loaded() {
    let mut h = Harness::new();
    let plugins = h.paths().config_dir.join("plugins");
    // Changed on disk after install.
    let staged = stage("hello", "yace_plugin_hello");
    install(&mut h, staged.path());
    let lib = plugins.join("dev.yace.hello/lib/libyace_plugin_hello.so");
    let mut bytes = std::fs::read(&lib).unwrap();
    bytes.push(0);
    std::fs::write(&lib, bytes).unwrap();
    h.command("hello.greet");
    match state(&h, "dev.yace.hello") {
        PluginState::Untrusted(why) => assert!(why.contains("changed on disk"), "{why}"),
        s => panic!("{s:?}"),
    }

    // Copied in by hand: listed but not loaded until approved.
    let manual = plugins.join("dev.yace.tasks");
    let t = stage("tasks", "yace_plugin_tasks");
    std::fs::create_dir_all(manual.join("lib")).unwrap();
    for f in ["yace-plugin.toml", "lib/libyace_plugin_tasks.so"] {
        std::fs::copy(t.path().join(f), manual.join(f)).unwrap();
    }
    // Incompatible and invalid ones.
    let old = plugins.join("dev.x.future");
    std::fs::create_dir_all(&old).unwrap();
    std::fs::write(
        old.join("yace-plugin.toml"),
        "id = \"dev.x.future\"\nname = \"Future\"\nversion = \"1.0.0\"\nyace = \">=99\"\n",
    )
    .unwrap();
    let bad = plugins.join("dev.x.bad");
    std::fs::create_dir_all(&bad).unwrap();
    std::fs::write(bad.join("yace-plugin.toml"), "id = \"dev.x.bad\"\nname = \"\"\n").unwrap();
    h.app.discover_plugins();
    h.settle();
    assert!(matches!(state(&h, "dev.yace.tasks"), PluginState::Untrusted(_)));
    assert!(matches!(state(&h, "dev.x.future"), PluginState::Incompatible(w) if w.contains("needs YACE >=99")));
    assert_eq!(state(&h, "dev.x.bad"), PluginState::Invalid);
    // The manager lists all of them.
    h.command("workbench.extensions.action.listInstalled");
    let screen = h.screen();
    assert!(screen.contains("Future") && screen.contains("incompatible") && screen.contains("invalid"), "{screen}");
    // Approve the hand-copied one.
    h.type_text("task runner");
    h.key("enter");
    h.type_text("trust");
    h.key("enter");
    assert!(h.screen().contains("NOT sandboxed"));
    h.key("t");
    h.settle();
    assert_eq!(state(&h, "dev.yace.tasks"), PluginState::Inactive);
}

#[test]
fn safe_mode_loads_nothing() {
    let state_dir = tempfile::tempdir().unwrap();
    let plugins = state_dir.path().join("config/plugins");
    let staged = stage("hello", "yace_plugin_hello");
    let plan = store::plan_install(staged.path(), &plugins, yace_app::app::App::yace_version()).unwrap();
    store::install(&plan, &plugins).unwrap();
    let mut h = Harness::with_state_mode(state_dir, true);
    h.settle();
    assert_eq!(state(&h, "dev.yace.hello"), PluginState::Disabled);
    assert!(h.app.commands.get("hello.greet").is_none());
    h.command("workbench.extensions.action.listInstalled");
    assert!(h.screen().contains("safe mode"));
}

#[test]
fn unclean_exit_with_active_plugins_is_reported() {
    let mut h = Harness::new();
    let running = h.paths().state_dir.join("running");
    std::fs::create_dir_all(&running).unwrap();
    // A pid that can't exist.
    std::fs::write(running.join("4000000000.plugins"), "dev.yace.hello\n").unwrap();
    let app = h.new_app();
    let msgs: Vec<String> = app.notifications.iter().map(|n| n.text.clone()).collect();
    assert!(msgs.iter().any(|m| m.contains("closed unexpectedly") && m.contains("dev.yace.hello")), "{msgs:?}");
    assert!(!running.join("4000000000.plugins").exists());
    // Our own activations are recorded and cleared on a clean exit.
    let staged = stage("hello", "yace_plugin_hello");
    install(&mut h, staged.path());
    h.command("hello.greet");
    let marker = running.join(format!("{}.plugins", std::process::id()));
    assert_eq!(std::fs::read_to_string(&marker).unwrap(), "dev.yace.hello");
    h.app.shutdown_plugins();
    assert!(!marker.exists());
}

#[test]
fn data_only_theme_pack_and_packaging() {
    let src = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(src.path().join("themes")).unwrap();
    std::fs::write(
        src.path().join("yace-plugin.toml"),
        "id = \"dev.example.sunset\"\nname = \"Sunset Themes\"\nversion = \"1.0.0\"\n[[themes]]\npath = \"themes/sunset.toml\"\n",
    )
    .unwrap();
    std::fs::write(
        src.path().join("themes/sunset.toml"),
        "name = \"Sunset\"\nextends = \"yace-dark\"\n[colors]\n\"editor.background\" = \"#2b1b2e\"\n",
    )
    .unwrap();
    let out = tempfile::tempdir().unwrap();
    let pkg = out.path().join("sunset.tar.gz");
    store::package(src.path(), &pkg, yace_app::app::App::yace_version()).unwrap();
    let mut h = Harness::new();
    h.command("yace.extensions.installFromPackage");
    h.type_text(&pkg.display().to_string());
    h.key("enter");
    assert!(h.screen().contains("Data only"), "{}", h.screen());
    h.key("i");
    h.settle();
    assert_eq!(state(&h, "dev.example.sunset"), PluginState::Inactive);
    h.command("workbench.action.selectTheme");
    h.type_text("sunset");
    h.key("enter");
    assert_eq!(&*h.app.theme.name, "Sunset");
    assert_eq!(h.app.theme.bg, ratatui::style::Color::Rgb(0x2b, 0x1b, 0x2e));
    // Uninstall: the theme goes away, the current one stays until changed.
    h.command("yace.extensions.uninstall");
    h.type_text("sunset");
    h.key("enter");
    h.key("u");
    h.settle();
    assert!(h.app.plugins.find("dev.example.sunset").is_none());
    assert!(!h.paths().config_dir.join("plugins/dev.example.sunset").exists());
}

/// The grammar package, built once (only C++, unoptimised, for speed).
fn grammar_package() -> PathBuf {
    static PKG: OnceLock<PathBuf> = OnceLock::new();
    PKG.get_or_init(|| {
        let out = Path::new(env!("CARGO_TARGET_TMPDIR")).join("grammar-pkg");
        let _ = std::fs::remove_dir_all(&out);
        let st = std::process::Command::new("python3")
            .arg(workspace_root().join("scripts/build_grammars.py"))
            .arg(&out)
            .env("YACE_GRAMMARS_ONLY", "cpp,go")
            .env("YACE_GRAMMARS_OPT", "-O0")
            .status()
            .expect("python3");
        assert!(st.success(), "scripts/build_grammars.py failed");
        out.join("yace-grammars")
    })
    .clone()
}

#[test]
fn packaged_grammars_load_on_first_use() {
    let ws = tempfile::tempdir().unwrap();
    std::fs::write(ws.path().join("main.cpp"), "int main() {\n    return 0; // done\n}\n").unwrap();
    let mut h = Harness::new();
    open_folder(&mut h, ws.path());
    h.app.open_file(&ws.path().join("main.cpp"), false);
    h.settle();
    h.app.on_tick(std::time::Instant::now());
    let d = h.app.active().unwrap().2;
    // Not installed: plain text, a hint once, editing rules still apply.
    assert_eq!(h.app.syntax.language(d).id, "cpp");
    assert!(!h.app.syntax.has_tree(d));
    assert!(notes(&h).contains("C++ highlighting comes in the YACE Grammars package"), "{}", notes(&h));
    h.key("ctrl+/");
    assert!(h.active_text().starts_with("// int main"), "{}", h.active_text());
    h.key("ctrl+z");

    // Installed: the open document is highlighted right away.
    let pkg = grammar_package();
    install(&mut h, &pkg);
    h.app.on_tick(std::time::Instant::now());
    h.settle();
    assert!(h.wait_for(Duration::from_secs(10), |h| h.app.syntax.has_tree(d)), "the C++ document got a tree");
    let tree = h.app.syntax.tree(d).unwrap().root_node().to_sexp();
    assert!(tree.starts_with("(translation_unit (function_definition"), "{tree}");
    let kw = h.app.theme.syntax[yace_syntax::Scope::Keyword as usize];
    let buf = h.render();
    let (mut found, area) = (false, buf.area);
    for y in 0..area.height {
        let row: String = (0..area.width).map(|x| buf[(x, y)].symbol().to_string()).collect();
        if let Some(b) = row.find("return 0") {
            let x = row[..b].chars().count() as u16;
            assert_eq!(buf[(x, y)].fg, kw, "`return` is highlighted as a keyword");
            found = true;
        }
    }
    assert!(found);
    assert!(log(&h, "dev.yace.grammars").contains("grammar cpp loaded"), "{}", log(&h, "dev.yace.grammars"));
}

#[test]
fn a_modified_grammar_library_is_refused() {
    // Go: no other test loads it, so the process-wide registry is clean.
    let ws = tempfile::tempdir().unwrap();
    std::fs::write(ws.path().join("main.go"), "package main\n").unwrap();
    let mut h = Harness::new();
    let pkg = grammar_package();
    install(&mut h, &pkg);
    let lib = h.paths().config_dir.join("plugins/dev.yace.grammars/grammars/go.so");
    let mut bytes = std::fs::read(&lib).unwrap();
    bytes.extend_from_slice(b"tampered");
    std::fs::write(&lib, bytes).unwrap();
    open_folder(&mut h, ws.path());
    h.app.open_file(&ws.path().join("main.go"), false);
    h.settle();
    h.app.on_tick(std::time::Instant::now());
    let d = h.app.active().unwrap().2;
    assert!(!yace_syntax::has_external_grammar("go"));
    assert!(!h.app.syntax.has_tree(d));
    assert!(notes(&h).contains("the go grammar could not be loaded: grammars/go.so changed on disk"), "{}", notes(&h));
}
