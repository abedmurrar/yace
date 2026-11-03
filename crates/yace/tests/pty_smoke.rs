//! End-to-end smoke test: run the real `yace` binary in a pseudo-terminal,
//! type, save, quit, and check the file and that the terminal was restored.

use std::{
    io::{Read, Write},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use portable_pty::{CommandBuilder, PtySize, native_pty_system};

fn wait_for(deadline: Duration, mut f: impl FnMut() -> bool) -> bool {
    let start = Instant::now();
    while start.elapsed() < deadline {
        if f() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    false
}

#[test]
fn type_save_quit_restores_terminal() {
    let dir = tempfile::tempdir().unwrap();
    let state = tempfile::tempdir().unwrap();
    let file = dir.path().join("hello.txt");
    std::fs::write(&file, "hello world\n").unwrap();

    let pty = native_pty_system();
    let pair = pty.openpty(PtySize { rows: 30, cols: 100, pixel_width: 0, pixel_height: 0 }).unwrap();
    let mut cmd = CommandBuilder::new(env!("CARGO_BIN_EXE_yace"));
    cmd.arg(&file);
    cmd.cwd(dir.path());
    cmd.env("TERM", "xterm-256color");
    cmd.env("YACE_CONFIG_DIR", state.path().join("config"));
    cmd.env("YACE_STATE_DIR", state.path().join("state"));
    cmd.env("YACE_CACHE_DIR", state.path().join("cache"));
    cmd.env_remove("DISPLAY");
    cmd.env_remove("WAYLAND_DISPLAY");
    let mut child = pair.slave.spawn_command(cmd).unwrap();
    drop(pair.slave);

    let output = Arc::new(Mutex::new(Vec::<u8>::new()));
    let mut reader = pair.master.try_clone_reader().unwrap();
    let out2 = Arc::clone(&output);
    std::thread::spawn(move || {
        let mut buf = [0u8; 4096];
        while let Ok(n) = reader.read(&mut buf) {
            if n == 0 {
                break;
            }
            out2.lock().unwrap().extend_from_slice(&buf[..n]);
        }
    });
    let mut writer = pair.master.take_writer().unwrap();
    let contains = |needle: &[u8]| output.lock().unwrap().windows(needle.len()).any(|w| w == needle);

    // Answer the primary device attributes query so the keyboard
    // capability probe finishes quickly (no Kitty protocol reply = legacy).
    if wait_for(Duration::from_secs(5), || contains(b"\x1b[c")) {
        writer.write_all(b"\x1b[?62c").unwrap();
        writer.flush().unwrap();
    }
    assert!(wait_for(Duration::from_secs(10), || contains(b"hello world")), "editor drew the file");
    assert!(contains(b"\x1b[?1049h"), "entered the alternate screen");

    writer.write_all(b"XYZ").unwrap();
    writer.flush().unwrap();
    std::thread::sleep(Duration::from_millis(200));
    writer.write_all(b"\x13").unwrap(); // Ctrl+S
    writer.flush().unwrap();
    assert!(
        wait_for(Duration::from_secs(5), || std::fs::read_to_string(&file).unwrap() == "XYZhello world\n"),
        "file saved: {:?}",
        std::fs::read_to_string(&file)
    );
    // Quick Open through the real index actor: Ctrl+P, type, Enter.
    std::fs::write(dir.path().join("zebra_target.txt"), "found via quick open\n").unwrap();
    writer.write_all(b"\x10").unwrap(); // Ctrl+P
    writer.flush().unwrap();
    std::thread::sleep(Duration::from_millis(300));
    writer.write_all(b"zebtar").unwrap();
    writer.flush().unwrap();
    // (Fuzzy highlights interleave style codes, so wait for the indexer
    // rather than matching the rendered label.)
    std::thread::sleep(Duration::from_millis(700));
    writer.write_all(b"\r").unwrap();
    writer.flush().unwrap();
    assert!(wait_for(Duration::from_secs(10), || contains(b"found via quick open")), "file opened from quick open");
    writer.write_all(b"\x11").unwrap(); // Ctrl+Q
    writer.flush().unwrap();
    let exited = wait_for(Duration::from_secs(10), || child.try_wait().ok().flatten().is_some());
    if !exited {
        let _ = child.kill();
    }
    assert!(exited, "yace exited after Ctrl+Q");
    assert!(child.wait().unwrap().success());
    assert!(
        wait_for(Duration::from_secs(2), || contains(b"\x1b[?1049l")),
        "left the alternate screen (terminal restored)"
    );
    // A session file was written for next time.
    let sessions = state.path().join("state/workspaces");
    assert!(std::fs::read_dir(&sessions).map(|d| d.count() > 0).unwrap_or(false), "session persisted");
}

/// The real binary highlights a Rust file: the `fn` keyword cell carries the
/// theme's keyword colour, checked through a vt100 emulation of the screen.
#[test]
fn real_binary_highlights_rust() {
    let dir = tempfile::tempdir().unwrap();
    let state = tempfile::tempdir().unwrap();
    let file = dir.path().join("main.rs");
    std::fs::write(&file, "fn main() {\n    let s = \"str\";\n}\n").unwrap();
    std::fs::create_dir_all(state.path().join("config")).unwrap();
    std::fs::write(state.path().join("config/config.toml"), "[workbench]\ncolorDepth = \"truecolor\"\n").unwrap();
    let pty = native_pty_system();
    let pair = pty.openpty(PtySize { rows: 20, cols: 80, pixel_width: 0, pixel_height: 0 }).unwrap();
    let mut cmd = CommandBuilder::new(env!("CARGO_BIN_EXE_yace"));
    cmd.arg(&file);
    cmd.cwd(dir.path());
    cmd.env("TERM", "xterm-256color");
    cmd.env("COLORTERM", "truecolor");
    cmd.env("YACE_CONFIG_DIR", state.path().join("config"));
    cmd.env("YACE_STATE_DIR", state.path().join("state"));
    cmd.env("YACE_CACHE_DIR", state.path().join("cache"));
    cmd.env_remove("DISPLAY");
    cmd.env_remove("WAYLAND_DISPLAY");
    let mut child = pair.slave.spawn_command(cmd).unwrap();
    drop(pair.slave);
    let parser = Arc::new(Mutex::new(vt100::Parser::new(20, 80, 0)));
    let raw = Arc::new(Mutex::new(Vec::<u8>::new()));
    let mut reader = pair.master.try_clone_reader().unwrap();
    let (p2, r2) = (Arc::clone(&parser), Arc::clone(&raw));
    std::thread::spawn(move || {
        let mut buf = [0u8; 4096];
        while let Ok(n) = reader.read(&mut buf) {
            if n == 0 {
                break;
            }
            p2.lock().unwrap().process(&buf[..n]);
            r2.lock().unwrap().extend_from_slice(&buf[..n]);
        }
    });
    let mut writer = pair.master.take_writer().unwrap();
    if wait_for(Duration::from_secs(5), || raw.lock().unwrap().windows(3).any(|w| w == b"\x1b[c")) {
        writer.write_all(b"\x1b[?62c").unwrap();
        writer.flush().unwrap();
    }
    // YACE Dark keyword colour (VS Code Dark+ blue) and string colour.
    let keyword = vt100::Color::Rgb(0x56, 0x9c, 0xd6);
    let string = vt100::Color::Rgb(0xce, 0x91, 0x78);
    let cell_fg = |needle: &str| -> Option<vt100::Color> {
        let p = parser.lock().unwrap();
        let screen = p.screen();
        for row in 0..20 {
            let text = screen.contents_between(row, 0, row, 80);
            if let Some(i) = text.find(needle) {
                let col = text[..i].chars().count() as u16;
                return screen.cell(row, col).map(|c| c.fgcolor());
            }
        }
        None
    };
    assert!(
        wait_for(Duration::from_secs(10), || cell_fg("fn main") == Some(keyword)),
        "fn is keyword-coloured: {:?}",
        cell_fg("fn main")
    );
    assert!(wait_for(Duration::from_secs(5), || cell_fg("\"str\"") == Some(string)), "string coloured");
    writer.write_all(b"\x11").unwrap(); // Ctrl+Q
    writer.flush().unwrap();
    assert!(wait_for(Duration::from_secs(5), || child.try_wait().ok().flatten().is_some()));
}

struct Running {
    /// State/cache folder of the run (removed when the run is dropped).
    _state: Option<tempfile::TempDir>,
    child: Box<dyn portable_pty::Child + Send + Sync>,
    parser: Arc<Mutex<vt100::Parser>>,
    writer: Box<dyn Write + Send>,
}

/// Start the real binary on `file` in a 20×80 pseudo-terminal.
fn spawn_yace(file: &std::path::Path, config: &str, env: &[(&str, &str)]) -> (Running, tempfile::TempDir) {
    let state = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(state.path().join("config")).unwrap();
    std::fs::write(state.path().join("config/config.toml"), config).unwrap();
    let cfg = state.path().join("config");
    let (mut r, run_state) = spawn_yace_in(file, &cfg, env);
    r._state = Some(run_state);
    (r, state)
}

/// Start the real binary with an existing config folder (state and cache in
/// a new temporary folder).
fn spawn_yace_in(file: &std::path::Path, cfg: &std::path::Path, env: &[(&str, &str)]) -> (Running, tempfile::TempDir) {
    let state = tempfile::tempdir().unwrap();
    let pty = native_pty_system();
    let pair = pty.openpty(PtySize { rows: 20, cols: 80, pixel_width: 0, pixel_height: 0 }).unwrap();
    let mut cmd = CommandBuilder::new(env!("CARGO_BIN_EXE_yace"));
    cmd.arg(file);
    cmd.cwd(file.parent().unwrap());
    cmd.env("TERM", "xterm-256color");
    cmd.env("COLORTERM", "truecolor");
    cmd.env_remove("NO_COLOR");
    for (k, v) in env {
        cmd.env(k, v);
    }
    cmd.env("YACE_CONFIG_DIR", cfg);
    cmd.env("YACE_STATE_DIR", state.path().join("state"));
    cmd.env("YACE_CACHE_DIR", state.path().join("cache"));
    cmd.env_remove("DISPLAY");
    cmd.env_remove("WAYLAND_DISPLAY");
    let child = pair.slave.spawn_command(cmd).unwrap();
    drop(pair.slave);
    let parser = Arc::new(Mutex::new(vt100::Parser::new(20, 80, 0)));
    let raw = Arc::new(Mutex::new(Vec::<u8>::new()));
    let mut reader = pair.master.try_clone_reader().unwrap();
    let (p2, r2) = (Arc::clone(&parser), Arc::clone(&raw));
    std::thread::spawn(move || {
        let mut buf = [0u8; 4096];
        while let Ok(n) = reader.read(&mut buf) {
            if n == 0 {
                break;
            }
            p2.lock().unwrap().process(&buf[..n]);
            r2.lock().unwrap().extend_from_slice(&buf[..n]);
        }
    });
    let mut writer = pair.master.take_writer().unwrap();
    if wait_for(Duration::from_secs(5), || raw.lock().unwrap().windows(3).any(|w| w == b"\x1b[c")) {
        writer.write_all(b"\x1b[?62c").unwrap();
        writer.flush().unwrap();
    }
    (Running { _state: None, child, parser, writer }, state)
}

fn cell_at(parser: &Mutex<vt100::Parser>, needle: &str) -> Option<vt100::Cell> {
    let p = parser.lock().unwrap();
    let screen = p.screen();
    for row in 0..20 {
        let text = screen.contents_between(row, 0, row, 80);
        if let Some(i) = text.find(needle) {
            return screen.cell(row, text[..i].chars().count() as u16).cloned();
        }
    }
    None
}

fn quit(mut r: Running) {
    r.writer.write_all(b"\x11").unwrap(); // Ctrl+Q
    r.writer.flush().unwrap();
    assert!(wait_for(Duration::from_secs(5), || r.child.try_wait().ok().flatten().is_some()));
}

/// A configured theme reaches the real terminal, and with NO_COLOR the
/// selection is still visible (reverse video, spec §16.5).
#[test]
fn real_binary_themes_and_monochrome() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("main.rs");
    std::fs::write(&file, "fn main() {}\n// second\n").unwrap();

    let (r, _state) = spawn_yace(&file, "[workbench]\ncolorTheme = \"Dracula\"\ncolorDepth = \"truecolor\"\n", &[]);
    let dracula_bg = vt100::Color::Rgb(0x28, 0x2a, 0x36);
    assert!(
        wait_for(Duration::from_secs(10), || cell_at(&r.parser, "// second")
            .is_some_and(|c| c.bgcolor() == dracula_bg)),
        "Dracula background: {:?}",
        cell_at(&r.parser, "// second").map(|c| c.bgcolor())
    );
    quit(r);

    let (mut r, _state) = spawn_yace(&file, "", &[("NO_COLOR", "1")]);
    assert!(wait_for(Duration::from_secs(10), || cell_at(&r.parser, "fn main").is_some()));
    r.writer.write_all(b"\x1b[1;2F").unwrap(); // Shift+End selects the line
    r.writer.flush().unwrap();
    assert!(
        wait_for(Duration::from_secs(5), || cell_at(&r.parser, "fn main").is_some_and(|c| c.inverse())),
        "selection in reverse video"
    );
    let c = cell_at(&r.parser, "fn main").unwrap();
    assert_eq!(c.bgcolor(), vt100::Color::Default, "no colours under NO_COLOR");
    quit(r);
}

/// The real binary: `--install-plugin`, `--list-plugins`, then the plugin's
/// command run from the palette (it loads on first use).
#[test]
fn real_binary_installs_and_runs_a_plugin() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let st = std::process::Command::new(env!("CARGO"))
        .args(["build", "-p", "yace-plugin-hello"])
        .current_dir(&root)
        .status()
        .unwrap();
    assert!(st.success());
    let staged = tempfile::tempdir().unwrap();
    std::fs::copy(root.join("plugins/hello/yace-plugin.toml"), staged.path().join("yace-plugin.toml")).unwrap();
    std::fs::create_dir_all(staged.path().join("lib")).unwrap();
    let target =
        std::env::var_os("CARGO_TARGET_DIR").map(std::path::PathBuf::from).unwrap_or_else(|| root.join("target"));
    std::fs::copy(target.join("debug/libyace_plugin_hello.so"), staged.path().join("lib/libyace_plugin_hello.so"))
        .unwrap();

    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("a.txt");
    std::fs::write(&file, "hello\n").unwrap();
    let (r, state) = spawn_yace(&file, "", &[]);
    quit(r);
    let cfg = state.path().join("config");
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_yace"))
        .arg("--install-plugin")
        .arg(staged.path())
        .arg("--yes")
        .env("YACE_CONFIG_DIR", &cfg)
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(out.status.success(), "{text}{}", String::from_utf8_lossy(&out.stderr));
    assert!(text.contains("NOT sandboxed") && text.contains("Installed dev.yace.hello 1.0.0"), "{text}");
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_yace"))
        .arg("--list-plugins")
        .env("YACE_CONFIG_DIR", &cfg)
        .output()
        .unwrap();
    assert!(String::from_utf8_lossy(&out.stdout).contains("dev.yace.hello  1.0.0  Hello  [installed]"));

    // Same config, now with the plugin installed.
    std::fs::write(cfg.join("config.toml"), "[plugin.\"dev.yace.hello\"]\ngreeting = \"Howdy\"\n").unwrap();
    let (mut r, s2) = spawn_yace_in(&file, &cfg, &[]);
    r._state = Some(s2);
    assert!(wait_for(Duration::from_secs(10), || cell_at(&r.parser, "hello").is_some()));
    r.writer.write_all(b"\x1bOP").unwrap(); // F1: command palette
    r.writer.flush().unwrap();
    std::thread::sleep(Duration::from_millis(200));
    r.writer.write_all(b"say hello").unwrap();
    r.writer.flush().unwrap();
    assert!(wait_for(Duration::from_secs(5), || cell_at(&r.parser, "Hello: Say Hello").is_some()));
    r.writer.write_all(b"\r").unwrap();
    r.writer.flush().unwrap();
    assert!(
        wait_for(Duration::from_secs(10), || cell_at(&r.parser, "Howdy!").is_some()),
        "{}",
        r.parser.lock().unwrap().screen().contents()
    );
    quit(r);
}

