//! One language server process: writer, reader and stderr threads. The
//! reader decodes responses into typed `Reply`s (it knows each request's
//! method), so the UI thread only ever sees ready-to-use data.

use std::{
    collections::{HashMap, VecDeque},
    io::{self, BufRead, BufReader},
    path::PathBuf,
    process::{Child, Command, Stdio},
    sync::{Arc, Mutex},
    thread,
    time::Duration,
};

use crossbeam_channel::{Sender, unbounded};
use serde_json::Value;

use crate::{
    decode::{Incoming, decode_notification, decode_reply},
    jsonrpc,
};

/// How to start a server.
#[derive(Debug, Clone, PartialEq)]
pub struct Launch {
    pub program: PathBuf,
    pub args: Vec<String>,
    pub env: Vec<(String, String)>,
    pub cwd: PathBuf,
}

/// Lines of stderr kept per server ("Show Language Server Output").
pub const STDERR_LINES: usize = 500;

pub type Pending = Arc<Mutex<HashMap<u64, String>>>;

pub struct Client {
    tx: Sender<Value>,
    pending: Pending,
    next_id: u64,
    child: Option<Child>,
    pid: u32,
    pub stderr: Arc<Mutex<VecDeque<String>>>,
}

impl std::fmt::Debug for Client {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Client(pid {})", self.pid)
    }
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

impl Client {
    /// Spawn the server. Messages arrive on `events` tagged with `server`.
    pub fn spawn(server: u32, launch: &Launch, events: Sender<(u32, Incoming)>) -> io::Result<Client> {
        let mut cmd = Command::new(&launch.program);
        cmd.args(&launch.args)
            .current_dir(&launch.cwd)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        for (k, v) in &launch.env {
            cmd.env(k, v);
        }
        #[cfg(unix)]
        {
            // Its own process group: terminal signals never reach it, and the
            // whole group (e.g. tsserver children) can be stopped together.
            use std::os::unix::process::CommandExt;
            cmd.process_group(0);
        }
        let mut child = cmd.spawn()?;
        let pid = child.id();
        let stdin = child.stdin.take().ok_or_else(|| io::Error::other("no stdin"))?;
        let stdout = child.stdout.take().ok_or_else(|| io::Error::other("no stdout"))?;
        let stderr_pipe = child.stderr.take().ok_or_else(|| io::Error::other("no stderr"))?;
        let (tx, rx) = unbounded::<Value>();
        thread::Builder::new().name("yace-lsp-write".into()).spawn(move || {
            let mut w = io::BufWriter::new(stdin);
            for msg in rx {
                if jsonrpc::write_message(&mut w, &msg).is_err() {
                    break;
                }
            }
        })?;
        let pending: Pending = Arc::default();
        let p = pending.clone();
        let stderr_done = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let done = stderr_done.clone();
        thread::Builder::new().name("yace-lsp-read".into()).spawn(move || {
            let mut r = BufReader::with_capacity(64 * 1024, stdout);
            let mut buf = Vec::new();
            while let Ok(Some(msg)) = jsonrpc::read_message(&mut r, &mut buf) {
                if let Some(inc) = classify(msg, &p)
                    && events.send((server, inc)).is_err()
                {
                    return;
                }
                // Don't keep a huge buffer after one big message.
                if buf.capacity() > 4 << 20 {
                    buf = Vec::new();
                }
            }
            // Let stderr drain so the exit report can say why.
            for _ in 0..30 {
                if done.load(std::sync::atomic::Ordering::Acquire) {
                    break;
                }
                thread::sleep(Duration::from_millis(10));
            }
            let _ = events.send((server, Incoming::Exited));
        })?;
        let ring: Arc<Mutex<VecDeque<String>>> = Arc::default();
        let rr = ring.clone();
        thread::Builder::new().name("yace-lsp-stderr".into()).spawn(move || {
            for line in BufReader::new(stderr_pipe).lines() {
                let Ok(line) = line else { break };
                let mut q = lock(&rr);
                if q.len() >= STDERR_LINES {
                    q.pop_front();
                }
                q.push_back(line);
            }
            stderr_done.store(true, std::sync::atomic::Ordering::Release);
        })?;
        Ok(Client { tx, pending, next_id: 0, child: Some(child), pid, stderr: ring })
    }

