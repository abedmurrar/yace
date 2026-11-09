//! Language server state (spec §13): one server process per (server,
//! workspace root), started lazily when a file of its language opens,
//! stopped when idle, restarted (a few times) after a crash. Nothing here
//! blocks: messages arrive as events, requests are answered asynchronously
//! and stale answers are dropped.

use std::{
    collections::{BTreeMap, HashMap, HashSet, VecDeque},
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

use crossbeam_channel::{Receiver, Sender, unbounded};
use serde_json::{Value, json};
use yace_buffer::{ChangeSet, DocEdit, Rope};
use yace_lsp::{Client, Encoding, Incoming, Launch, ServerCaps, ServerDef, to_lsp};

use crate::editor::DocId;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Starting,
    Running,
    /// Stopped by the user, idle shutdown or too many crashes.
    Stopped,
}

pub struct Server {
    pub id: u32,
    pub def: ServerDef,
    /// The resolved program (e.g. TypeScript 7's native `tsc`).
    pub program: PathBuf,
    pub root: PathBuf,
    pub root_name: String,
    pub client: Option<Client>,
    pub status: Status,
    pub caps: ServerCaps,
    /// Documents opened on the server.
    pub docs: HashSet<DocId>,
    /// Documents to open once `initialize` completes.
    pub waiting: Vec<DocId>,
    pub crashes: Vec<Instant>,
    pub idle_since: Option<Instant>,
    /// `window/logMessage` lines.
    pub log: VecDeque<String>,
    /// Work-done progress by token: (text, percentage).
    pub progress: BTreeMap<String, (String, Option<u32>)>,
    pub init_id: Option<u64>,
    pub started: Instant,
}

impl Server {
    pub fn encoding(&self) -> Encoding {
        self.caps.encoding
    }

    pub fn status_text(&self) -> String {
        match self.status {
            Status::Starting => format!("{} (starting)", self.def.name),
            Status::Stopped => format!("{} (stopped)", self.def.name),
            Status::Running => match self.progress.values().next() {
                Some((t, Some(p))) => format!("{}: {t} {p}%", self.def.name),
                Some((t, None)) => format!("{}: {t}", self.def.name),
                None => self.def.name.clone(),
            },
        }
    }
}

/// A document synced to a server.
#[derive(Debug, Clone)]
pub struct LspDoc {
    pub server: u32,
    pub uri: String,
    pub version: i32,
    pub language: &'static str,
    /// A pull-diagnostics request is in flight.
    pub pulling: bool,
    /// The text changed while it was in flight: pull again afterwards.
    pub pull_again: bool,
}