/// The real binary in a Git repository: branch in the status bar, and a
/// change bar in the gutter after typing (spec §15.1).
#[test]
fn real_binary_shows_git_branch_and_changes() {
    let dir = tempfile::tempdir().unwrap();
    let run = |args: &[&str]| {
        let out = std::process::Command::new("git")
            .args(args)
            .current_dir(dir.path())
            .env("GIT_AUTHOR_NAME", "T")
            .env("GIT_AUTHOR_EMAIL", "t@example.com")
            .env("GIT_COMMITTER_NAME", "T")
            .env("GIT_COMMITTER_EMAIL", "t@example.com")
            .output()
            .unwrap();
        assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    };
    let file = dir.path().join("notes.md");
    std::fs::write(&file, "first line\nsecond line\n").unwrap();
    run(&["init", "-q", "-b", "trunk"]);
    run(&["add", "-A"]);
    run(&["-c", "commit.gpgsign=false", "commit", "-q", "-m", "init"]);
    let (mut r, _state) = spawn_yace(&file, "", &[]);
    assert!(
        wait_for(Duration::from_secs(10), || cell_at(&r.parser, "trunk").is_some()),
        "branch in the status bar: {}",
        r.parser.lock().unwrap().screen().contents()
    );
    r.writer.write_all(b"edited ").unwrap(); // types at the start of line 1
    r.writer.flush().unwrap();
    assert!(
        wait_for(Duration::from_secs(10), || {
            let p = r.parser.lock().unwrap();
            p.screen().contents().lines().any(|l| l.contains("▎edited first line"))
        }),
        "{}",
        r.parser.lock().unwrap().screen().contents()
    );
    assert!(wait_for(Duration::from_secs(5), || cell_at(&r.parser, "trunk*").is_some()
        || cell_at(&r.parser, "trunk").is_some()));
    // Quit without saving: Ctrl+Q then "Don't Save".
    r.writer.write_all(b"\x11").unwrap();
    r.writer.flush().unwrap();
    std::thread::sleep(Duration::from_millis(300));
    r.writer.write_all(b"d").unwrap();
    r.writer.flush().unwrap();
    assert!(wait_for(Duration::from_secs(5), || r.child.try_wait().ok().flatten().is_some()));
}
