//! The subset of LSP types YACE uses. Deserialisation is lenient: unknown
//! fields are ignored, optional ones default, and the several shapes some
//! fields may take (bool or options object, string or markup) are accepted.

use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Position {
    pub line: u32,
    pub character: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Range {
    pub start: Position,
    pub end: Position,
}

#[derive(Debug, Clone, PartialEq, Default, Deserialize)]
pub struct Location {
    pub uri: String,
    #[serde(default)]
    pub range: Range,
}

#[derive(Debug, Clone, PartialEq, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TextEdit {
    pub range: Range,
    pub new_text: String,
}

/// A string, `{value}` markup / marked string, or a list of those, as text.
pub fn markup_text(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Array(a) => a.iter().map(markup_text).filter(|s| !s.is_empty()).collect::<Vec<_>>().join("\n\n"),
        Value::Object(o) => {
            let text = o.get("value").and_then(Value::as_str).unwrap_or("");
            match o.get("language").and_then(Value::as_str) {
                Some(lang) => format!("```{lang}\n{text}\n```"),
                None => text.to_string(),
            }
        }
        _ => String::new(),
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct ServerCaps {
    /// 0 none, 1 full, 2 incremental.
    pub sync: u8,
    pub open_close: bool,
    pub save: bool,
    pub save_include_text: bool,
    pub completion: bool,
    pub completion_triggers: Vec<String>,
    pub completion_resolve: bool,
    pub hover: bool,
    pub signature_help: bool,
    pub signature_triggers: Vec<String>,
    pub definition: bool,
    pub declaration: bool,
    pub type_definition: bool,
    pub implementation: bool,
    pub references: bool,
    pub document_symbol: bool,
    pub workspace_symbol: bool,
    pub code_action: bool,
    pub code_action_resolve: bool,
    pub formatting: bool,
    pub range_formatting: bool,
    pub rename: bool,
    pub prepare_rename: bool,
    pub execute_command: bool,
    /// Pull diagnostics (`textDocument/diagnostic`, LSP 3.17).
    pub diagnostic_pull: bool,
    /// Diagnostics of one file depend on others (re-pull open files).
    pub diagnostic_inter_file: bool,
    pub encoding: crate::position::Encoding,
}

fn strings(v: Option<&Value>) -> Vec<String> {
    v.and_then(Value::as_array)
        .map(|a| a.iter().filter_map(|x| x.as_str().map(String::from)).collect())
        .unwrap_or_default()
}

fn has(v: &Value, k: &str) -> bool {
    v.get(k).is_some_and(|x| !matches!(x, Value::Null | Value::Bool(false)))
}

impl ServerCaps {
    pub fn from_initialize(result: &Value) -> ServerCaps {
        let c = result.get("capabilities").cloned().unwrap_or(Value::Null);
        let mut caps = ServerCaps { open_close: true, ..Default::default() };
        match c.get("textDocumentSync") {
            Some(Value::Number(n)) => {
                caps.sync = n.as_u64().unwrap_or(0) as u8;
                caps.open_close = caps.sync > 0;
            }
            Some(o @ Value::Object(_)) => {
                caps.sync = o.get("change").and_then(Value::as_u64).unwrap_or(0) as u8;
                caps.open_close = o.get("openClose").and_then(Value::as_bool).unwrap_or(false);
                caps.save = has(o, "save");
                caps.save_include_text =
                    o.get("save").and_then(|s| s.get("includeText")).and_then(Value::as_bool).unwrap_or(false);
            }
            _ => caps.open_close = false,
        }
        if let Some(cp) = c.get("completionProvider").filter(|v| v.is_object()) {
            caps.completion = true;
            caps.completion_triggers = strings(cp.get("triggerCharacters"));
            caps.completion_resolve = cp.get("resolveProvider").and_then(Value::as_bool).unwrap_or(false);
        }
        caps.hover = has(&c, "hoverProvider");
        if let Some(sp) = c.get("signatureHelpProvider").filter(|v| v.is_object()) {
            caps.signature_help = true;
            caps.signature_triggers = strings(sp.get("triggerCharacters"));
            caps.signature_triggers.extend(strings(sp.get("retriggerCharacters")));
        }
        caps.definition = has(&c, "definitionProvider");
        caps.declaration = has(&c, "declarationProvider");
        caps.type_definition = has(&c, "typeDefinitionProvider");
        caps.implementation = has(&c, "implementationProvider");
        caps.references = has(&c, "referencesProvider");
        caps.document_symbol = has(&c, "documentSymbolProvider");
        caps.workspace_symbol = has(&c, "workspaceSymbolProvider");
        caps.code_action = has(&c, "codeActionProvider");
        caps.code_action_resolve = c
            .get("codeActionProvider")
            .and_then(|v| v.get("resolveProvider"))
            .and_then(Value::as_bool)
            .unwrap_or(false);
        caps.formatting = has(&c, "documentFormattingProvider");
        caps.range_formatting = has(&c, "documentRangeFormattingProvider");
        caps.rename = has(&c, "renameProvider");
        caps.prepare_rename =
            c.get("renameProvider").and_then(|v| v.get("prepareProvider")).and_then(Value::as_bool).unwrap_or(false);
        caps.execute_command = has(&c, "executeCommandProvider");
        caps.diagnostic_pull = has(&c, "diagnosticProvider");
        caps.diagnostic_inter_file = c
            .get("diagnosticProvider")
            .and_then(|v| v.get("interFileDependencies"))
            .and_then(Value::as_bool)
            .unwrap_or(false);
        caps.encoding =
            crate::position::Encoding::from_name(c.get("positionEncoding").and_then(Value::as_str).unwrap_or("utf-16"));
        caps
    }
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(untagged)]
pub enum NumberOrString {
    Number(i64),
    String(String),
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Diagnostic {
    pub range: Range,
    /// 1 error, 2 warning, 3 information, 4 hint.
    #[serde(default)]
    pub severity: Option<u8>,
    #[serde(default)]
    pub code: Option<NumberOrString>,
    #[serde(default)]
    pub source: Option<String>,
    pub message: String,
    /// Kept for code actions: servers (ESLint, Biome) read it back.
    #[serde(default)]
    pub data: Option<serde_json::Value>,
}

impl Diagnostic {
    pub fn code_text(&self) -> Option<String> {
        self.code.as_ref().map(|c| match c {
            NumberOrString::Number(n) => n.to_string(),
            NumberOrString::String(s) => s.clone(),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct PublishDiagnostics {
    pub uri: String,
    #[serde(default)]
    pub version: Option<i64>,
    pub diagnostics: Vec<Diagnostic>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(untagged)]
pub enum CompletionEdit {
    Edit(TextEdit),
    InsertReplace {
        #[serde(rename = "newText")]
        new_text: String,
        insert: Range,
        #[allow(dead_code)]
        replace: Range,
    },
}

impl CompletionEdit {
    pub fn text_and_range(&self) -> (&str, Range) {
        match self {
            CompletionEdit::Edit(e) => (&e.new_text, e.range),
            CompletionEdit::InsertReplace { new_text, insert, .. } => (new_text, *insert),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CompletionItem {
    pub label: String,
    #[serde(default)]
    pub kind: Option<u32>,
    #[serde(default)]
    pub detail: Option<String>,
    #[serde(default)]
    pub documentation: Option<Value>,
    #[serde(default)]
    pub sort_text: Option<String>,
    #[serde(default)]
    pub filter_text: Option<String>,
    #[serde(default)]
    pub insert_text: Option<String>,
    /// 1 plain text, 2 snippet.
    #[serde(default)]
    pub insert_text_format: Option<u8>,
    #[serde(default)]
    pub text_edit: Option<CompletionEdit>,
    #[serde(default)]
    pub additional_text_edits: Option<Vec<TextEdit>>,
    #[serde(default)]
    pub preselect: Option<bool>,
    #[serde(default)]
    pub command: Option<Value>,
    /// The item as received (compact JSON text), for
    /// `completionItem/resolve`. Lists can have tens of thousands of items,
    /// so this is text, not a `Value` tree.
    #[serde(skip)]
    pub raw: Box<str>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ParameterInformation {
    /// A string, or `[start, end]` offsets into the signature label.
    pub label: Value,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SignatureInformation {
    pub label: String,
    #[serde(default)]
    pub documentation: Option<Value>,
    #[serde(default)]
    pub parameters: Vec<ParameterInformation>,
    #[serde(default)]
    pub active_parameter: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SignatureHelp {
    pub signatures: Vec<SignatureInformation>,
    #[serde(default)]
    pub active_signature: Option<u32>,
    #[serde(default)]
    pub active_parameter: Option<u32>,
}

/// A symbol, flattened from `DocumentSymbol[]` (with depth) or
/// `SymbolInformation[]` / `WorkspaceSymbol[]`.
#[derive(Debug, Clone, PartialEq)]
pub struct Symbol {
    pub name: String,
    pub detail: String,
    pub kind: u32,
    pub uri: Option<String>,
    /// Where to jump (selection range when available).
    pub range: Range,
    pub container: String,
    pub depth: usize,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct WorkspaceEdit {
    /// (uri, document version if given, edits) in order.
    pub edits: Vec<(String, Option<i64>, Vec<TextEdit>)>,
    /// Resource operations we do not apply (create/rename/delete).
    pub unsupported: usize,
}

impl WorkspaceEdit {
    pub fn parse(v: &Value) -> WorkspaceEdit {
        let mut out = WorkspaceEdit::default();
        if let Some(dc) = v.get("documentChanges").and_then(Value::as_array) {
            for c in dc {
                if c.get("kind").is_some() {
                    out.unsupported += 1;
                    continue;
                }
                let uri = c.pointer("/textDocument/uri").and_then(Value::as_str).unwrap_or("").to_string();
                let version = c.pointer("/textDocument/version").and_then(Value::as_i64);
                let edits = c
                    .get("edits")
                    .and_then(Value::as_array)
                    .map(|a| a.iter().filter_map(|e| TextEdit::deserialize(e).ok()).collect())
                    .unwrap_or_default();
                out.edits.push((uri, version, edits));
            }
        } else if let Some(ch) = v.get("changes").and_then(Value::as_object) {
            for (uri, edits) in ch {
                let edits = edits
                    .as_array()
                    .map(|a| a.iter().filter_map(|e| TextEdit::deserialize(e).ok()).collect())
                    .unwrap_or_default();
                out.edits.push((uri.clone(), None, edits));
            }
        }
        out
    }

    pub fn file_count(&self) -> usize {
        let mut uris: Vec<&str> = self.edits.iter().map(|(u, _, _)| u.as_str()).collect();
        uris.sort_unstable();
        uris.dedup();
        uris.len()
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct CodeAction {
    pub title: String,
    pub kind: Option<String>,
    pub preferred: bool,
    pub disabled: Option<String>,
    pub edit: Option<WorkspaceEdit>,
    /// `{command, arguments}` to execute after the edit.
    pub command: Option<Value>,
    /// The action as received, for `codeAction/resolve`.
    pub raw: Value,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Progress {
    pub token: String,
    /// "begin", "report" or "end".
    pub kind: String,
    pub title: Option<String>,
    pub message: Option<String>,
    pub percentage: Option<u32>,
}