/// What a pending request is for (its answer is applied accordingly).
#[derive(Debug, Clone, PartialEq)]
pub enum ReqKind {
    Completion { seq: u64 },
    CompletionResolve { seq: u64, label: String, apply_edits: bool },
    Hover,
    SignatureHelp { seq: u64 },
    Goto { what: &'static str },
    References,
    Rename,
    DocSymbols,
    WorkspaceSymbols { seq: u64 },
    CodeActions,
    CodeActionResolve,
    Format { then_save: bool },
    ExecuteCommand,
    PullDiagnostics,
}

impl ReqKind {
    /// Shown to the user when it times out.
    pub fn user_facing(&self) -> Option<&'static str> {
        match self {
            ReqKind::Goto { what } => Some(what),
            ReqKind::References => Some("Find references"),
            ReqKind::Rename => Some("Rename"),
            ReqKind::CodeActions => Some("Code actions"),
            ReqKind::Format { .. } => Some("Formatting"),
            _ => None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Pending {
    pub kind: ReqKind,
    pub doc: Option<DocId>,
    /// Document version when sent (stale answers are dropped).
    pub version: u64,
    pub sent: Instant,
}

/// A diagnostic, positioned in chars for open documents (mapped through
/// edits until the server republishes) and by LSP position otherwise.
#[derive(Debug, Clone, PartialEq)]
pub struct Diag {
    pub severity: u8,
    pub message: String,
    pub source: Option<String>,
    pub code: Option<String>,
    pub range: yace_lsp::Range,
    pub chars: Option<(usize, usize)>,
    pub server: u32,
}

impl Diag {
    pub fn severity_name(&self) -> &'static str {
        match self.severity {
            1 => "error",
            2 => "warning",
            3 => "info",
            _ => "hint",
        }
    }

    /// (line, column in chars) for display.
    pub fn line_col(&self, rope: Option<&Rope>) -> (usize, usize) {
        match (rope, self.chars) {
            (Some(r), Some((a, _))) => {
                let a = a.min(r.len_chars());
                let l = r.char_to_line(a);
                (l, a - r.line_to_char(l))
            }
            _ => (self.range.start.line as usize, self.range.start.character as usize),
        }
    }
}

pub struct LspState {
    pub servers: Vec<Server>,
    next_server: u32,
    tx: Sender<(u32, Incoming)>,
    rx: Receiver<(u32, Incoming)>,
    pub docs: HashMap<DocId, LspDoc>,
    pub pending: HashMap<(u32, u64), Pending>,
    /// By canonical path.
    pub diagnostics: BTreeMap<PathBuf, Vec<Diag>>,
    /// Languages whose "install a server" hint was shown.
    pub hinted: HashSet<String>,
    pub seq: u64,
}

impl Default for LspState {
    fn default() -> Self {
        let (tx, rx) = unbounded();
        Self {
            servers: Vec::new(),
            next_server: 0,
            tx,
            rx,
            docs: HashMap::new(),
            pending: HashMap::new(),
            diagnostics: BTreeMap::new(),
            hinted: HashSet::new(),
            seq: 0,
        }
    }
}

/// What YACE can do, so servers only send what the UI shows (§13.1).
pub fn client_capabilities() -> Value {
    json!({
        "general": { "positionEncodings": ["utf-8", "utf-16"] },
        "workspace": {
            "configuration": true,
            "workspaceFolders": true,
            "applyEdit": true,
            "workspaceEdit": { "documentChanges": true },
            "symbol": { "dynamicRegistration": false },
            "didChangeConfiguration": { "dynamicRegistration": false },
            "executeCommand": { "dynamicRegistration": false },
            "diagnostics": { "refreshSupport": true }
        },
        "window": { "workDoneProgress": true },
        "textDocument": {
            "synchronization": { "didSave": true, "willSave": false, "dynamicRegistration": false },
            "completion": {
                "completionItem": {
                    "snippetSupport": true,
                    "insertReplaceSupport": true,
                    "resolveSupport": { "properties": ["additionalTextEdits", "documentation", "detail"] },
                    "documentationFormat": ["markdown", "plaintext"]
                },
                "contextSupport": true
            },
            "hover": { "contentFormat": ["markdown", "plaintext"] },
            "signatureHelp": {
                "signatureInformation": {
                    "parameterInformation": { "labelOffsetSupport": true },
                    "activeParameterSupport": true
                }
            },
            "definition": { "linkSupport": true },
            "declaration": { "linkSupport": true },
            "typeDefinition": { "linkSupport": true },
            "implementation": { "linkSupport": true },
            "references": {},
            "documentSymbol": { "hierarchicalDocumentSymbolSupport": true },
            "codeAction": {
                "codeActionLiteralSupport": { "codeActionKind": { "valueSet": [
                    "", "quickfix", "refactor", "refactor.extract", "refactor.inline", "refactor.rewrite",
                    "source", "source.organizeImports", "source.fixAll"
                ] } },
                "resolveSupport": { "properties": ["edit"] },
                "isPreferredSupport": true,
                "disabledSupport": true
            },
            "formatting": {},
            "rangeFormatting": {},
            "rename": { "prepareSupport": false },
            "publishDiagnostics": { "versionSupport": true },
            "diagnostic": { "dynamicRegistration": false, "relatedDocumentSupport": false }
        }
    })
}

fn toml_to_json(v: Option<&toml::Value>) -> Value {
    v.and_then(|v| serde_json::to_value(v).ok()).unwrap_or(Value::Null)
}

/// User server definitions from settings.
pub fn user_defs(s: &yace_config::LspSettings) -> Vec<ServerDef> {
    s.servers
        .iter()
        .filter(|u| !u.name.is_empty() && !u.command.is_empty())
        .map(|u| ServerDef {
            name: u.name.clone(),
            command: u.command.clone(),
            args: u.args.clone(),
            languages: u.languages.clone(),
            env: u.env.iter().map(|(k, v)| (k.clone(), v.clone())).collect(),
            init_options: toml_to_json(u.initialization_options.as_ref()),
            settings: toml_to_json(u.settings.as_ref()),
            install_hint: String::new(),
            needs_node: false,
            kind: Default::default(),
        })
        .collect()
}

/// Incremental `didChange` content changes for a batch of edits. Each
/// change set is applied last-change-first, so every change is expressed
/// against the text as it is at that moment — exactly what LSP expects for
/// sequentially applied changes.
pub fn content_changes(edits: &[DocEdit], enc: Encoding) -> Vec<Value> {
    let mut out = Vec::new();
    for e in edits {
        for c in e.changes.changes().iter().rev() {
            let start = to_lsp(&e.before, c.from, enc);
            let end = to_lsp(&e.before, c.to, enc);
            out.push(json!({ "range": { "start": start, "end": end }, "text": c.insert }));
        }
    }
    out
}

/// Map diagnostic char ranges through an edit.
pub fn map_diags(diags: &mut [Diag], cs: &ChangeSet) {
    for d in diags {
        if let Some((a, b)) = d.chars {
            let na = cs.map_pos(a, yace_buffer::Assoc::After);
            let nb = cs.map_pos(b, yace_buffer::Assoc::Before).max(na);
            d.chars = Some((na, nb));
        }
    }
}

impl LspState {
    pub fn events(&self) -> Receiver<(u32, Incoming)> {
        self.rx.clone()
    }

