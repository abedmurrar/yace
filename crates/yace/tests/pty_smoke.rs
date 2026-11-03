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
