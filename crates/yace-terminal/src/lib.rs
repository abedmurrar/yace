//! # yace-terminal
//!
//! The integrated terminal's engine (spec §11): shells on pseudo-terminals,
//! emulated by [`alacritty_terminal`] (Apache-2.0, by the Alacritty
//! project), plus xterm-compatible encoding of keys, pastes and mouse
//! events. No UI dependencies: the app renders a [`Screen`] snapshot.

pub mod input;
pub mod session;

pub use input::{MouseButton, MouseKind, encode_key, encode_mouse, encode_paste};
pub use session::{
    CellStyle, Modes, Notice, Screen, ScreenCell, Session, SessionConfig, SessionId, TermColor, default_shell,
};

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use crossbeam_channel::{Receiver, unbounded};

    use super::*;

    fn spawn(
        cmd: Option<&str>,
        rows: u16,
        cols: u16,
        scrollback: usize,
    ) -> (Session, Receiver<Notice>, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let (tx, rx) = unbounded();
        let (shell, args) = match cmd {
            Some(c) => ("/bin/sh".to_string(), vec!["-c".to_string(), c.to_string()]),
            None => ("/bin/sh".to_string(), vec![]),
        };
        let cfg = SessionConfig {
            shell,
            args,
            env: vec![("PS1".into(), "$ ".into()), ("ENV".into(), String::new())],
            cwd: dir.path().to_path_buf(),
            rows,
            cols,
            scrollback,
            startup_command: None,
        };
        (Session::spawn(&cfg, 1, tx).unwrap(), rx, dir)
    }

    /// Poll the screen until `pred` holds (re-arming output notices).
    fn wait_screen(s: &Session, timeout: Duration, pred: impl Fn(&Screen) -> bool) -> Screen {
        let start = Instant::now();
        let mut screen = Screen::default();
        loop {
            s.screen(&mut screen);
            s.mark_seen();
            if pred(&screen) || start.elapsed() > timeout {
                return screen;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    #[test]
    fn output_colours_and_exit_code() {
        let (s, rx, _d) = spawn(Some("printf 'hello \\033[31mred\\033[0m'; exit 3"), 10, 40, 100);
        let screen = wait_screen(&s, Duration::from_secs(5), |sc| sc.text().contains("hello red"));
        assert!(screen.text().contains("hello red"), "{:?}", screen.text());
        let red = screen.cell(0, 6);
        assert_eq!(red.ch, 'r');
        assert_eq!(red.fg, TermColor::Named(1));
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut code = None;
        while Instant::now() < deadline {
            if let Ok(Notice::Exit(_, c)) = rx.recv_timeout(Duration::from_millis(100)) {
                code = Some(c);
                break;
            }
        }
        assert_eq!(code, Some(Some(3)));
        assert_eq!(s.exit_status(), Some(Some(3)));
    }

    #[test]
    fn interactive_shell_resize_interrupt_and_cwd() {
        let (s, _rx, dir) = spawn(None, 12, 60, 100);
        wait_screen(&s, Duration::from_secs(5), |sc| sc.text().contains('$'));
        s.write(b"pwd\r");
        let want = std::fs::canonicalize(dir.path()).unwrap().display().to_string();
        let sc = wait_screen(&s, Duration::from_secs(5), |sc| sc.text().contains(&want));
        assert!(sc.text().contains(&want), "starts in the requested cwd: {}", sc.text());
        s.resize(20, 70);
        s.write(b"stty size\r");
        let sc = wait_screen(&s, Duration::from_secs(5), |sc| sc.text().contains("20 70"));
        assert!(sc.text().contains("20 70"), "{}", sc.text());
        s.write(b"sleep 30\r");
        let start = Instant::now();
        while !s.has_running_job() && start.elapsed() < Duration::from_secs(5) {
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(s.has_running_job(), "sleep is a foreground job");
        s.write(&[3]); // Ctrl+C
        let start = Instant::now();
        while s.has_running_job() && start.elapsed() < Duration::from_secs(5) {
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(!s.has_running_job(), "Ctrl+C interrupted the job");
        assert!(s.is_running(), "the shell survives");
    }

    #[test]
    fn alternate_screen_and_bounded_scrollback() {
        let (s, _rx, _d) = spawn(None, 8, 40, 50);
        wait_screen(&s, Duration::from_secs(5), |sc| sc.text().contains('$'));
        s.write(b"printf 'MAIN\\n'; printf '\\033[?1049hALT'; sleep 0.3; printf '\\033[?1049l'\r");
        let sc = wait_screen(&s, Duration::from_secs(5), |sc| sc.modes.alt_screen);
        assert!(sc.modes.alt_screen);
        assert!(sc.text().contains("ALT"));
        let sc = wait_screen(&s, Duration::from_secs(5), |sc| !sc.modes.alt_screen && sc.text().contains("MAIN"));
        assert!(!sc.text().contains("ALT"), "main screen restored: {}", sc.text());
        s.write(b"i=0; while [ $i -lt 500 ]; do echo line$i; i=$((i+1)); done\r");
        let sc = wait_screen(&s, Duration::from_secs(10), |sc| sc.text().contains("line499"));
        assert!(sc.history <= 50, "history bounded: {}", sc.history);
        s.scroll(10);
        let mut scrolled = Screen::default();
        s.screen(&mut scrolled);
        assert_eq!(scrolled.display_offset, 10);
        s.scroll_to_bottom();
    }

    #[test]
    fn kill_stops_the_process() {
        let (mut s, _rx, _d) = spawn(None, 5, 20, 10);
        let pid = s.pid();
        wait_screen(&s, Duration::from_secs(5), |sc| sc.text().contains('$'));
        s.kill();
        let start = Instant::now();
        while std::path::Path::new(&format!("/proc/{pid}")).exists() && start.elapsed() < Duration::from_secs(5) {
            std::thread::sleep(Duration::from_millis(50));
        }
        // A zombie may linger briefly until reaped; it must not be running.
        let state = std::fs::read_to_string(format!("/proc/{pid}/stat")).unwrap_or_default();
        assert!(state.is_empty() || state.contains(") Z"), "process gone: {state}");
        assert!(!s.is_running());
    }
}
