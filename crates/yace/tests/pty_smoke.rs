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