    pub fn try_events(&self) -> Vec<(u32, Incoming)> {
        self.rx.try_iter().collect()
    }

    pub fn server(&self, id: u32) -> Option<&Server> {
        self.servers.iter().find(|s| s.id == id)
    }

    pub fn server_mut(&mut self, id: u32) -> Option<&mut Server> {
        self.servers.iter_mut().find(|s| s.id == id)
    }

    pub fn find(&self, name: &str, root: &Path) -> Option<u32> {
        self.servers.iter().find(|s| s.def.name == name && s.root == root).map(|s| s.id)
    }

    /// Start a server process and send `initialize`.
    pub fn spawn(&mut self, def: ServerDef, program: PathBuf, root: PathBuf, root_name: String) -> Result<u32, String> {
        self.next_server += 1;
        let id = self.next_server;
        let mut server = Server {
            id,
            def,
            program: program.clone(),
            root,
            root_name,
            client: None,
            status: Status::Starting,
            caps: ServerCaps::default(),
            docs: HashSet::new(),
            waiting: Vec::new(),
            crashes: Vec::new(),
            idle_since: None,
            log: VecDeque::new(),
            progress: BTreeMap::new(),
            init_id: None,
            started: Instant::now(),
        };
        self.start_process(&mut server, program)?;
        self.servers.push(server);
        Ok(id)
    }

    fn start_process(&self, server: &mut Server, program: PathBuf) -> Result<(), String> {
        let launch =
            Launch { program, args: server.def.args.clone(), env: server.def.env.clone(), cwd: server.root.clone() };
        let mut client = Client::spawn(server.id, &launch, self.tx.clone()).map_err(|e| e.to_string())?;
        let root_uri = yace_lsp::uri::from_path(&server.root);
        let params = json!({
            "processId": std::process::id(),
            "clientInfo": { "name": "YACE", "version": env!("CARGO_PKG_VERSION") },
            "rootUri": root_uri,
            "rootPath": server.root.display().to_string(),
            "workspaceFolders": [{ "uri": root_uri, "name": server.root_name }],
            "initializationOptions": server.def.init_options,
            "capabilities": client_capabilities(),
            "trace": "off",
        });
        server.init_id = Some(client.request("initialize", params));
        server.client = Some(client);
        server.status = Status::Starting;
        server.started = Instant::now();
        server.caps = ServerCaps::default();
        server.progress.clear();
        Ok(())
    }

    /// Restart after a crash or on request. Documents are re-opened once
    /// the new process has initialised.
    pub fn restart(&mut self, id: u32) -> Result<(), String> {
        let Some(i) = self.servers.iter().position(|s| s.id == id) else { return Err("no such server".into()) };
        let program = self.servers[i].program.clone();
        if !program.is_file() {
            return Err(format!("{} no longer exists", program.display()));
        }
        let mut server = self.servers.remove(i);
        if let Some(mut c) = server.client.take() {
            c.shutdown();
        }
        server.waiting.extend(server.docs.drain());
        self.pending.retain(|(s, _), _| *s != id);
        let r = self.start_process(&mut server, program);
        self.servers.insert(i, server);
        r
    }

    pub fn stop(&mut self, id: u32) {
        if let Some(s) = self.server_mut(id) {
            if let Some(mut c) = s.client.take() {
                c.shutdown();
            }
            s.status = Status::Stopped;
            s.waiting.extend(s.docs.drain());
            s.progress.clear();
        }
        self.pending.retain(|(s, _), _| *s != id);
    }

    pub fn shutdown_all(&mut self) {
        for s in &mut self.servers {
            if let Some(mut c) = s.client.take() {
                c.shutdown();
            }
            s.status = Status::Stopped;
        }
        self.pending.clear();
    }

