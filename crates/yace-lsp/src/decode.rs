//! Typed decoding of server messages, done on the reader thread so large
//! responses (completion lists, symbol lists) never cost UI time.

use serde::Deserialize;
use serde_json::Value;

use crate::types::*;

#[derive(Debug, Clone)]
pub enum Reply {
    Initialize(Box<ServerCaps>),
    Completion {
        items: Vec<CompletionItem>,
        incomplete: bool,
    },
    CompletionResolved(Box<CompletionItem>),
    Hover(String),
    SignatureHelp(Option<SignatureHelp>),
    Locations(Vec<Location>),
    Symbols(Vec<Symbol>),
    CodeActions(Vec<CodeAction>),
    CodeActionResolved(Box<CodeAction>),
    Edits(Vec<TextEdit>),
    WorkspaceEdit(WorkspaceEdit),
    /// `textDocument/diagnostic`: `None` when unchanged since the last pull.
    PulledDiagnostics(Option<Vec<Diagnostic>>),
    Other(Value),
}

#[derive(Debug, Clone)]
pub enum Incoming {
    Response {
        id: u64,
        method: String,
        result: Result<Reply, String>,
    },
    /// A request from the server; it must be answered with `respond`.
    Request {
        id: Value,
        method: String,
        params: Value,
    },
    Diagnostics(PublishDiagnostics),
    Progress(Progress),
    /// `window/logMessage` (1 error … 4 log).
    Log {
        level: u8,
        message: String,
    },
    /// `window/showMessage`.
    Show {
        level: u8,
        message: String,
    },
    /// The server's stdout closed (it exited or crashed).
    Exited,
}

fn locations(v: &Value) -> Vec<Location> {
    let one = |x: &Value| -> Option<Location> {
        if let Some(uri) = x.get("targetUri").and_then(Value::as_str) {
            let range = x.get("targetSelectionRange").or_else(|| x.get("targetRange"))?;
            return Some(Location { uri: uri.into(), range: Range::deserialize(range).ok()? });
        }
        Location::deserialize(x).ok()
    };
    match v {
        Value::Array(a) => a.iter().filter_map(one).collect(),
        Value::Object(_) => one(v).into_iter().collect(),
        _ => Vec::new(),
    }
}

fn symbols(v: &Value) -> Vec<Symbol> {
    fn doc_symbol(x: &Value, depth: usize, container: &str, out: &mut Vec<Symbol>) {
        let name = x.get("name").and_then(Value::as_str).unwrap_or("").to_string();
        let range = x
            .get("selectionRange")
            .or_else(|| x.get("range"))
            .and_then(|r| Range::deserialize(r).ok())
            .unwrap_or_default();
        out.push(Symbol {
            name: name.clone(),
            detail: x.get("detail").and_then(Value::as_str).unwrap_or("").to_string(),
            kind: x.get("kind").and_then(Value::as_u64).unwrap_or(0) as u32,
            uri: None,
            range,
            container: container.to_string(),
            depth,
        });
        if let Some(children) = x.get("children").and_then(Value::as_array) {
            for c in children {
                doc_symbol(c, depth + 1, &name, out);
            }
        }
    }
    let mut out = Vec::new();
    for x in v.as_array().into_iter().flatten() {
        if let Some(loc) = x.get("location") {
            out.push(Symbol {
                name: x.get("name").and_then(Value::as_str).unwrap_or("").to_string(),
                detail: String::new(),
                kind: x.get("kind").and_then(Value::as_u64).unwrap_or(0) as u32,
                uri: loc.get("uri").and_then(Value::as_str).map(String::from),
                range: loc.get("range").and_then(|r| Range::deserialize(r).ok()).unwrap_or_default(),
                container: x.get("containerName").and_then(Value::as_str).unwrap_or("").to_string(),
                depth: 0,
            });
        } else {
            doc_symbol(x, 0, "", &mut out);
        }
    }
    out
}

pub fn code_action(x: &Value) -> Option<CodeAction> {
    let title = x.get("title")?.as_str()?.to_string();
    // A bare `Command` has a string `command`.
    if x.get("command").is_some_and(Value::is_string) {
        return Some(CodeAction {
            title,
            kind: None,
            preferred: false,
            disabled: None,
            edit: None,
            command: Some(x.clone()),
            raw: x.clone(),
        });
    }
    Some(CodeAction {
        title,
        kind: x.get("kind").and_then(Value::as_str).map(String::from),
        preferred: x.get("isPreferred").and_then(Value::as_bool).unwrap_or(false),
        disabled: x.pointer("/disabled/reason").and_then(Value::as_str).map(String::from),
        edit: x.get("edit").map(WorkspaceEdit::parse),
        command: x.get("command").cloned().filter(|c| !c.is_null()),
        raw: x.clone(),
    })
}

