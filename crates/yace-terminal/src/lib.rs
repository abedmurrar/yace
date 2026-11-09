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
    CellStyle, Modes, Notice, Screen, ScreenCell, Searcher, Session, SessionConfig, SessionId, TermColor, TermMatch,
    TermQuery, default_shell,
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

    #[test]
    fn search_through_scrollback() {
        let (s, _rx, _d) = spawn(None, 10, 40, 5000);
        wait_screen(&s, Duration::from_secs(5), |sc| sc.text().contains('$'));
        s.write(b"i=0; while [ $i -lt 3000 ]; do echo row$i; i=$((i+1)); done; echo Row-END\r");
        wait_screen(&s, Duration::from_secs(20), |sc| sc.text().contains("Row-END"));
        let f = s.searcher();
        let go = || false;
        let mut q = TermQuery::new("row1", false, false, false).unwrap();
        // From the bottom upwards: row1999 is the newest "row1…" line.
        let m = f.find(&mut q, None, true, &go).unwrap();
        s.show_match(m);
        let mut screen = Screen::default();
        s.screen(&mut screen);
        assert!(screen.display_offset > 0, "scrolled to the match");
        assert_eq!(s.selection_text().as_deref(), Some("row1"));
        assert!(screen.text().contains("row1999"));
        let older = f.find(&mut q, Some(m), true, &go).unwrap();
        assert!(older.start < m.start, "up goes to older output");
        assert_eq!(f.find(&mut q, Some(older), false, &go), Some(m), "down comes back");
        // Walk all the way up: crosses chunk boundaries, then wraps to m.
        let mut cur = m;
        let mut steps = 0;
        loop {
            cur = f.find(&mut q, Some(cur), true, &go).unwrap();
            steps += 1;
            if cur == m {
                break;
            }
            assert!(steps < 2000);
        }
        assert_eq!(steps, 1111, "row1, row10–19, row100–199, row1000–1999");
        let (n, at) = f.count(&mut q, Some(m), 5000, &go).unwrap();
        assert_eq!(n, 1111);
        assert_eq!(at, Some(1111));
        assert_eq!(f.count(&mut q, None, 50, &go).unwrap().0, 50, "capped");
        assert!(f.find(&mut q, None, true, &|| true).is_none(), "stopped");
        // Case, whole word, regex.
        let mut cs = TermQuery::new("Row", true, false, false).unwrap();
        assert_eq!(f.count(&mut cs, None, 5000, &go).unwrap().0, 2, "the command line and its output");
        let mut word = TermQuery::new("row2", false, true, false).unwrap();
        assert_eq!(f.count(&mut word, None, 5000, &go).unwrap().0, 1, "whole word: row2 only");
        let mut re = TermQuery::new(r"row\d\d\d\d", false, false, true).unwrap();
        assert_eq!(f.count(&mut re, None, 5000, &go).unwrap().0, 2000);
        assert!(TermQuery::new("(", false, false, true).is_err());
        let mut none = TermQuery::new("nothing-like-this", false, false, false).unwrap();
        assert_eq!(f.find(&mut none, None, true, &go), None);
        // Highlights on screen.
        s.screen_with_matches(&mut screen, &mut q, Some(m));
        assert!(screen.cells.iter().any(|c| c.style.find_current));
    }

    /// Timings: `cargo test --release -p yace-terminal -- --ignored --nocapture search_cost`.
    #[test]
    #[ignore]
    fn search_cost() {
        for lines in [10_000usize, 100_000] {
            let (s, _rx, _d) = spawn(None, 40, 120, lines);
            wait_screen(&s, Duration::from_secs(5), |sc| sc.text().contains('$'));
            s.write(format!("seq 1 {lines}; echo DONE-MARK\r").as_bytes());
            wait_screen(&s, Duration::from_secs(60), |sc| sc.text().contains("DONE-MARK"));
            let f = s.searcher();
            let go = || false;
            let mut q = TermQuery::new("12345", false, false, false).unwrap();
            let t = Instant::now();
            let m = f.find(&mut q, None, true, &go);
            let find = t.elapsed();
            let mut rare = TermQuery::new("not-there-at-all", false, false, false).unwrap();
            let t = Instant::now();
            let none = f.find(&mut rare, None, true, &go);
            let miss = t.elapsed();
            let t = Instant::now();
            let n = f.count(&mut q, m, 1000, &go);
            let count = t.elapsed();
            // The longest the lock is held: one chunk.
            let t = Instant::now();
            let _ = f.count(&mut rare, None, 1000, &|| t.elapsed() > Duration::ZERO);
            let mut screen = Screen::default();
            let t = Instant::now();
            s.screen_with_matches(&mut screen, &mut q, m);
            let draw = t.elapsed();
            eprintln!(
                "{lines} lines: find {find:?}, no match (whole scrollback) {miss:?} ({none:?}), count {n:?} {count:?}, \
                 screen+highlights {draw:?}"
            );
        }
    }
}
