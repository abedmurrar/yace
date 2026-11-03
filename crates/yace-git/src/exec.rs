//! Running the `git` CLI (spec §15.4): every call has a timeout, never
//! prompts, never opens an editor or pager, and reads never take the
//! index lock. Every call is recorded for "Git: Show Output".

use std::{
    collections::VecDeque,
    io::{Read, Write},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{Arc, Mutex, mpsc},
    thread,
    time::{Duration, Instant},
};

/// Commands kept in the log.
pub const LOG_LEN: usize = 300;

#[derive(Debug, Clone, PartialEq)]
pub enum GitError {
    /// The git program could not be started.
    NotFound(String),
    /// git exited with an error.
    Failed {
        code: Option<i32>,
        stderr: String,
    },
    Timeout(Duration),
    Io(String),
}

impl std::fmt::Display for GitError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            GitError::NotFound(p) => write!(f, "git was not found ({p})"),
            GitError::Failed { stderr, code } => {
                let msg = stderr.trim();
                if msg.is_empty() { write!(f, "git failed (exit {code:?})") } else { f.write_str(msg) }
            }
            GitError::Timeout(d) => write!(f, "git did not finish within {} s", d.as_secs()),
            GitError::Io(e) => f.write_str(e),
        }
    }
}

impl std::error::Error for GitError {}

/// One logged git call.
#[derive(Debug, Clone, PartialEq)]
pub struct LogEntry {
    pub cwd: PathBuf,
    pub args: Vec<String>,
    pub millis: u64,
    /// `None` on success, else the error text.
    pub error: Option<String>,
    /// Whether the command can change the repository.
    pub mutating: bool,
}

impl LogEntry {
    pub fn line(&self) -> String {
        let status = match &self.error {
            None => "ok".to_string(),
            Some(e) => format!("FAILED: {}", e.lines().next().unwrap_or("")),
        };
        format!("[{} ms] git {}   ({})  {status}", self.millis, self.args.join(" "), self.cwd.display())
    }
}

/// The git program and its call log (cheap to clone; shared log).
#[derive(Debug, Clone)]
pub struct Git {
    pub program: PathBuf,
    pub timeout: Duration,
    log: Arc<Mutex<VecDeque<LogEntry>>>,
}

impl Default for Git {
    fn default() -> Self {
        Self::new("git")
    }
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

impl Git {
    pub fn new(program: impl Into<PathBuf>) -> Self {
        Self { program: program.into(), timeout: Duration::from_secs(30), log: Arc::default() }
    }

    pub fn log(&self) -> Vec<LogEntry> {
        lock(&self.log).iter().cloned().collect()
    }

    /// Run a read-only command in `cwd`; stdout on success.
    pub fn read(&self, cwd: &Path, args: &[&str]) -> Result<Vec<u8>, GitError> {
        self.run(cwd, args, None, false, &[])
    }

    /// Like `read`, also accepting these non-zero exit codes as success
    /// (`git diff --no-index` exits 1 when the files differ).
    pub fn read_ok(&self, cwd: &Path, args: &[&str], ok_codes: &[i32]) -> Result<Vec<u8>, GitError> {
        self.run(cwd, args, None, false, ok_codes)
    }

    /// Run a command that changes the repository (only on explicit user
    /// action, spec §15.4). `stdin` is fed to git (commit messages).
    pub fn write(&self, cwd: &Path, args: &[&str], stdin: Option<&[u8]>) -> Result<Vec<u8>, GitError> {
        self.run(cwd, args, stdin, true, &[])
    }

    fn run(
        &self,
        cwd: &Path,
        args: &[&str],
        stdin: Option<&[u8]>,
        mutating: bool,
        ok_codes: &[i32],
    ) -> Result<Vec<u8>, GitError> {
        let start = Instant::now();
        let r = self.run_inner(cwd, args, stdin, mutating, ok_codes);
        let entry = LogEntry {
            cwd: cwd.to_path_buf(),
            args: args.iter().map(|a| a.to_string()).collect(),
            millis: start.elapsed().as_millis() as u64,
            error: r.as_ref().err().map(ToString::to_string),
            mutating,
        };
        let mut log = lock(&self.log);
        if log.len() >= LOG_LEN {
            log.pop_front();
        }
        log.push_back(entry);
        r
    }

    fn run_inner(
        &self,
        cwd: &Path,
        args: &[&str],
        stdin: Option<&[u8]>,
        mutating: bool,
        ok_codes: &[i32],
    ) -> Result<Vec<u8>, GitError> {
        let mut cmd = Command::new(&self.program);
        if !mutating {
            // Reads never take the index lock (they would race with the
            // user's own git commands).
            cmd.arg("--no-optional-locks");
        }
        cmd.args(["-c", "core.quotepath=off", "-c", "color.ui=false"])
            .args(args)
            .current_dir(cwd)
            .env("LC_ALL", "C")
            .env("GIT_TERMINAL_PROMPT", "0")
            .env("GIT_PAGER", "cat")
            .env("GIT_EDITOR", "true")
            .env("GIT_ASKPASS", "")
            .env("SSH_ASKPASS", "")
            .stdin(if stdin.is_some() { Stdio::piped() } else { Stdio::null() })
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            cmd.process_group(0); // Ctrl+C in the terminal never reaches it
        }
        let mut child = cmd.spawn().map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                GitError::NotFound(self.program.display().to_string())
            } else {
                GitError::Io(e.to_string())
            }
        })?;
        if let (Some(data), Some(mut sin)) = (stdin, child.stdin.take()) {
            let data = data.to_vec();
            thread::spawn(move || {
                let _ = sin.write_all(&data);
            });
        }
        let mut out = child.stdout.take().ok_or_else(|| GitError::Io("no stdout".into()))?;
        let mut err = child.stderr.take().ok_or_else(|| GitError::Io("no stderr".into()))?;
        let err_thread = thread::spawn(move || {
            let mut s = Vec::new();
            let _ = err.read_to_end(&mut s);
            s
        });
        // Watchdog: kill git if it outlives the timeout.
        let (done_tx, done_rx) = mpsc::channel::<()>();
        let pid = child.id();
        let timeout = self.timeout;
        let watchdog = thread::spawn(move || {
            if done_rx.recv_timeout(timeout).is_err() {
                let _ = Command::new("kill").args(["-KILL", &pid.to_string()]).status();
                return true;
            }
            false
        });
        let mut stdout = Vec::new();
        let _ = out.read_to_end(&mut stdout);
        let status = child.wait().map_err(|e| GitError::Io(e.to_string()))?;
        let _ = done_tx.send(());
        let timed_out = watchdog.join().unwrap_or(false);
        let stderr = String::from_utf8_lossy(&err_thread.join().unwrap_or_default()).into_owned();
        if timed_out {
            return Err(GitError::Timeout(timeout));
        }
        if status.success() || status.code().is_some_and(|c| ok_codes.contains(&c)) {
            Ok(stdout)
        } else {
            let tail: String = {
                let lines: Vec<&str> = stderr.lines().collect();
                lines[lines.len().saturating_sub(20)..].join("\n")
            };
            Err(GitError::Failed { code: status.code(), stderr: tail })
        }
    }
}