fn completion_item(x: &Value) -> Option<CompletionItem> {
    let mut it = CompletionItem::deserialize(x).ok()?;
    it.raw = x.clone();
    Some(it)
}

pub fn decode_reply(method: &str, v: Value) -> Reply {
    match method {
        "initialize" => Reply::Initialize(Box::new(ServerCaps::from_initialize(&v))),
        "textDocument/completion" => {
            let (items, incomplete) = match &v {
                Value::Array(a) => (a.iter().filter_map(completion_item).collect(), false),
                Value::Object(o) => (
                    o.get("items")
                        .and_then(Value::as_array)
                        .map(|a| a.iter().filter_map(completion_item).collect())
                        .unwrap_or_default(),
                    o.get("isIncomplete").and_then(Value::as_bool).unwrap_or(false),
                ),
                _ => (Vec::new(), false),
            };
            Reply::Completion { items, incomplete }
        }
        "completionItem/resolve" => match completion_item(&v) {
            Some(i) => Reply::CompletionResolved(Box::new(i)),
            None => Reply::Other(v),
        },
        "textDocument/hover" => Reply::Hover(v.get("contents").map(markup_text).unwrap_or_default()),
        "textDocument/signatureHelp" => Reply::SignatureHelp(SignatureHelp::deserialize(&v).ok()),
        "textDocument/definition"
        | "textDocument/declaration"
        | "textDocument/typeDefinition"
        | "textDocument/implementation"
        | "textDocument/references" => Reply::Locations(locations(&v)),
        "textDocument/documentSymbol" | "workspace/symbol" => Reply::Symbols(symbols(&v)),
        "textDocument/codeAction" => {
            Reply::CodeActions(v.as_array().map(|a| a.iter().filter_map(code_action).collect()).unwrap_or_default())
        }
        "codeAction/resolve" => match code_action(&v) {
            Some(a) => Reply::CodeActionResolved(Box::new(a)),
            None => Reply::Other(v),
        },
        "textDocument/formatting" | "textDocument/rangeFormatting" => Reply::Edits(
            v.as_array().map(|a| a.iter().filter_map(|e| TextEdit::deserialize(e).ok()).collect()).unwrap_or_default(),
        ),
        "textDocument/rename" => Reply::WorkspaceEdit(WorkspaceEdit::parse(&v)),
        "textDocument/diagnostic" => Reply::PulledDiagnostics(match v.get("kind").and_then(Value::as_str) {
            Some("unchanged") => None,
            _ => Some(
                v.get("items")
                    .and_then(Value::as_array)
                    .map(|a| a.iter().filter_map(|d| Diagnostic::deserialize(d).ok()).collect())
                    .unwrap_or_default(),
            ),
        }),
        _ => Reply::Other(v),
    }
}

