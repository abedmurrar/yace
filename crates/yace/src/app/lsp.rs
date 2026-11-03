//! Language server integration (spec §13): document sync, server messages,
//! diagnostics, and the request-based features — navigation, references,
//! rename, symbols, code actions, formatting. Completion, hover and
//! signature help popups are in `completion.rs`.

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

use serde_json::{Value, json};
use yace_buffer::{Change, ChangeSet, DocEdit, Selection};
use yace_lsp::{Encoding, Incoming, Reply, from_lsp, registry, to_lsp, uri};

use super::{App, Severity};
use crate::{
    actions::{PickerAction, PromptAction},
    editor::{DocId, path_key},
    lsp::{Diag, LspDoc, Pending, ReqKind, Status, content_changes, map_diags, user_defs},
    overlay::{LineInput, Overlay, Picker, PickerItem, Prompt},
};

/// A place to jump to (definition, reference, symbol, problem).
#[derive(Debug, Clone, PartialEq)]
pub struct NavTarget {
    pub path: PathBuf,
    pub pos: yace_lsp::Position,
    pub enc: Encoding,
}

pub const SYMBOL_KINDS: [&str; 27] = [
    "",
    "file",
    "module",
    "namespace",
    "package",
    "class",
    "method",
    "property",
    "field",
    "constructor",
    "enum",
    "interface",
    "function",
    "variable",
    "constant",
    "string",
    "number",
    "boolean",
    "array",
    "object",
    "key",
    "null",
    "enum member",
    "struct",
    "event",
    "operator",
    "type parameter",
];

pub fn symbol_kind(k: u32) -> &'static str {
    SYMBOL_KINDS.get(k as usize).copied().unwrap_or("")
}

const RESTART_WINDOW: Duration = Duration::from_secs(300);
const MAX_RESTARTS: usize = 3;

impl App {
    /// Server messages (the runtime forwards them as `AppEvent::Lsp`).
    pub fn lsp_events(&self) -> crossbeam_channel::Receiver<(u32, Incoming)> {
        self.lsp.events()
    }

    /// Handle pending server messages without blocking (tests).
    pub fn drain_lsp_events(&mut self) {
        for (s, m) in self.lsp.try_events() {
            self.on_lsp(s, m);
        }
    }

    fn lsp_timeout(&self) -> Duration {
        Duration::from_millis(self.settings.lsp.request_timeout_ms.max(500))
    }

    pub(crate) fn lsp_deadline(&self) -> Option<Instant> {
        let idle = Some(self.settings.lsp.idle_shutdown_secs).filter(|s| *s > 0).map(Duration::from_secs);
        [self.lsp.next_deadline(self.lsp_timeout(), idle), self.completion_deadline, self.format_save.map(|(_, t)| t)]
            .into_iter()
            .flatten()
            .min()
    }

    pub(crate) fn lsp_on_tick(&mut self, now: Instant) {
        for p in self.lsp.expire(now, self.lsp_timeout()) {
            if let Some(what) = p.kind.user_facing() {
                self.notify(Severity::Warning, format!("{what}: the language server did not answer in time"));
            }
            if let ReqKind::Format { then_save: true } = p.kind
                && let Some(d) = p.doc
            {
                self.format_save = None;
                self.save_doc(d, false);
            }
        }
        if self.completion_deadline.is_some_and(|t| now >= t) {
            self.completion_deadline = None;
            self.request_completion(None, false);
        }
        if let Some((d, t)) = self.format_save
            && now >= t
        {
            self.format_save = None;
            self.lsp.cancel_where(|k| matches!(k, ReqKind::Format { then_save: true }));
            self.save_doc(d, false);
        }
        let idle = self.settings.lsp.idle_shutdown_secs;
        if idle > 0 {
            let limit = Duration::from_secs(idle);
            let stale: Vec<u32> = self
                .lsp
                .servers
                .iter()
                .filter(|s| s.idle_since.is_some_and(|t| now.duration_since(t) >= limit))
                .map(|s| s.id)
                .collect();
            for id in stale {
                self.lsp.stop(id);
                self.lsp.servers.retain(|s| s.id != id);
            }
        }
    }

    // ------------------------------------------------------------------
    // Document sync

    /// The workspace root containing `path` (deepest), else its directory.
    fn root_for(&self, path: &Path) -> (PathBuf, String) {
        let canon = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
        self.workspace
            .roots()
            .iter()
            .filter(|r| path.starts_with(&r.path) || r.canonical.as_ref().is_some_and(|c| canon.starts_with(c)))
            .max_by_key(|r| r.path.components().count())
            .map(|r| (r.path.clone(), r.display_name()))
            .unwrap_or_else(|| {
                let dir = path.parent().unwrap_or(Path::new("/")).to_path_buf();
                let name = dir.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                (dir, name)
            })
    }

    pub(crate) fn lsp_sync(&mut self, attached: &[DocId], removed: &[DocId], edits: &[(DocId, Vec<DocEdit>)]) {
        for d in removed {
            self.lsp_close(*d);
        }
        for d in attached {
            self.lsp_close(*d);
            self.lsp_open(*d);
        }
        let mut changed = Vec::new();
        for (d, list) in edits {
            if let Some(path) = self.docs.doc(*d).and_then(|x| x.path()).map(path_key)
                && let Some(ds) = self.lsp.diagnostics.get_mut(&path)
            {
                for e in list {
                    map_diags(ds, &e.changes);
                }
            }
            let crate::lsp::LspState { docs, servers, .. } = &mut self.lsp;
            let Some(ld) = docs.get_mut(d) else { continue };
            let Some(server) = servers.iter().find(|s| s.id == ld.server) else { continue };
            if server.status != Status::Running || !server.docs.contains(d) {
                // Opened with the full text once the server is ready.
                continue;
            }
            let changes = match server.caps.sync {
                2 => content_changes(list, server.encoding()),
                1 => match self.docs.doc(*d) {
                    Some(doc) => vec![json!({ "text": doc.text() })],
                    None => continue,
                },
                _ => continue,
            };
            ld.version += 1;
            let params = json!({ "textDocument": { "uri": ld.uri, "version": ld.version }, "contentChanges": changes });
            let id = ld.server;
            self.lsp.notify(id, "textDocument/didChange", params);
            changed.push(*d);
        }
        if !changed.is_empty() {
            self.pull_after_change(&changed);
        }
    }