    /// Send a request on a running server, remembering what it is for.
    pub fn request(
        &mut self,
        server: u32,
        method: &str,
        params: Value,
        kind: ReqKind,
        doc: Option<DocId>,
        version: u64,
    ) -> Option<u64> {
        let s = self.servers.iter_mut().find(|s| s.id == server && s.status == Status::Running)?;
        let id = s.client.as_mut()?.request(method, params);
        self.pending.insert((server, id), Pending { kind, doc, version, sent: Instant::now() });
        Some(id)
    }

    /// Cancel outstanding requests of a kind (e.g. an older completion).
    pub fn cancel_where(&mut self, f: impl Fn(&ReqKind) -> bool) {
        let ids: Vec<(u32, u64)> = self.pending.iter().filter(|(_, p)| f(&p.kind)).map(|(k, _)| *k).collect();
        for (s, id) in ids {
            self.pending.remove(&(s, id));
            if let Some(c) = self.server(s).and_then(|s| s.client.as_ref()) {
                c.cancel(id);
            }
        }
    }

    pub fn notify(&self, server: u32, method: &str, params: Value) {
        if let Some(c) = self.server(server).filter(|s| s.status == Status::Running).and_then(|s| s.client.as_ref()) {
            c.notify(method, params);
        }
    }

    /// Requests older than `timeout` (dropped; returned for reporting).
    pub fn expire(&mut self, now: Instant, timeout: Duration) -> Vec<Pending> {
        let old: Vec<(u32, u64)> =
            self.pending.iter().filter(|(_, p)| now.duration_since(p.sent) >= timeout).map(|(k, _)| *k).collect();
        let mut out = Vec::new();
        for (s, id) in old {
            if let Some(p) = self.pending.remove(&(s, id)) {
                out.push(p);
            }
            if let Some(c) = self.server(s).and_then(|s| s.client.as_ref()) {
                c.forget(id);
            }
        }
        out
    }

    pub fn next_deadline(&self, timeout: Duration, idle: Option<Duration>) -> Option<Instant> {
        let req = self.pending.values().map(|p| p.sent + timeout).min();
        let idle = idle.and_then(|d| self.servers.iter().filter_map(|s| s.idle_since.map(|t| t + d)).min());
        [req, idle].into_iter().flatten().min()
    }

    /// Counts of (errors, warnings) across all files.
    pub fn counts(&self) -> (usize, usize) {
        let mut e = 0;
        let mut w = 0;
        for d in self.diagnostics.values().flatten() {
            match d.severity {
                1 => e += 1,
                2 => w += 1,
                _ => {}
            }
        }
        (e, w)
    }

    /// Resident memory of a server process (and its children) in KiB.
    pub fn rss_kib(pid: u32) -> u64 {
        fn one(pid: u32) -> u64 {
            std::fs::read_to_string(format!("/proc/{pid}/status"))
                .ok()
                .and_then(|s| {
                    s.lines().find(|l| l.starts_with("VmRSS:")).and_then(|l| l.split_whitespace().nth(1)?.parse().ok())
                })
                .unwrap_or(0)
        }
        // Children in the same process group (e.g. tsserver under
        // typescript-language-server).
        let mut total = one(pid);
        if let Ok(rd) = std::fs::read_dir("/proc") {
            for e in rd.flatten() {
                let Some(p) = e.file_name().to_str().and_then(|n| n.parse::<u32>().ok()) else { continue };
                if p == pid {
                    continue;
                }
                let stat = std::fs::read_to_string(format!("/proc/{p}/stat")).unwrap_or_default();
                let pgrp =
                    stat.rsplit(')').next().and_then(|r| r.split_whitespace().nth(2)).and_then(|x| x.parse().ok());
                if pgrp == Some(pid) {
                    total += one(p);
                }
            }
        }
        total
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use yace_buffer::{Change, Document, EditKind, Selection};

    #[test]
    fn content_changes_are_sequential() {
        let mut d = Document::from_text("ab\ncd\n");
        d.track_edits();
        let sel = Selection::point(0);
        // Two cursors typing "x" at 1 and 4 (one change set).
        let cs = ChangeSet::new(vec![Change::insert(1, "x"), Change::insert(4, "x")], 6);
        d.apply(cs, &sel, sel.clone(), EditKind::Insert);
        let ch = content_changes(&d.take_edits(), Encoding::Utf16);
        assert_eq!(ch.len(), 2);
        // Last change first: line 1 col 1, then line 0 col 1.
        assert_eq!(ch[0]["range"]["start"], json!({"line": 1, "character": 1}));
        assert_eq!(ch[1]["range"]["start"], json!({"line": 0, "character": 1}));
    }
}