pub fn decode_notification(method: &str, params: Value) -> Option<Incoming> {
    match method {
        "textDocument/publishDiagnostics" => PublishDiagnostics::deserialize(&params).ok().map(Incoming::Diagnostics),
        "$/progress" => {
            let token = match params.get("token") {
                Some(Value::String(s)) => s.clone(),
                Some(v) => v.to_string(),
                None => String::new(),
            };
            let val = params.get("value")?;
            Some(Incoming::Progress(Progress {
                token,
                kind: val.get("kind").and_then(Value::as_str).unwrap_or("").to_string(),
                title: val.get("title").and_then(Value::as_str).map(String::from),
                message: val.get("message").and_then(Value::as_str).map(String::from),
                percentage: val.get("percentage").and_then(Value::as_u64).map(|p| p as u32),
            }))
        }
        "window/logMessage" | "window/showMessage" => {
            let level = params.get("type").and_then(Value::as_u64).unwrap_or(4) as u8;
            let message = params.get("message").and_then(Value::as_str).unwrap_or("").to_string();
            Some(if method == "window/logMessage" {
                Incoming::Log { level, message }
            } else {
                Incoming::Show { level, message }
            })
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn completion_shapes() {
        let list = json!({"isIncomplete": true, "items": [
            {"label": "useState", "kind": 3, "textEdit": {"range": {"start": {"line":0,"character":0},"end":{"line":0,"character":3}}, "newText": "useState"},
             "additionalTextEdits": [{"range": {"start":{"line":0,"character":0},"end":{"line":0,"character":0}}, "newText": "import x;\n"}]},
            {"label": "b", "textEdit": {"newText": "b", "insert": {"start":{"line":0,"character":0},"end":{"line":0,"character":1}}, "replace": {"start":{"line":0,"character":0},"end":{"line":0,"character":2}}}},
            {"label": "c", "documentation": {"kind": "markdown", "value": "doc"}}
        ]});
        let Reply::Completion { items, incomplete } = decode_reply("textDocument/completion", list) else { panic!() };
        assert!(incomplete);
        assert_eq!(items.len(), 3);
        assert_eq!(items[0].additional_text_edits.as_ref().unwrap().len(), 1);
        assert_eq!(items[1].text_edit.as_ref().unwrap().text_and_range().1.end.character, 1);
        assert_eq!(items[2].raw["label"], "c");
        let Reply::Completion { items, .. } = decode_reply("textDocument/completion", json!([{"label": "x"}])) else {
            panic!()
        };
        assert_eq!(items[0].label, "x");
    }

    #[test]
    fn locations_symbols_actions() {
        let link = json!([{"targetUri": "file:///a", "targetRange": {"start":{"line":1,"character":0},"end":{"line":3,"character":0}},
                           "targetSelectionRange": {"start":{"line":2,"character":4},"end":{"line":2,"character":8}}}]);
        let Reply::Locations(l) = decode_reply("textDocument/definition", link) else { panic!() };
        assert_eq!(l[0].range.start.line, 2);
        let Reply::Locations(l) = decode_reply("textDocument/definition", Value::Null) else { panic!() };
        assert!(l.is_empty());
        let syms = json!([{"name": "A", "kind": 5, "range": {"start":{"line":0,"character":0},"end":{"line":9,"character":0}},
                           "selectionRange": {"start":{"line":0,"character":6},"end":{"line":0,"character":7}},
                           "children": [{"name": "m", "kind": 6, "range": {"start":{"line":1,"character":0},"end":{"line":2,"character":0}},
                                         "selectionRange": {"start":{"line":1,"character":2},"end":{"line":1,"character":3}}}]}]);
        let Reply::Symbols(s) = decode_reply("textDocument/documentSymbol", syms) else { panic!() };
        assert_eq!(s.len(), 2);
        assert_eq!((s[1].depth, s[1].container.as_str()), (1, "A"));
        let acts = json!([{"title": "Fix", "kind": "quickfix", "isPreferred": true, "edit": {"changes": {"file:///a": [{"range": {"start":{"line":0,"character":0},"end":{"line":0,"character":1}}, "newText": "y"}]}}},
                          {"title": "Run", "command": "x.run", "arguments": [1]}]);
        let Reply::CodeActions(a) = decode_reply("textDocument/codeAction", acts) else { panic!() };
        assert!(a[0].preferred && a[0].edit.as_ref().unwrap().edits.len() == 1);
        assert_eq!(a[1].command.as_ref().unwrap()["command"], "x.run");
    }

    #[test]
    fn hover_and_capabilities() {
        let Reply::Hover(h) = decode_reply(
            "textDocument/hover",
            json!({"contents": [{"language": "ts", "value": "let x: number"}, "Doc"]}),
        ) else {
            panic!()
        };
        assert_eq!(h, "```ts\nlet x: number\n```\n\nDoc");
        let caps = ServerCaps::from_initialize(&json!({"capabilities": {
            "textDocumentSync": {"openClose": true, "change": 2, "save": {"includeText": false}},
            "completionProvider": {"triggerCharacters": ["."], "resolveProvider": true},
            "hoverProvider": true, "renameProvider": {"prepareProvider": true}, "definitionProvider": false,
            "positionEncoding": "utf-8"}}));
        assert_eq!(caps.sync, 2);
        assert!(caps.save && caps.completion_resolve && caps.hover && caps.rename && caps.prepare_rename);
        assert!(!caps.definition);
        assert_eq!(caps.completion_triggers, vec!["."]);
        assert_eq!(caps.encoding, crate::position::Encoding::Utf8);
        let caps = ServerCaps::from_initialize(&json!({"capabilities": {
            "diagnosticProvider": {"identifier": "typescript", "interFileDependencies": true, "workspaceDiagnostics": false}}}));
        assert!(caps.diagnostic_pull && caps.diagnostic_inter_file);
        let full = json!({"kind": "full", "items": [{"range": {"start":{"line":1,"character":6},"end":{"line":1,"character":7}},
                          "severity": 1, "code": 2345, "source": "ts", "message": "not assignable"}]});
        let Reply::PulledDiagnostics(Some(d)) = decode_reply("textDocument/diagnostic", full) else { panic!() };
        assert_eq!(d[0].code_text().as_deref(), Some("2345"));
        let Reply::PulledDiagnostics(None) =
            decode_reply("textDocument/diagnostic", json!({"kind": "unchanged", "resultId": "1"}))
        else {
            panic!()
        };
    }
}