    fn lsp_open(&mut self, d: DocId) {
        if !self.settings.lsp.enabled {
            return;
        }
        let Some(doc) = self.docs.doc(d) else { return };
        if doc.large_file {
            return;
        }
        let Some(path) = doc.path().map(Path::to_path_buf) else { return };
        let lang = self.syntax.language(d);
        let defs = registry::candidates(lang.id, &user_defs(&self.settings.lsp), &self.settings.lsp.disabled);
        if defs.is_empty() {
            return;
        }
        let (root, root_name) = self.root_for(&path);
        let id = match defs.iter().find_map(|def| self.lsp.find(&def.name, &root)) {
            Some(id) => id,
            None => {
                let tried: Vec<(&yace_lsp::ServerDef, Result<PathBuf, String>)> =
                    defs.iter().map(|d| (d, registry::resolve(d, &root))).collect();
                let found = tried.iter().find_map(|(d, r)| r.as_ref().ok().map(|p| ((*d).clone(), p.clone())));
                let Some((def, program)) = found else {
                    // Suggest the server that applies here but is missing
                    // (not one that is unsuitable, e.g. for another
                    // TypeScript version).
                    let missing = tried
                        .iter()
                        .find(|(_, r)| {
                            r.as_ref().err().is_some_and(|e| e.contains("not found") || e.contains("no TypeScript 7"))
                        })
                        .map(|(d, _)| *d)
                        .unwrap_or(&defs[0]);
                    if self.lsp.hinted.insert(format!("{}\u{0}{}", lang.id, missing.name)) {
                        let msg = format!(
                            "No language server for {} files in {root_name}. For completion and navigation run: {}  (Esc to dismiss)",
                            lang.name, missing.install_hint
                        );
                        self.notify_sticky(Severity::Info, msg);
                    }
                    return;
                };
                match self.lsp.spawn(def.clone(), program, root, root_name) {
                    Ok(id) => id,
                    Err(e) => {
                        self.notify(Severity::Warning, format!("Could not start {}: {e}", def.name));
                        return;
                    }
                }
            }
        };
        let uri = uri::from_path(&path);
        self.lsp
            .docs
            .insert(d, LspDoc { server: id, uri, version: 0, language: lang.id, pulling: false, pull_again: false });
        // Diagnostics published before the file was opened.
        self.reposition_diags(&path, id);
        let Some(s) = self.lsp.server_mut(id) else { return };
        s.idle_since = None;
        match s.status {
            Status::Running => self.send_did_open(d),
            _ => {
                if !s.waiting.contains(&d) {
                    s.waiting.push(d);
                }
            }
        }
    }

    fn reposition_diags(&mut self, path: &Path, server: u32) {
        let enc = self.lsp.server(server).map(|s| s.encoding()).unwrap_or_default();
        let key = path_key(path);
        let Some(rope) = self.docs.find_by_path(path).and_then(|d| self.docs.doc(d)).map(|d| d.rope().clone()) else {
            return;
        };
        if let Some(ds) = self.lsp.diagnostics.get_mut(&key) {
            for x in ds.iter_mut() {
                x.chars = Some((from_lsp(&rope, x.range.start, enc), from_lsp(&rope, x.range.end, enc)));
            }
        }
    }

    fn send_did_open(&mut self, d: DocId) {
        let Some(ld) = self.lsp.docs.get(&d).cloned() else { return };
        let Some(doc) = self.docs.doc(d) else { return };
        let params = json!({ "textDocument": {
            "uri": ld.uri, "languageId": ld.language, "version": ld.version, "text": doc.text()
        }});
        let Some(s) = self.lsp.server_mut(ld.server) else { return };
        if !s.caps.open_close || s.docs.contains(&d) {
            s.docs.insert(d);
            return;
        }
        s.docs.insert(d);
        self.lsp.notify(ld.server, "textDocument/didOpen", params);
        self.pull_diagnostics(d);
    }

    /// Pull diagnostics for `d` (servers that don't push them). At most one
    /// request per document is in flight; changes meanwhile pull again.
    pub(crate) fn pull_diagnostics(&mut self, d: DocId) {
        let Some(ld) = self.lsp.docs.get(&d).cloned() else { return };
        let Some(s) = self.lsp.server(ld.server) else { return };
        if !s.caps.diagnostic_pull || s.status != Status::Running || !s.docs.contains(&d) {
            return;
        }
        if ld.pulling {
            if let Some(x) = self.lsp.docs.get_mut(&d) {
                x.pull_again = true;
            }
            return;
        }
        let ver = self.docs.doc(d).map(|x| x.version()).unwrap_or(0);
        let params = json!({ "textDocument": { "uri": ld.uri } });
        if self
            .lsp
            .request(ld.server, "textDocument/diagnostic", params, ReqKind::PullDiagnostics, Some(d), ver)
            .is_some()
            && let Some(x) = self.lsp.docs.get_mut(&d)
        {
            x.pulling = true;
            x.pull_again = false;
        }
    }

    /// After a change on a server whose diagnostics span files, every open
    /// document of that server may have new problems.
    fn pull_after_change(&mut self, changed: &[DocId]) {
        let mut todo: Vec<DocId> = changed.to_vec();
        for d in changed {
            let Some(server) = self.lsp.docs.get(d).map(|l| l.server) else { continue };
            if self.lsp.server(server).is_some_and(|s| s.caps.diagnostic_inter_file) {
                todo.extend(self.lsp.docs.iter().filter(|(_, l)| l.server == server).map(|(k, _)| *k));
            }
        }
        todo.sort_unstable();
        todo.dedup();
        for d in todo {
            self.pull_diagnostics(d);
        }
    }

    fn lsp_close(&mut self, d: DocId) {
        let Some(ld) = self.lsp.docs.remove(&d) else { return };
        let Some(s) = self.lsp.server_mut(ld.server) else { return };
        s.waiting.retain(|x| *x != d);
        let was_open = s.docs.remove(&d);
        if s.docs.is_empty() && s.waiting.is_empty() {
            s.idle_since = Some(Instant::now());
        }
        if was_open && s.caps.open_close {
            self.lsp.notify(ld.server, "textDocument/didClose", json!({ "textDocument": { "uri": ld.uri } }));
        }
    }

    pub(crate) fn lsp_did_save(&mut self, d: DocId) {
        let Some(ld) = self.lsp.docs.get(&d).cloned() else { return };
        let Some(s) = self.lsp.server(ld.server) else { return };
        if !s.caps.save {
            return;
        }
        let mut params = json!({ "textDocument": { "uri": ld.uri } });
        if s.caps.save_include_text
            && let Some(doc) = self.docs.doc(d)
        {
            params["text"] = Value::String(doc.text());
        }
        self.lsp.notify(ld.server, "textDocument/didSave", params);
    }

