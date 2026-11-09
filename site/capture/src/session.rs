//! One run of the real `yace` binary in a pseudo-terminal, with an
//! emulated screen (alacritty_terminal, the emulator YACE itself uses),
//! scripted keys, waits on screen contents, and asciicast v2 recording.
//!
//! Spawning and the keyboard capability probe follow
//! `crates/yace/tests/pty_smoke.rs` and `pty_terminal.rs`.

use std::{
    io::{Read, Write},
    path::Path,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

use alacritty_terminal::{
    Term,
    event::VoidListener,
    grid::Dimensions,
    term::Config,
    vte::ansi::{Processor, StdSyncHandler},
};
use portable_pty::{Child, CommandBuilder, MasterPty, PtySize, native_pty_system};

use crate::render::{self, Defaults, Frame};

pub struct Size {
    pub rows: usize,
    pub cols: usize,
}

impl Dimensions for Size {
    fn total_lines(&self) -> usize {
        self.rows
    }
    fn screen_lines(&self) -> usize {
        self.rows
    }
    fn columns(&self) -> usize {
        self.cols
    }
}

pub fn new_term(rows: usize, cols: usize) -> Term<VoidListener> {
    Term::new(Config::default(), &Size { rows, cols }, VoidListener)
}

/// What the reader thread shares with the script.
struct Shared {
    term: Term<VoidListener>,
    parser: Processor<StdSyncHandler>,
    /// Everything YACE wrote, for the asciicast.
    events: Vec<(f64, String)>,
    /// Incomplete UTF-8 at the end of the last read.
    pending: Vec<u8>,
    last_output: Instant,
}

pub struct Session {
    shared: Arc<Mutex<Shared>>,
    writer: Box<dyn Write + Send>,
    child: Box<dyn Child + Send + Sync>,
    _master: Box<dyn MasterPty + Send>,
    started: Instant,
    defaults: Defaults,
    rows: usize,
    cols: usize,
    /// Index into `events` and time where the current recording starts.
    mark: Option<(usize, f64)>,
    /// Pause between typed characters (longer while recording).
    pub type_delay: Duration,
}

pub struct Recording {
    pub cast: String,
    pub frames: Vec<(Frame, f32)>,
}

impl Session {
    pub fn spawn(
        yace: &Path,
        args: &[&str],
        env: &[(String, String)],
        cwd: &Path,
        rows: usize,
        cols: usize,
        defaults: Defaults,
    ) -> Session {
        let pair = native_pty_system()
            .openpty(PtySize { rows: rows as u16, cols: cols as u16, pixel_width: 0, pixel_height: 0 })
            .expect("openpty");
        let mut cmd = CommandBuilder::new(yace);
        cmd.args(args);
        cmd.cwd(cwd);
        cmd.env_clear();
        for (k, v) in env {
            cmd.env(k, v);
        }
        let child = pair.slave.spawn_command(cmd).expect("spawn yace");
        drop(pair.slave);
        let now = Instant::now();
        let shared = Arc::new(Mutex::new(Shared {
            term: new_term(rows, cols),
            parser: Processor::new(),
            events: Vec::new(),
            pending: Vec::new(),
            last_output: now,
        }));
        let probed = Arc::new(AtomicBool::new(false));
        let mut reader = pair.master.try_clone_reader().expect("pty reader");
        let (s2, p2) = (Arc::clone(&shared), Arc::clone(&probed));
        thread::spawn(move || {
            let mut buf = [0u8; 16384];
            let mut tail: Vec<u8> = Vec::new();
            while let Ok(n) = reader.read(&mut buf) {
                if n == 0 {
                    break;
                }
                let bytes = &buf[..n];
                // The keyboard capability probe ends with a primary device
                // attributes query (`CSI c`).
                tail.extend_from_slice(bytes);
                if !p2.load(Ordering::Relaxed) && tail.windows(3).any(|w| w == b"\x1b[c") {
                    p2.store(true, Ordering::Relaxed);
                }
                if tail.len() > 8 {
                    tail.drain(..tail.len() - 8);
                }
                let mut s = s2.lock().unwrap();
                let s = &mut *s;
                s.parser.advance(&mut s.term, bytes);
                s.pending.extend_from_slice(bytes);
                let valid = match std::str::from_utf8(&s.pending) {
                    Ok(_) => s.pending.len(),
                    Err(e) if e.error_len().is_none() => e.valid_up_to(),
                    Err(_) => s.pending.len(),
                };
                let text = String::from_utf8_lossy(&s.pending[..valid]).into_owned();
                s.pending.drain(..valid);
                let t = now.elapsed().as_secs_f64();
                s.events.push((t, text));
                s.last_output = Instant::now();
            }
        });
        let writer = pair.master.take_writer().expect("pty writer");
        let mut s = Session {
            shared,
            writer,
            child,
            _master: pair.master,
            started: now,
            defaults,
            rows,
            cols,
            mark: None,
            type_delay: Duration::from_millis(15),
        };
        // Answer the probe: no Kitty keyboard reply, so YACE uses the legacy
        // protocol and its documented fallbacks.
        let t = Instant::now();
        while !probed.load(Ordering::Relaxed) && t.elapsed() < Duration::from_secs(5) {
            thread::sleep(Duration::from_millis(10));
        }
        s.raw(b"\x1b[?62c");
        s
    }

    pub fn frame(&self) -> Frame {
        render::snapshot(&self.shared.lock().unwrap().term, self.defaults)
    }

    pub fn text(&self) -> String {
        self.frame().text()
    }

    pub fn raw(&mut self, bytes: &[u8]) {
        self.writer.write_all(bytes).expect("write to pty");
        self.writer.flush().expect("flush pty");
    }

    /// One key (an escape sequence or a control byte), then a short pause
    /// so the next key is read separately.
    pub fn key(&mut self, bytes: &[u8]) {
        self.raw(bytes);
        thread::sleep(Duration::from_millis(60));
    }

    pub fn typ(&mut self, text: &str) {
        for ch in text.chars() {
            let mut b = [0u8; 4];
            self.raw(ch.encode_utf8(&mut b).as_bytes());
            thread::sleep(self.type_delay);
        }
    }

    pub fn pause(&self, ms: u64) {
        thread::sleep(Duration::from_millis(ms));
    }

    /// Wait until the screen shows `needle`.
    pub fn wait(&self, needle: &str) {
        self.wait_until(needle, |t| t.contains(needle));
    }

    /// Wait until the screen no longer shows `needle`.
    pub fn wait_gone(&self, needle: &str) {
        self.wait_until(&format!("not {needle}"), |t| !t.contains(needle));
    }

    pub fn wait_until(&self, what: &str, f: impl Fn(&str) -> bool) {
        let t = Instant::now();
        loop {
            let text = self.text();
            if f(&text) {
                break;
            }
            if t.elapsed() > Duration::from_secs(20) {
                panic!("timed out waiting for {what:?}; the screen shows:\n{text}");
            }
            thread::sleep(Duration::from_millis(20));
        }
        self.settle();
    }

    /// Wait until YACE has written nothing for 250 ms.
    pub fn settle(&self) {
        let t = Instant::now();
        loop {
            let quiet = self.shared.lock().unwrap().last_output.elapsed();
            if quiet >= Duration::from_millis(250) || t.elapsed() > Duration::from_secs(10) {
                return;
            }
            thread::sleep(Duration::from_millis(20));
        }
    }

    pub fn record_start(&mut self) {
        self.settle();
        let s = self.shared.lock().unwrap();
        self.mark = Some((s.events.len(), self.started.elapsed().as_secs_f64()));
        drop(s);
        self.type_delay = Duration::from_millis(90);
    }

    /// End the recording: the asciicast (everything before the start is
    /// folded into its first event, so playback begins on the current
    /// screen) and the frames for the animated SVG.
    pub fn record_stop(&mut self, title: &str) -> Recording {
        self.settle();
        self.type_delay = Duration::from_millis(15);
        let (idx, t0) = self.mark.take().expect("record_start first");
        let events = self.shared.lock().unwrap().events.clone();
        let before: String = events[..idx].iter().map(|(_, s)| s.as_str()).collect();
        let after: Vec<(f64, String)> = events[idx..].iter().map(|(t, s)| ((t - t0).max(0.0), s.clone())).collect();

        let header = serde_json::json!({
            "version": 2,
            "width": self.cols,
            "height": self.rows,
            "title": title,
            "idle_time_limit": 2.0,
            "env": {"TERM": "xterm-256color", "SHELL": "/bin/sh"},
        });
        let mut cast = format!("{header}\n");
        cast.push_str(&format!("{}\n", serde_json::json!([0.0, "o", before])));
        for (t, s) in &after {
            cast.push_str(&format!("{}\n", serde_json::json!([(t * 1000.0).round() / 1000.0, "o", s])));
        }

        // Frames: replay into a fresh screen and keep the screen at the end
        // of each burst of output (a gap of 40 ms or more).
        let mut term = new_term(self.rows, self.cols);
        let mut parser: Processor<StdSyncHandler> = Processor::new();
        parser.advance(&mut term, before.as_bytes());
        let mut stamps: Vec<(Frame, f64)> = vec![(render::snapshot(&term, self.defaults), 0.0)];
        for (i, (t, s)) in after.iter().enumerate() {
            parser.advance(&mut term, s.as_bytes());
            let burst_ends = after.get(i + 1).is_none_or(|(next, _)| next - t >= 0.04);
            if burst_ends {
                let f = render::snapshot(&term, self.defaults);
                if stamps.last().is_some_and(|(last, _)| *last != f) {
                    stamps.push((f, *t));
                }
            }
        }
        let mut frames = Vec::new();
        for i in 0..stamps.len() {
            let dur = match stamps.get(i + 1) {
                Some((_, next)) => (next - stamps[i].1).clamp(0.06, 1.8) as f32,
                None => 3.0,
            };
            frames.push((stamps[i].0.clone(), dur));
        }
        // Hold the opening screen long enough to read it.
        if let Some(first) = frames.first_mut() {
            first.1 = first.1.max(1.2);
        }
        Recording { cast, frames }
    }

    /// Quit YACE with Ctrl+Q and reap it. If it is still running after
    /// 5 s (a scene that left unsaved edits gets a "save changes?" prompt),
    /// kill it: the demo files are thrown away anyway.
    pub fn quit(mut self) {
        self.raw(b"\x11");
        let t = Instant::now();
        while t.elapsed() < Duration::from_secs(5) {
            if self.child.try_wait().ok().flatten().is_some() {
                return;
            }
            thread::sleep(Duration::from_millis(30));
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        if self.child.try_wait().ok().flatten().is_none() {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }
}