    pub fn pid(&self) -> u32 {
        self.pid
    }

    /// Send a request; the response arrives as `Incoming::Response { id }`.
    pub fn request(&mut self, method: &str, params: Value) -> u64 {
        self.next_id += 1;
        let id = self.next_id;
        lock(&self.pending).insert(id, method.to_string());
        let _ = self.tx.send(jsonrpc::request(id, method, params));
        id
    }

    /// Forget a request (timed out or superseded); its late response is dropped.
    pub fn forget(&self, id: u64) {
        lock(&self.pending).remove(&id);
    }

    pub fn cancel(&self, id: u64) {
        self.forget(id);
        self.notify("$/cancelRequest", serde_json::json!({ "id": id }));
    }

    pub fn notify(&self, method: &str, params: Value) {
        let _ = self.tx.send(jsonrpc::notification(method, params));
    }

    pub fn respond(&self, id: Value, result: Value) {
        let _ = self.tx.send(jsonrpc::response(id, result));
    }

    /// Exit status if the process has ended.
    pub fn try_exit_code(&mut self) -> Option<Option<i32>> {
        let c = self.child.as_mut()?;
        match c.try_wait() {
            Ok(Some(s)) => Some(s.code()),
            _ => None,
        }
    }

    /// Ask the server to shut down; a background reaper makes sure the
    /// whole process group is gone within a few seconds. Never blocks.
    pub fn shutdown(&mut self) {
        let Some(mut child) = self.child.take() else { return };
        self.next_id += 1;
        let _ = self.tx.send(jsonrpc::request(self.next_id, "shutdown", Value::Null));
        let _ = self.tx.send(jsonrpc::notification("exit", Value::Null));
        let pid = self.pid;
        let _ = thread::Builder::new().name("yace-lsp-reap".into()).spawn(move || {
            for _ in 0..20 {
                if matches!(child.try_wait(), Ok(Some(_))) {
                    break;
                }
                thread::sleep(Duration::from_millis(100));
            }
            if !matches!(child.try_wait(), Ok(Some(_))) {
                kill_group(pid, "TERM");
                thread::sleep(Duration::from_millis(500));
                kill_group(pid, "KILL");
                let _ = child.wait();
            } else {
                // The leader exited; stop any helpers it left behind.
                kill_group(pid, "TERM");
            }
        });
    }
}

fn kill_group(pid: u32, sig: &str) {
    let _ = Command::new("kill")
        .arg(format!("-{sig}"))
        .arg("--")
        .arg(format!("-{pid}"))
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
}

impl Drop for Client {
    fn drop(&mut self) {
        self.shutdown();
    }
}

fn classify(mut msg: Value, pending: &Pending) -> Option<Incoming> {
    let method = msg.get("method").and_then(Value::as_str).map(String::from);
    let id = msg.get_mut("id").map(Value::take);
    match (id, method) {
        (Some(id), Some(method)) => {
            let params = msg.get_mut("params").map(Value::take).unwrap_or(Value::Null);
            Some(Incoming::Request { id, method, params })
        }
        (Some(id), None) => {
            let id = id.as_u64()?;
            let method = lock(pending).remove(&id)?;
            let result = match msg.get_mut("error") {
                Some(e) if !e.is_null() => {
                    Err(e.get("message").and_then(Value::as_str).unwrap_or("request failed").to_string())
                }
                _ => Ok(decode_reply(&method, msg.get_mut("result").map(Value::take).unwrap_or(Value::Null))),
            };
            Some(Incoming::Response { id, method, result })
        }
        (None, Some(method)) => {
            decode_notification(&method, msg.get_mut("params").map(Value::take).unwrap_or(Value::Null))
        }
        (None, None) => None,
    }
}