    // ------------------------------------------------------------------
    // Server messages

    pub(crate) fn on_lsp(&mut self, server: u32, msg: Incoming) {
        match msg {
            Incoming::Response { id, result, .. } => {
                if self.lsp.server(server).and_then(|s| s.init_id) == Some(id) {
                    self.on_initialized(server, result);
                } else if let Some(p) = self.lsp.pending.remove(&(server, id)) {
                    self.on_reply(server, p, result);
                }
            }
            Incoming::Request { id, method, params } => self.on_server_request(server, id, &method, params),
            Incoming::Diagnostics(pd) => self.on_diagnostics(server, pd),
            Incoming::Progress(p) => {
                if let Some(s) = self.lsp.server_mut(server) {
                    if p.kind == "end" {
                        s.progress.remove(&p.token);
                    } else {
                        let prev = s.progress.get(&p.token).map(|x| x.0.clone()).unwrap_or_default();
                        let text = p.title.or(p.message).unwrap_or(prev);
                        s.progress.insert(p.token, (text, p.percentage));
                    }
                }
            }
            Incoming::Log { message, .. } => {
                if let Some(s) = self.lsp.server_mut(server) {
                    if s.log.len() >= 500 {
                        s.log.pop_front();
                    }
                    s.log.push_back(message);
                }
            }
            Incoming::Show { level, message } => {
                let name = self.lsp.server(server).map(|s| s.def.name.clone()).unwrap_or_default();
                let sev = match level {
                    1 => Severity::Error,
                    2 => Severity::Warning,
                    _ => Severity::Info,
                };
                self.notify(sev, format!("{name}: {message}"));
            }
            Incoming::Exited => self.on_server_exit(server),
        }
    }

    fn on_initialized(&mut self, server: u32, result: Result<Reply, String>) {
        match result {
            Ok(Reply::Initialize(caps)) => {
                let Some(s) = self.lsp.server_mut(server) else { return };
                s.caps = *caps;
                s.status = Status::Running;
                s.init_id = None;
                let settings = s.def.settings.clone();
                let waiting = std::mem::take(&mut s.waiting);
                self.lsp.notify(server, "initialized", json!({}));
                if !settings.is_null() {
                    self.lsp.notify(server, "workspace/didChangeConfiguration", json!({ "settings": settings }));
                }
                for d in waiting {
                    self.send_did_open(d);
                }
            }
            Ok(_) | Err(_) => {
                let name = self.lsp.server(server).map(|s| s.def.name.clone()).unwrap_or_default();
                let why = result.err().unwrap_or_else(|| "unexpected answer".into());
                self.notify(Severity::Error, format!("{name} failed to initialize: {why}"));
                self.lsp.stop(server);
            }
        }
    }

    fn on_server_exit(&mut self, server: u32) {
        let Some(s) = self.lsp.server_mut(server) else { return };
        let Some(mut client) = s.client.take() else { return }; // stopped on purpose
        let tail: Vec<String> = {
            let q = client.stderr.lock().unwrap_or_else(|e| e.into_inner());
            q.iter().rev().take(2).rev().cloned().collect()
        };
        let code = client.try_exit_code().flatten();
        drop(client);
        let now = Instant::now();
        s.crashes.retain(|t| now.duration_since(*t) < RESTART_WINDOW);
        s.crashes.push(now);
        let name = s.def.name.clone();
        let count = s.crashes.len();
        let never_started = s.status == Status::Starting;
        let hint = s.def.install_hint.clone();
        let detail = match (code, tail.is_empty()) {
            (Some(c), true) => format!(" (exit code {c})"),
            (Some(c), false) => format!(" (exit code {c}: {})", tail.join(" ")),
            (None, false) => format!(" ({})", tail.join(" ")),
            (None, true) => String::new(),
        };
        if never_started {
            // It exited before initializing: not installed properly (e.g. a
            // rustup proxy without the component). Explain; don't loop.
            self.lsp.stop(server);
            let install = if hint.is_empty() { String::new() } else { format!(" Install it with: {hint}") };
            self.notify_sticky(Severity::Warning, format!("{name} could not start{detail}.{install}"));
        } else if count <= MAX_RESTARTS {
            self.notify(Severity::Warning, format!("{name} stopped unexpectedly{detail}. Restarting…"));
            if let Err(e) = self.lsp.restart(server) {
                self.notify(Severity::Error, format!("Could not restart {name}: {e}"));
            }
        } else {
            self.lsp.stop(server);
            self.notify(
                Severity::Error,
                format!(
                    "{name} crashed {count} times in 5 minutes and was stopped{detail}. Run \"Restart Language Server\" to try again."
                ),
            );
        }
    }

    fn on_server_request(&mut self, server: u32, id: Value, method: &str, params: Value) {
        let result = match method {
            "workspace/configuration" => {
                let settings = self.lsp.server(server).map(|s| s.def.settings.clone()).unwrap_or(Value::Null);
                let items = params.get("items").and_then(Value::as_array).cloned().unwrap_or_default();
                Value::Array(
                    items
                        .iter()
                        .map(|it| registry::config_section(&settings, it.get("section").and_then(Value::as_str)))
                        .collect(),
                )
            }
            "workspace/diagnostic/refresh" => {
                let docs: Vec<DocId> =
                    self.lsp.docs.iter().filter(|(_, l)| l.server == server).map(|(d, _)| *d).collect();
                for d in docs {
                    self.pull_diagnostics(d);
                }
                Value::Null
            }
            "workspace/workspaceFolders" => match self.lsp.server(server) {
                Some(s) => json!([{ "uri": uri::from_path(&s.root), "name": s.root_name }]),
                None => Value::Null,
            },
            "workspace/applyEdit" => {
                let we = yace_lsp::WorkspaceEdit::parse(params.get("edit").unwrap_or(&Value::Null));
                let ok = self.apply_workspace_edit(server, &we).is_ok();
                json!({ "applied": ok })
            }
            "window/showMessageRequest" => {
                if let Some(m) = params.get("message").and_then(Value::as_str) {
                    let name = self.lsp.server(server).map(|s| s.def.name.clone()).unwrap_or_default();
                    self.notify(Severity::Info, format!("{name}: {m}"));
                }
                Value::Null
            }
            "window/showDocument" => {
                if let Some(p) = params.get("uri").and_then(Value::as_str).and_then(uri::to_path) {
                    self.open_file(&p, false);
                }
                json!({ "success": true })
            }
            _ => Value::Null,
        };
        if let Some(c) = self.lsp.server(server).and_then(|s| s.client.as_ref()) {
            c.respond(id, result);
        }
    }

