//! Performance instrumentation (spec §23): startup time, per-event handling
//! and render latency, resident memory. Cheap enough to be always on.

use std::time::{Duration, Instant};

const WINDOW: usize = 512;

#[derive(Debug, Default, Clone)]
pub struct Samples {
    buf: Vec<Duration>,
    next: usize,
    pub count: u64,
    pub max: Duration,
}

impl Samples {
    pub fn record(&mut self, d: Duration) {
        if self.buf.len() < WINDOW {
            self.buf.push(d);
        } else {
            self.buf[self.next] = d;
        }
        self.next = (self.next + 1) % WINDOW;
        self.count += 1;
        self.max = self.max.max(d);
    }
    pub fn percentile(&self, p: f64) -> Duration {
        if self.buf.is_empty() {
            return Duration::ZERO;
        }
        let mut v = self.buf.clone();
        v.sort_unstable();
        let idx = ((v.len() as f64 - 1.0) * p).round() as usize;
        v[idx.min(v.len() - 1)]
    }
}

#[derive(Debug, Clone)]
pub struct Metrics {
    pub process_start: Instant,
    pub first_frame: Option<Duration>,
    pub input: Samples,
    pub render: Samples,
    pub frames: u64,
}

impl Metrics {
    pub fn new(process_start: Instant) -> Self {
        Self { process_start, first_frame: None, input: Samples::default(), render: Samples::default(), frames: 0 }
    }

    pub fn report(&self) -> Vec<String> {
        let ms = |d: Duration| format!("{:.2} ms", d.as_secs_f64() * 1000.0);
        let mut out = vec![
            format!("Startup → first frame: {}", self.first_frame.map(ms).unwrap_or_else(|| "n/a".into())),
            format!(
                "Input handling: p50 {} · p95 {} · max {} ({} events)",
                ms(self.input.percentile(0.5)),
                ms(self.input.percentile(0.95)),
                ms(self.input.max),
                self.input.count
            ),
            format!(
                "Render: p50 {} · p95 {} · max {} ({} frames)",
                ms(self.render.percentile(0.5)),
                ms(self.render.percentile(0.95)),
                ms(self.render.max),
                self.frames
            ),
        ];
        match rss_kib() {
            Some(kib) => out.push(format!("Resident memory (editor process only): {:.1} MiB", kib as f64 / 1024.0)),
            None => out.push("Resident memory: unavailable".into()),
        }
        out
    }
}

/// Resident set size of this process in KiB (Linux `/proc`).
pub fn rss_kib() -> Option<u64> {
    let status = std::fs::read_to_string("/proc/self/status").ok()?;
    status
        .lines()
        .find(|l| l.starts_with("VmRSS:"))
        .and_then(|l| l.split_whitespace().nth(1))
        .and_then(|n| n.parse().ok())
}
