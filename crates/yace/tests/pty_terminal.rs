//! End-to-end: the integrated terminal inside the real binary, checked
//! through a vt100 screen model; quitting leaves no shell behind.

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

fn children_of(pid: u32) -> Vec<u32> {
    std::fs::read_to_string(format!("/proc/{pid}/task/{pid}/children"))
        .unwrap_or_default()
        .split_whitespace()
        .filter_map(|p| p.parse().ok())
        .collect()
}

#[test]
fn terminal_runs_commands_and_leaves_no_orphans() {
    let dir = tempfile::tempdir().unwrap();
    let state = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(state.path().join("c")).unwrap();
    std::fs::write(
        state.path().join("c/config.toml"),
        "[terminal]\nshell = \"/bin/sh\"\n[terminal.env]\nPS1 = \"$ \"\nENV = \"\"\n",
    )
    .unwrap();
    let pair = native_pty_system().openpty(PtySize { rows: 30, cols: 100, pixel_width: 0, pixel_height: 0 }).unwrap();
    let mut cmd = CommandBuilder::new(env!("CARGO_BIN_EXE_yace"));
    cmd.arg(dir.path());
    cmd.env("TERM", "xterm-256color");
    cmd.env("YACE_CONFIG_DIR", state.path().join("c"));
    cmd.env("YACE_STATE_DIR", state.path().join("s"));
    cmd.env("YACE_CACHE_DIR", state.path().join("k"));
    cmd.env_remove("DISPLAY");
    cmd.env_remove("WAYLAND_DISPLAY");
    let mut child = pair.slave.spawn_command(cmd).unwrap();
    let yace_pid = child.process_id().unwrap();
    let parser = Arc::new(Mutex::new(vt100::Parser::new(30, 100, 0)));
    let mut reader = pair.master.try_clone_reader().unwrap();
    let p2 = parser.clone();
    std::thread::spawn(move || {
        let mut b = [0u8; 8192];
        while let Ok(n) = reader.read(&mut b) {
            if n == 0 {
                break;
            }
            p2.lock().unwrap().process(&b[..n]);
        }
    });
    let mut w = pair.master.take_writer().unwrap();
    let screen = || parser.lock().unwrap().screen().contents();
    std::thread::sleep(Duration::from_millis(300));
    w.write_all(b"\x1b[?62c").unwrap(); // answer the capability probe (legacy keyboard)
    assert!(wait_for(Duration::from_secs(10), || screen().contains("EXPLORER")), "editor drawn");
    w.write_all(b"\x1b`").unwrap(); // Alt+` (legacy fallback for Ctrl+`)
    w.flush().unwrap();
    assert!(
        wait_for(Duration::from_secs(10), || screen().contains("TERMINAL") && screen().contains("$ ")),
        "panel + prompt:\n{}",
        screen()
    );
    w.write_all(b"echo hi-from-$((40+2))\r").unwrap();
    w.flush().unwrap();
    assert!(
        wait_for(Duration::from_secs(10), || screen().contains("hi-from-42")),
        "command output in the panel:\n{}",
        screen()
    );
    let shells = children_of(yace_pid);
    assert!(!shells.is_empty(), "a shell runs as a child of yace");
    w.write_all(b"\x1bOP").unwrap(); // F1 (skip-shell): command palette
    w.flush().unwrap();
    std::thread::sleep(Duration::from_millis(300));
    w.write_all(b"Quit\r").unwrap();
    w.flush().unwrap();
    let exited = wait_for(Duration::from_secs(10), || child.try_wait().ok().flatten().is_some());
    if !exited {
        let _ = child.kill();
    }
    assert!(exited, "yace quit:\n{}", screen());
    for pid in shells {
        assert!(
            wait_for(Duration::from_secs(5), || {
                let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).unwrap_or_default();
                stat.is_empty() || stat.contains(") Z")
            }),
            "shell {pid} was stopped"
        );
    }
}