    fn on_diagnostics(&mut self, server: u32, pd: yace_lsp::PublishDiagnostics) {
        let Some(path) = uri::to_path(&pd.uri) else { return };
        let key = path_key(&path);
        let enc = self.lsp.server(server).map(|s| s.encoding()).unwrap_or_default();
        let rope = self.docs.find_by_path(&path).and_then(|d| self.docs.doc(d)).map(|d| d.rope().clone());
        let mut diags: Vec<Diag> = pd
            .diagnostics
            .into_iter()
            .map(|x| Diag {
                severity: x.severity.unwrap_or(1),
                code: x.code_text(),
                source: x.source,
                chars: rope.as_ref().map(|r| (from_lsp(r, x.range.start, enc), from_lsp(r, x.range.end, enc))),
                range: x.range,
                message: x.message,
                server,
            })
            .collect();
        diags.sort_by_key(|d| (d.chars.map(|c| c.0), d.range.start.line, d.range.start.character));
        if diags.is_empty() {
            self.lsp.diagnostics.remove(&key);
        } else {
            self.lsp.diagnostics.insert(key, diags);
        }
    }

    // ------------------------------------------------------------------
    // Requests

    /// Server, `textDocument` + `position` params and doc version for the
    /// active cursor. Explains why not when no server can answer.
    pub(crate) fn lsp_position_params(
        &mut self,
        cap: fn(&yace_lsp::ServerCaps) -> bool,
        what: &str,
    ) -> Option<(u32, DocId, Value, u64)> {
        let (g, v, d) = self.active()?;
        let head = self.layout.group(g)?.views.get(v)?.sel.primary().head;
        self.lsp_params_at(d, head, cap, what)
    }

    pub(crate) fn lsp_params_at(
        &mut self,
        d: DocId,
        pos: usize,
        cap: fn(&yace_lsp::ServerCaps) -> bool,
        what: &str,
    ) -> Option<(u32, DocId, Value, u64)> {
        // The server must have every edit before a request about the text.
        self.sync_documents();
        let Some(ld) = self.lsp.docs.get(&d).cloned() else {
            let lang = self.syntax.language(d).name;
            self.notify(Severity::Info, format!("{what}: no language server is running for {lang} files"));
            return None;
        };
        let s = self.lsp.server(ld.server)?;
        if s.status != Status::Running {
            let t = s.status_text();
            self.notify(Severity::Info, format!("{what}: {t}"));
            return None;
        }
        if !cap(&s.caps) {
            let name = s.def.name.clone();
            self.notify(Severity::Info, format!("{what} is not supported by {name}"));
            return None;
        }
        let enc = s.encoding();
        let doc = self.docs.doc(d)?;
        let params = json!({ "textDocument": { "uri": ld.uri }, "position": to_lsp(doc.rope(), pos, enc) });
        Some((ld.server, d, params, doc.version()))
    }

    pub(crate) fn lsp_goto(&mut self, method: &'static str, what: &'static str) {
        let cap: fn(&yace_lsp::ServerCaps) -> bool = match method {
            "textDocument/declaration" => |c| c.declaration,
            "textDocument/typeDefinition" => |c| c.type_definition,
            "textDocument/implementation" => |c| c.implementation,
            _ => |c| c.definition,
        };
        let Some((s, d, params, ver)) = self.lsp_position_params(cap, what) else { return };
        self.lsp.request(s, method, params, ReqKind::Goto { what }, Some(d), ver);
    }

    pub(crate) fn lsp_references(&mut self) {
        let Some((s, d, mut params, ver)) = self.lsp_position_params(|c| c.references, "Find references") else {
            return;
        };
        params["context"] = json!({ "includeDeclaration": true });
        self.lsp.request(s, "textDocument/references", params, ReqKind::References, Some(d), ver);
    }

    pub(crate) fn lsp_rename_prompt(&mut self) {
        let Some((g, v, d)) = self.active() else { return };
        if self.lsp_position_params(|c| c.rename, "Rename").is_none() {
            return;
        }
        let Some(rope) = self.docs.doc(d).map(|x| x.rope().clone()) else { return };
        let head = self.layout.group(g).map(|gr| gr.views[v].sel.primary().head).unwrap_or(0);
        let r = yace_buffer::movement::word_at(&rope, head);
        let word: String = rope.slice(r.from()..r.to()).into();
        let mut input = LineInput::new(word);
        input.all_selected = true;
        self.open_overlay(Overlay::Prompt(Prompt {
            title: "Rename Symbol".into(),
            hint: Some("Enter to rename, Escape to cancel".into()),
            input,
            action: PromptAction::RenameSymbol { doc: d, pos: head },
            path_completion: false,
            error: None,
        }));
    }

    pub(crate) fn lsp_rename(&mut self, d: DocId, pos: usize, new_name: &str) -> Result<(), String> {
        if new_name.is_empty() {
            return Err("Enter a name".into());
        }
        let Some((s, d, mut params, ver)) = self.lsp_params_at(d, pos, |c| c.rename, "Rename") else { return Ok(()) };
        params["newName"] = Value::String(new_name.to_string());
        self.lsp.request(s, "textDocument/rename", params, ReqKind::Rename, Some(d), ver);
        Ok(())
    }

    pub(crate) fn lsp_document_symbols(&mut self) {
        self.sync_documents();
        let Some((_, _, d)) = self.active() else { return };
        let Some(ld) = self.lsp.docs.get(&d).cloned() else { return };
        let Some(ver) = self.docs.doc(d).map(|x| x.version()) else { return };
        if self.doc_symbols.as_ref().is_some_and(|(dd, v, _, _)| *dd == d && *v == ver) {
            return;
        }
        if !self.lsp.server(ld.server).is_some_and(|s| s.caps.document_symbol) {
            return;
        }
        let params = json!({ "textDocument": { "uri": ld.uri } });
        self.lsp.request(ld.server, "textDocument/documentSymbol", params, ReqKind::DocSymbols, Some(d), ver);
    }

    /// Workspace symbols from every running server that offers them.
    pub(crate) fn lsp_workspace_symbols(&mut self, query: &str) {
        self.lsp.cancel_where(|k| matches!(k, ReqKind::WorkspaceSymbols { .. }));
        self.lsp.seq += 1;
        let seq = self.lsp.seq;
        self.workspace_symbols = (seq, Vec::new());
        let ids: Vec<u32> = self
            .lsp
            .servers
            .iter()
            .filter(|s| s.status == Status::Running && s.caps.workspace_symbol)
            .map(|s| s.id)
            .collect();
        for id in ids {
            self.lsp.request(
                id,
                "workspace/symbol",
                json!({ "query": query }),
                ReqKind::WorkspaceSymbols { seq },
                None,
                0,
            );
        }
    }

    pub(crate) fn lsp_code_actions(&mut self) {
        let Some((g, v, d)) = self.active() else { return };
        if self.lsp_position_params(|c| c.code_action, "Code actions").is_none() {
            return;
        }
        let Some(ld) = self.lsp.docs.get(&d).cloned() else { return };
        let enc = self.lsp.server(ld.server).map(|s| s.encoding()).unwrap_or_default();
        let Some(doc) = self.docs.doc(d) else { return };
        let rope = doc.rope();
        let r = self.layout.group(g).map(|gr| gr.views[v].sel.primary()).unwrap_or(yace_buffer::Range::point(0));
        let (from, to) = (r.from(), r.to());
        // Diagnostics touching the selection (or the cursor's line).
        let line = rope.char_to_line(r.head.min(rope.len_chars()));
        let path = doc.path().map(path_key);
        let diags: Vec<Value> = path
            .and_then(|p| self.lsp.diagnostics.get(&p))
            .into_iter()
            .flatten()
            .filter(|x| match x.chars {
                Some((a, b)) => (a <= to && b >= from) || rope.char_to_line(a.min(rope.len_chars())) == line,
                None => false,
            })
            .map(|x| {
                let (a, b) = x.chars.unwrap_or_default();
                json!({
                    "range": { "start": to_lsp(rope, a, enc), "end": to_lsp(rope, b, enc) },
                    "severity": x.severity, "message": x.message, "source": x.source, "code": x.code,
                })
            })
            .collect();
        let params = json!({
            "textDocument": { "uri": ld.uri },
            "range": { "start": to_lsp(rope, from, enc), "end": to_lsp(rope, to, enc) },
            "context": { "diagnostics": diags, "triggerKind": 1 },
        });
        let ver = doc.version();
        self.lsp.request(ld.server, "textDocument/codeAction", params, ReqKind::CodeActions, Some(d), ver);
    }

    /// Format the document (or the selection). Returns whether a request
    /// was sent.
    pub(crate) fn lsp_format(&mut self, d: DocId, selection_only: bool, then_save: bool) -> bool {
        self.sync_documents();
        let Some(ld) = self.lsp.docs.get(&d).cloned() else {
            if !then_save {
                let lang = self.syntax.language(d).name;
                self.notify(Severity::Info, format!("Formatting: no language server is running for {lang} files"));
            }
            return false;
        };
        let Some(s) = self.lsp.server(ld.server).filter(|s| s.status == Status::Running) else { return false };
        let supported = if selection_only { s.caps.range_formatting } else { s.caps.formatting };
        if !supported {
            if !then_save {
                let name = s.def.name.clone();
                self.notify(Severity::Info, format!("Formatting is not supported by {name}"));
            }
            return false;
        }
        let enc = s.encoding();
        let Some(doc) = self.docs.doc(d) else { return false };
        let (tab, spaces) = {
            let e = &self.settings_for(doc.path()).editor;
            (e.tab_size, e.insert_spaces)
        };
        let mut params = json!({
            "textDocument": { "uri": ld.uri },
            "options": { "tabSize": tab, "insertSpaces": spaces, "trimTrailingWhitespace": true },
        });
        let method = if selection_only {
            let r = self.active().and_then(|(g, v, _)| self.layout.group(g).map(|gr| gr.views[v].sel.primary()));
            let r = r.unwrap_or(yace_buffer::Range::point(0));
            params["range"] =
                json!({ "start": to_lsp(doc.rope(), r.from(), enc), "end": to_lsp(doc.rope(), r.to(), enc) });
            "textDocument/rangeFormatting"
        } else {
            "textDocument/formatting"
        };
        let ver = doc.version();
        self.lsp.request(ld.server, method, params, ReqKind::Format { then_save }, Some(d), ver).is_some()
    }

    /// Ctrl+S: format first when `editor.formatOnSave` is on and a server
    /// can (bounded wait), then save.
    pub(crate) fn save_with_format(&mut self, d: DocId) {
        let path = self.docs.doc(d).and_then(|x| x.path().map(Path::to_path_buf));
        let enabled = path.is_some() && self.settings_for(path.as_deref()).editor.format_on_save;
        if enabled && self.format_save.is_none() && self.lsp_format(d, false, true) {
            self.format_save = Some((d, Instant::now() + Duration::from_secs(2)));
            return;
        }
        self.save_doc(d, false);
    }

    // ------------------------------------------------------------------
    // Replies

    fn on_reply(&mut self, server: u32, p: Pending, result: Result<Reply, String>) {
        let current = p.doc.and_then(|d| self.docs.doc(d)).map(|x| x.version());
        let fresh = p.doc.is_none() || current == Some(p.version);
        if p.kind == ReqKind::PullDiagnostics
            && let Some(d) = p.doc
        {
            let again = self.lsp.docs.get_mut(&d).map(|x| {
                x.pulling = false;
                std::mem::take(&mut x.pull_again)
            });
            if let (true, Ok(Reply::PulledDiagnostics(Some(items)))) = (fresh, &result)
                && let Some(uri) = self.lsp.docs.get(&d).map(|x| x.uri.clone())
            {
                let pd = yace_lsp::PublishDiagnostics { uri, version: None, diagnostics: items.clone() };
                self.on_diagnostics(server, pd);
            }
            if again == Some(true) || !fresh {
                self.pull_diagnostics(d);
            }
            return;
        }
        let reply = match result {
            Ok(r) => r,
            Err(e) => {
                if let Some(what) = p.kind.user_facing() {
                    self.notify(Severity::Warning, format!("{what} failed: {e}"));
                }
                if let ReqKind::Format { then_save: true } = p.kind
                    && let Some(d) = p.doc
                {
                    self.format_save = None;
                    self.save_doc(d, false);
                }
                return;
            }
        };
        let enc = self.lsp.server(server).map(|s| s.encoding()).unwrap_or_default();
        match (p.kind, reply) {
            (ReqKind::Goto { what }, Reply::Locations(l)) => self.show_locations(l, enc, what, true),
            (ReqKind::References, Reply::Locations(l)) => self.show_locations(l, enc, "References", false),
            (ReqKind::Rename, Reply::WorkspaceEdit(we)) => {
                if we.edits.is_empty() {
                    self.notify(Severity::Info, "Rename: nothing to change");
                    return;
                }
                match self.apply_workspace_edit(server, &we) {
                    Ok((files, edits)) => self.notify(
                        Severity::Info,
                        format!(
                            "Renamed {edits} occurrence{} in {files} file{}",
                            if edits == 1 { "" } else { "s" },
                            if files == 1 { "" } else { "s" }
                        ),
                    ),
                    Err(problems) => {
                        self.notify(Severity::Error, format!("Rename incomplete: {}", problems.join("; ")))
                    }
                }
            }
            (ReqKind::DocSymbols, Reply::Symbols(syms)) => {
                if let Some(d) = p.doc {
                    self.doc_symbols = Some((d, p.version, syms, server));
                    self.refresh_symbol_quick_open();
                }
            }
            (ReqKind::WorkspaceSymbols { seq }, Reply::Symbols(syms)) => {
                if seq == self.workspace_symbols.0 {
                    self.workspace_symbols.1.extend(syms.into_iter().map(|s| (s, server)));
                    self.refresh_symbol_quick_open();
                }
            }
            (ReqKind::CodeActions, Reply::CodeActions(actions)) => {
                if fresh {
                    self.show_code_actions(server, actions);
                }
            }
            (ReqKind::CodeActionResolve, Reply::CodeActionResolved(a)) => self.run_code_action(server, *a),
            (ReqKind::Format { then_save }, Reply::Edits(edits)) => {
                if let Some(d) = p.doc {
                    if fresh && !edits.is_empty() {
                        self.apply_text_edits(d, &edits, enc);
                    }
                    if then_save {
                        self.format_save = None;
                        self.save_doc(d, false);
                    }
                }
            }
            (ReqKind::Completion { seq }, Reply::Completion { items, incomplete }) => {
                self.on_completion(server, seq, items, incomplete)
            }
            (ReqKind::CompletionResolve { seq, label, apply_edits }, Reply::CompletionResolved(item)) => {
                self.on_completion_resolved(server, seq, &label, apply_edits, *item)
            }
            (ReqKind::SignatureHelp { seq }, Reply::SignatureHelp(sh)) => self.on_signature_help(seq, sh),
            (ReqKind::Hover, Reply::Hover(text)) if fresh => self.show_hover(p.doc, text),
            _ => {}
        }
    }

    // ------------------------------------------------------------------
    // Edits

    /// Apply LSP text edits to an open document as one undoable change.
    pub(crate) fn apply_text_edits(&mut self, d: DocId, edits: &[yace_lsp::TextEdit], enc: Encoding) {
        let Some(rope) = self.docs.doc(d).map(|x| x.rope().clone()) else { return };
        let changes: Vec<Change> = edits
            .iter()
            .map(|e| {
                Change::replace(
                    from_lsp(&rope, e.range.start, enc),
                    from_lsp(&rope, e.range.end, enc),
                    e.new_text.clone(),
                )
            })
            .collect();
        self.apply_doc_changes(d, ChangeSet::new(changes, rope.len_chars()));
        self.on_document_edited();
    }

    /// Apply a workspace edit: open documents get undoable (unsaved) edits,
    /// other files are rewritten safely on disk (as Replace in Files).
    /// Returns (files, edits) or the problems met.
    pub(crate) fn apply_workspace_edit(
        &mut self,
        server: u32,
        we: &yace_lsp::WorkspaceEdit,
    ) -> Result<(usize, usize), Vec<String>> {
        let enc = self.lsp.server(server).map(|s| s.encoding()).unwrap_or_default();
        let mut by_uri: Vec<(String, Vec<yace_lsp::TextEdit>)> = Vec::new();
        for (u, _, edits) in &we.edits {
            match by_uri.iter_mut().find(|(x, _)| x == u) {
                Some((_, list)) => list.extend(edits.iter().cloned()),
                None => by_uri.push((u.clone(), edits.clone())),
            }
        }
        let mut problems = Vec::new();
        if we.unsupported > 0 {
            problems.push(format!("{} file create/rename/delete operation(s) skipped", we.unsupported));
        }
        let (mut files, mut count) = (0, 0);
        for (u, edits) in by_uri {
            if edits.is_empty() {
                continue;
            }
            let Some(path) = uri::to_path(&u) else {
                problems.push(format!("unsupported location {u}"));
                continue;
            };
            if let Some(d) = self.docs.find_by_path(&path) {
                self.apply_text_edits(d, &edits, enc);
            } else if let Err(e) = edit_file_on_disk(&path, &edits, enc) {
                problems.push(format!("{}: {e}", path.display()));
                continue;
            }
            files += 1;
            count += edits.len();
        }
        if problems.is_empty() { Ok((files, count)) } else { Err(problems) }
    }

    // ------------------------------------------------------------------
    // Pickers

    pub(crate) fn jump_to(&mut self, t: &NavTarget) {
        self.record_jump();
        self.open_file(&t.path, false);
        let Some((_, _, d)) = self.active() else { return };
        if self.docs.is_loading(d) {
            self.set_cursor_line_col(t.pos.line as usize, t.pos.character as usize);
        } else if let Some(rope) = self.docs.doc(d).map(|x| x.rope().clone()) {
            let c = from_lsp(&rope, t.pos, t.enc);
            self.set_active_selection(Selection::point(c));
            self.center_active_cursor();
        }
        self.focus_editor();
    }

    fn line_preview(&self, path: &Path, line: usize, cache: &mut HashMap<PathBuf, Vec<String>>) -> String {
        if let Some(d) = self.docs.find_by_path(path)
            && let Some(doc) = self.docs.doc(d)
        {
            let r = doc.rope();
            if line < r.len_lines() {
                return yace_buffer::text::line_without_ending(r, line).to_string().trim().to_string();
            }
            return String::new();
        }
        let lines = cache.entry(path.to_path_buf()).or_insert_with(|| {
            std::fs::read_to_string(path).map(|t| t.lines().map(String::from).collect()).unwrap_or_default()
        });
        lines.get(line).map(|l| l.trim().to_string()).unwrap_or_default()
    }

    fn rel_label(&self, path: &Path) -> String {
        match self.workspace.relative(path) {
            Some((root, rel)) if self.workspace.roots().len() > 1 => {
                format!("{} · {}", root.display_name(), rel.display())
            }
            Some((_, rel)) => rel.display().to_string(),
            None => path.display().to_string(),
        }
    }

    fn show_locations(&mut self, locs: Vec<yace_lsp::Location>, enc: Encoding, what: &str, jump_single: bool) {
        let targets: Vec<NavTarget> = locs
            .into_iter()
            .filter_map(|l| Some(NavTarget { path: uri::to_path(&l.uri)?, pos: l.range.start, enc }))
            .take(2000)
            .collect();
        if targets.is_empty() {
            self.notify(Severity::Info, format!("No {} found", what.to_lowercase()));
            return;
        }
        if targets.len() == 1 && jump_single {
            self.jump_to(&targets[0]);
            return;
        }
        let mut cache = HashMap::new();
        let items: Vec<PickerItem> = targets
            .iter()
            .enumerate()
            .map(|(i, t)| {
                let preview = self.line_preview(&t.path, t.pos.line as usize, &mut cache);
                PickerItem::new(preview, format!("{}:{}", self.rel_label(&t.path), t.pos.line + 1), "", i)
            })
            .collect();
        self.nav_targets = targets;
        let n = items.len();
        self.open_overlay(Overlay::Picker(Picker::new(format!("{what} ({n})"), items, PickerAction::Navigate)));
    }

    fn show_code_actions(&mut self, server: u32, mut actions: Vec<yace_lsp::CodeAction>) {
        if actions.is_empty() {
            self.notify(Severity::Info, "No code actions available");
            return;
        }
        actions.sort_by_key(|a| (a.disabled.is_some(), !a.preferred));
        let items = actions
            .iter()
            .enumerate()
            .map(|(i, a)| {
                let hint = match (&a.disabled, &a.kind) {
                    (Some(why), _) => format!("unavailable: {why}"),
                    (None, Some(k)) => k.clone(),
                    _ => String::new(),
                };
                PickerItem::new(a.title.clone(), "", hint, i)
            })
            .collect();
        self.code_actions = actions.into_iter().map(|a| (server, a)).collect();
        self.open_overlay(Overlay::Picker(Picker::new("Code Actions", items, PickerAction::CodeAction)));
    }

    pub(crate) fn pick_code_action(&mut self, i: usize) {
        let Some((server, a)) = self.code_actions.get(i).cloned() else { return };
        if let Some(why) = &a.disabled {
            self.notify(Severity::Info, format!("{}: {why}", a.title));
            return;
        }
        let resolve = self.lsp.server(server).is_some_and(|s| s.caps.code_action_resolve);
        if a.edit.is_none() && a.command.is_none() && resolve {
            self.lsp.request(server, "codeAction/resolve", a.raw.clone(), ReqKind::CodeActionResolve, None, 0);
            return;
        }
        self.run_code_action(server, a);
    }

    fn run_code_action(&mut self, server: u32, a: yace_lsp::CodeAction) {
        if let Some(we) = &a.edit
            && let Err(p) = self.apply_workspace_edit(server, we)
        {
            self.notify(Severity::Warning, format!("{}: {}", a.title, p.join("; ")));
        }
        if let Some(cmd) = &a.command {
            let (command, args) = (cmd.get("command").cloned().unwrap_or(Value::Null), cmd.get("arguments").cloned());
            let mut params = json!({ "command": command });
            if let Some(args) = args {
                params["arguments"] = args;
            }
            self.lsp.request(server, "workspace/executeCommand", params, ReqKind::ExecuteCommand, None, 0);
        }
    }

    /// Problems (`Ctrl+Shift+M`): every diagnostic, by file.
    pub(crate) fn show_problems(&mut self) {
        let mut targets = Vec::new();
        let mut items = Vec::new();
        let entries: Vec<(PathBuf, Vec<Diag>)> =
            self.lsp.diagnostics.iter().map(|(p, d)| (p.clone(), d.clone())).collect();
        for (path, diags) in entries {
            let rope = self.docs.find_by_path(&path).and_then(|d| self.docs.doc(d)).map(|d| d.rope().clone());
            for x in diags {
                let enc = self.lsp.server(x.server).map(|s| s.encoding()).unwrap_or_default();
                let (line, col) = x.line_col(rope.as_ref());
                let pos = match (&rope, x.chars) {
                    (Some(r), Some((a, _))) => to_lsp(r, a, enc),
                    _ => x.range.start,
                };
                let mut hint = x.severity_name().to_string();
                if let Some(s) = &x.source {
                    hint = format!("{hint} · {s}");
                }
                if let Some(c) = &x.code {
                    hint = format!("{hint}({c})");
                }
                let first = x.message.lines().next().unwrap_or("").to_string();
                items.push(PickerItem::new(
                    first,
                    format!("{}:{}:{}", self.rel_label(&path), line + 1, col + 1),
                    hint,
                    targets.len(),
                ));
                targets.push(NavTarget { path: path.clone(), pos, enc });
            }
        }
        if items.is_empty() {
            self.notify(Severity::Info, "No problems have been detected in the workspace");
            return;
        }
        self.nav_targets = targets;
        let (e, w) = self.lsp.counts();
        let mut p = Picker::new(format!("Problems — {e} errors, {w} warnings"), items, PickerAction::Navigate);
        p.empty_text = "No matching problems".into();
        self.open_overlay(Overlay::Picker(p));
    }

    /// F8 / Shift+F8: next / previous problem (this file, then the next
    /// file with problems).
    pub(crate) fn next_problem(&mut self, forward: bool) {
        let Some((g, v, d)) = self.active() else {
            self.show_problems();
            return;
        };
        let head = self.layout.group(g).map(|gr| gr.views[v].sel.primary().head).unwrap_or(0);
        let path = self.docs.doc(d).and_then(|x| x.path()).map(path_key);
        let here: Vec<usize> = path
            .as_ref()
            .and_then(|p| self.lsp.diagnostics.get(p))
            .map(|ds| ds.iter().filter_map(|x| x.chars.map(|c| c.0)).collect())
            .unwrap_or_default();
        let target = if forward {
            here.iter().copied().find(|a| *a > head)
        } else {
            here.iter().rev().copied().find(|a| *a < head)
        };
        if let Some(a) = target {
            self.record_jump();
            self.set_active_selection(Selection::point(a));
            self.center_active_cursor();
            return;
        }
        // Wrap to the next/previous file with problems (or around this one).
        let keys: Vec<PathBuf> = self.lsp.diagnostics.keys().cloned().collect();
        if keys.is_empty() {
            self.notify(Severity::Info, "No problems");
            return;
        }
        let i = path.as_ref().and_then(|p| keys.iter().position(|k| k == p));
        let n = keys.len();
        let next = match (i, forward) {
            (Some(i), true) => (i + 1) % n,
            (Some(i), false) => (i + n - 1) % n,
            (None, true) => 0,
            (None, false) => n - 1,
        };
        let Some(diags) = self.lsp.diagnostics.get(&keys[next]) else { return };
        let pick = if forward { diags.first() } else { diags.last() };
        let Some(x) = pick.cloned() else { return };
        let enc = self.lsp.server(x.server).map(|s| s.encoding()).unwrap_or_default();
        let pos = match (self.docs.find_by_path(&keys[next]).and_then(|d| self.docs.doc(d)), x.chars) {
            (Some(doc), Some((a, _))) => to_lsp(doc.rope(), a, enc),
            _ => x.range.start,
        };
        self.jump_to(&NavTarget { path: keys[next].clone(), pos, enc });
    }

    /// The diagnostic under the cursor of the active view (status bar).
    pub fn diagnostic_at_cursor(&self) -> Option<&Diag> {
        let (g, v, d) = self.active()?;
        let head = self.layout.group(g)?.views.get(v)?.sel.primary().head;
        let path = path_key(self.docs.doc(d)?.path()?);
        self.lsp
            .diagnostics
            .get(&path)?
            .iter()
            .filter(|x| x.chars.is_some_and(|(a, b)| head >= a && head <= b.max(a + 1)))
            .min_by_key(|x| x.severity)
    }

    /// Diagnostics of an open document, by char range.
    pub fn doc_diagnostics(&self, d: DocId) -> &[Diag] {
        self.docs
            .doc(d)
            .and_then(|x| x.path())
            .and_then(|p| self.lsp.diagnostics.get(&path_key(p)))
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }

    /// Status bar text for the active document's server.
    pub fn lsp_status(&self) -> Option<String> {
        let (_, _, d) = self.active()?;
        let ld = self.lsp.docs.get(&d)?;
        Some(self.lsp.server(ld.server)?.status_text())
    }

    // ------------------------------------------------------------------
    // Commands

    fn active_server(&self) -> Option<u32> {
        let (_, _, d) = self.active()?;
        self.lsp.docs.get(&d).map(|l| l.server)
    }

    pub(crate) fn lsp_restart_cmd(&mut self) {
        let ids: Vec<u32> = match self.active_server() {
            Some(id) => vec![id],
            None => self.lsp.servers.iter().map(|s| s.id).collect(),
        };
        if ids.is_empty() {
            self.notify(Severity::Info, "No language server to restart");
            return;
        }
        for id in ids {
            if let Some(s) = self.lsp.server_mut(id) {
                s.crashes.clear();
            }
            if let Err(e) = self.lsp.restart(id) {
                self.notify(Severity::Error, e);
            }
        }
    }

    pub(crate) fn lsp_stop_cmd(&mut self) {
        let ids: Vec<u32> = match self.active_server() {
            Some(id) => vec![id],
            None => self.lsp.servers.iter().map(|s| s.id).collect(),
        };
        for id in ids {
            self.lsp.stop(id);
        }
    }

    /// "Show Running Language Servers": name, root, pid and memory —
    /// reported separately from YACE's own (spec §13.4, §23.2).
    pub(crate) fn lsp_show_running(&mut self) {
        let mut lines = Vec::new();
        for s in &self.lsp.servers {
            let pid = s.client.as_ref().map(|c| c.pid());
            let mem = pid.map(|p| format!("{:.1} MiB", crate::lsp::LspState::rss_kib(p) as f64 / 1024.0));
            lines.push(format!(
                "{}{}  —  {}  —  {}  —  pid {}  —  {}  —  {} open",
                s.def.name,
                if s.def.needs_node { " (Node.js)" } else { "" },
                s.root_name,
                match s.status {
                    Status::Starting => "starting",
                    Status::Running => "running",
                    Status::Stopped => "stopped",
                },
                pid.map(|p| p.to_string()).unwrap_or_else(|| "-".into()),
                mem.unwrap_or_else(|| "-".into()),
                s.docs.len(),
            ));
        }
        if lines.is_empty() {
            lines.push("No language servers are running. They start when you open a file of their language.".into());
        }
        lines.push(String::new());
        lines.push(format!("YACE itself: {:.1} MiB", crate::metrics::rss_kib().unwrap_or(0) as f64 / 1024.0));
        self.open_overlay(Overlay::Info { title: "Running Language Servers".into(), lines });
    }

    /// "Show Language Server Output": stderr and log of the active server,
    /// in a new untitled editor.
    pub(crate) fn lsp_show_output(&mut self) {
        let Some(id) = self.active_server().or_else(|| self.lsp.servers.first().map(|s| s.id)) else {
            self.notify(Severity::Info, "No language server has run yet");
            return;
        };
        let Some(s) = self.lsp.server(id) else { return };
        let mut text = format!("# {} — {}\n\n## stderr\n", s.def.name, s.root.display());
        if let Some(c) = &s.client {
            let q = c.stderr.lock().unwrap_or_else(|e| e.into_inner());
            for l in q.iter() {
                text.push_str(l);
                text.push('\n');
            }
        }
        text.push_str("\n## log\n");
        for l in &s.log {
            text.push_str(l);
            text.push('\n');
        }
        self.new_untitled_with(&text);
    }
}

fn edit_file_on_disk(path: &Path, edits: &[yace_lsp::TextEdit], enc: Encoding) -> Result<(), String> {
    let loaded = yace_buffer::io::load(path).map_err(|e| e.to_string())?;
    let mut rope = loaded.rope;
    let changes: Vec<Change> = edits
        .iter()
        .map(|e| {
            Change::replace(from_lsp(&rope, e.range.start, enc), from_lsp(&rope, e.range.end, enc), e.new_text.clone())
        })
        .collect();
    ChangeSet::new(changes, rope.len_chars()).apply(&mut rope);
    yace_buffer::io::save(
        path,
        &rope,
        yace_buffer::io::SaveOptions { expected: Some(&loaded.snapshot), force: false, bom: loaded.has_bom },
    )
    .map(|_| ())
    .map_err(|e| e.to_string())
}
