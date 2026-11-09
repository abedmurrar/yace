//! Completion, hover and signature help popups (spec §13.1).
//!
//! Completion is non-modal: typing keeps going to the editor and re-filters
//! the list client-side (no round trip per key). Requests start on the
//! server's trigger characters and, after a short pause, while typing an
//! identifier — so terminals that cannot send Ctrl+Space get completion too.

use std::time::{Duration, Instant};

use serde_json::{Value, json};
use yace_buffer::{Change, ChangeSet, EditKind, Range, Rope, Selection, ops::Edit};
use yace_lsp::{CompletionItem, Encoding, SignatureHelp, from_lsp, to_lsp};

use super::App;
use crate::{editor::DocId, lsp::ReqKind, lsp::Status};

pub const COMPLETION_KINDS: [&str; 26] = [
    "",
    "text",
    "method",
    "fn",
    "ctor",
    "field",
    "var",
    "class",
    "interface",
    "module",
    "prop",
    "unit",
    "value",
    "enum",
    "keyword",
    "snippet",
    "color",
    "file",
    "ref",
    "folder",
    "member",
    "const",
    "struct",
    "event",
    "op",
    "type",
];

#[derive(Debug, Clone)]
pub struct Completion {
    pub doc: DocId,
    pub server: u32,
    pub enc: Encoding,
    /// Start of the word being completed.
    pub anchor: usize,
    pub items: Vec<CompletionItem>,
    /// Indices into `items`, best first (highlights are computed for the
    /// visible rows only, when drawing).
    pub shown: Vec<usize>,
    pub selected: usize,
    pub scroll: usize,
    pub incomplete: bool,
    pub seq: u64,
    /// The text the current `shown` list was filtered with.
    pub filtered_for: Option<String>,
    /// Items asked for their documentation (`completionItem/resolve`).
    pub resolve_asked: std::collections::HashSet<usize>,
}

pub fn completion_kind(k: u32) -> &'static str {
    COMPLETION_KINDS.get(k as usize).copied().unwrap_or("")
}

impl Completion {
    pub fn current(&self) -> Option<&CompletionItem> {
        self.shown.get(self.selected).map(|i| &self.items[*i])
    }
}

#[derive(Debug, Clone)]
pub struct HoverPopup {
    pub doc: DocId,
    pub lines: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct SignaturePopup {
    pub doc: DocId,
    pub label: String,
    /// Char range of the active parameter within `label`.
    pub active: Option<(usize, usize)>,
}

fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_' || c == '$'
}

fn word_start(rope: &Rope, pos: usize) -> usize {
    let mut s = pos.min(rope.len_chars());
    while s > 0 && is_word_char(rope.char(s - 1)) {
        s -= 1;
    }
    s
}

/// Expand an LSP snippet to plain text. Returns the text and the char range
/// to select afterwards (the first placeholder, or `$0`, or the end).
pub fn expand_snippet(s: &str) -> (String, usize, usize) {
    fn parse(chars: &[char], i: &mut usize, out: &mut String, stops: &mut Vec<(u32, usize, usize)>, depth: usize) {
        while *i < chars.len() {
            let c = chars[*i];
            if depth > 0 && c == '}' {
                return;
            }
            if c == '\\' && *i + 1 < chars.len() {
                out.push(chars[*i + 1]);
                *i += 2;
                continue;
            }
            if c == '$' && *i + 1 < chars.len() {
                let n = chars[*i + 1];
                if n.is_ascii_digit() {
                    let mut j = *i + 1;
                    let mut num = 0u32;
                    while j < chars.len() && chars[j].is_ascii_digit() {
                        num = num * 10 + chars[j].to_digit(10).unwrap_or(0);
                        j += 1;
                    }
                    let at = out.chars().count();
                    stops.push((num, at, at));
                    *i = j;
                    continue;
                }
                if n == '{' {
                    let mut j = *i + 2;
                    let mut num = 0u32;
                    let digits = j;
                    while j < chars.len() && chars[j].is_ascii_digit() {
                        num = num * 10 + chars[j].to_digit(10).unwrap_or(0);
                        j += 1;
                    }
                    let start = out.chars().count();
                    if j > digits && j < chars.len() && chars[j] == ':' {
                        *i = j + 1;
                        parse(chars, i, out, stops, depth + 1);
                        *i += 1; // '}'
                        stops.push((num, start, out.chars().count()));
                        continue;
                    }
                    if j > digits && j < chars.len() && chars[j] == '|' {
                        // ${1|a,b,c|}: the first choice.
                        let mut k = j + 1;
                        while k < chars.len() && chars[k] != ',' && chars[k] != '|' {
                            out.push(chars[k]);
                            k += 1;
                        }
                        while k < chars.len() && chars[k] != '}' {
                            k += 1;
                        }
                        *i = k + 1;
                        stops.push((num, start, out.chars().count()));
                        continue;
                    }
                    if j > digits && j < chars.len() && chars[j] == '}' {
                        *i = j + 1;
                        stops.push((num, start, start));
                        continue;
                    }
                    // ${VAR} / ${VAR:default}: variables resolve to their default.
                    let mut k = *i + 2;
                    while k < chars.len() && chars[k] != '}' && chars[k] != ':' {
                        k += 1;
                    }
                    *i = k;
                    if k < chars.len() && chars[k] == ':' {
                        *i = k + 1;
                        parse(chars, i, out, stops, depth + 1);
                    }
                    *i += 1;
                    continue;
                }
            }
            out.push(c);
            *i += 1;
        }
    }
    let chars: Vec<char> = s.chars().collect();
    let mut out = String::new();
    let mut stops = Vec::new();
    let mut i = 0;
    parse(&chars, &mut i, &mut out, &mut stops, 0);
    let end = out.chars().count();
    let first = stops.iter().filter(|(n, _, _)| *n > 0).min_by_key(|(n, _, _)| *n).copied();
    let zero = stops.iter().find(|(n, _, _)| *n == 0).copied();
    let (a, b) = first.or(zero).map(|(_, a, b)| (a, b)).unwrap_or((end, end));
    (out, a, b)
}

impl App {
    pub(crate) fn close_popups(&mut self) {
        self.completion = None;
        self.hover = None;
        self.signature = None;
        self.completion_deadline = None;
    }

    // ------------------------------------------------------------------
    // Typing hooks

    /// After a character was typed in the active editor.
    pub(crate) fn on_typed(&mut self, c: char) {
        self.hover = None;
        let Some((g, v, d)) = self.active() else { return };
        let Some(ld) = self.lsp.docs.get(&d).cloned() else { return };
        let Some(s) = self.lsp.server(ld.server).filter(|s| s.status == Status::Running) else { return };
        let (comp_triggers, sig_triggers) = (s.caps.completion_triggers.clone(), s.caps.signature_triggers.clone());
        let has_completion = s.caps.completion;
        let has_sig = s.caps.signature_help;
        let Some(rope) = self.docs.doc(d).map(|x| x.rope().clone()) else { return };
        let head = self.layout.group(g).map(|gr| gr.views[v].sel.primary().head).unwrap_or(0);
        let e = self.settings_for(self.docs.doc(d).and_then(|x| x.path())).editor.clone();
        // Signature help: open on its trigger characters, close on ')'.
        if has_sig && sig_triggers.iter().any(|t| t.ends_with(c)) {
            self.request_signature_help();
        } else if c == ')' {
            self.signature = None;
        }
        if !has_completion {
            return;
        }
        let line_start = rope.line_to_char(rope.char_to_line(head));
        let before: String = rope.slice(line_start..head).into();
        let triggered = e.suggest_on_trigger_characters && comp_triggers.iter().any(|t| before.ends_with(t.as_str()));
        if triggered {
            self.request_completion(Some(c), true);
            return;
        }
        if is_word_char(c) {
            if self.completion.as_ref().is_some_and(|x| x.doc == d) {
                self.refilter_completion();
                if self.completion.as_ref().is_some_and(|x| x.incomplete) {
                    self.completion_deadline = Some(Instant::now() + Duration::from_millis(e.quick_suggestions_delay));
                }
            } else if e.quick_suggestions {
                self.completion_deadline = Some(Instant::now() + Duration::from_millis(e.quick_suggestions_delay));
            }
        } else {
            self.completion = None;
            self.completion_deadline = None;
        }
    }

    /// After Backspace/Delete in the active editor.
    pub(crate) fn on_deleted(&mut self) {
        self.hover = None;
        if self.completion.is_some() {
            self.refilter_completion();
        }
    }

    // ------------------------------------------------------------------
    // Completion

    /// Ask for completions at the cursor (`trigger` = typed trigger char).
    pub(crate) fn request_completion(&mut self, trigger: Option<char>, explicit: bool) {
        // The server must have the keystroke before the request.
        self.sync_documents();
        let Some((g, v, d)) = self.active() else { return };
        let Some(ld) = self.lsp.docs.get(&d).cloned() else {
            if explicit && trigger.is_none() {
                let lang = self.syntax.language(d).name;
                self.notify(
                    super::Severity::Info,
                    format!("Completion: no language server is running for {lang} files"),
                );
            }
            return;
        };
        let Some(s) = self.lsp.server(ld.server).filter(|s| s.status == Status::Running && s.caps.completion) else {
            return;
        };
        let enc = s.encoding();
        let Some(rope) = self.docs.doc(d).map(|x| x.rope().clone()) else { return };
        let head = self.layout.group(g).map(|gr| gr.views[v].sel.primary().head).unwrap_or(0);
        let anchor = word_start(&rope, head);
        // Quick suggestions need at least one identifier character typed.
        if trigger.is_none() && !explicit && anchor == head {
            return;
        }
        self.lsp.cancel_where(|k| matches!(k, ReqKind::Completion { .. }));
        self.lsp.seq += 1;
        let seq = self.lsp.seq;
        let context = match trigger {
            Some(c) => json!({ "triggerKind": 2, "triggerCharacter": c.to_string() }),
            None => json!({ "triggerKind": 1 }),
        };
        let params = json!({
            "textDocument": { "uri": ld.uri },
            "position": to_lsp(&rope, head, enc),
            "context": context,
        });
        let ver = self.docs.doc(d).map(|x| x.version()).unwrap_or(0);
        self.lsp.request(ld.server, "textDocument/completion", params, ReqKind::Completion { seq }, Some(d), ver);
        self.completion_req = Some((seq, d, anchor));
    }

    pub(crate) fn on_completion(&mut self, server: u32, seq: u64, items: Vec<CompletionItem>, incomplete: bool) {
        let Some((want, d, anchor)) = self.completion_req else { return };
        if want != seq || items.is_empty() {
            if want == seq && self.completion.as_ref().is_some_and(|c| c.seq != seq) {
                self.completion = None;
            }
            return;
        }
        // Still typing the same word in the same document?
        let Some((g, v, ad)) = self.active() else { return };
        let head = self.layout.group(g).map(|gr| gr.views[v].sel.primary().head).unwrap_or(0);
        if ad != d || head < anchor {
            return;
        }
        let enc = self.lsp.server(server).map(|s| s.encoding()).unwrap_or_default();
        self.completion = Some(Completion {
            doc: d,
            server,
            enc,
            anchor,
            items,
            shown: Vec::new(),
            selected: 0,
            scroll: 0,
            incomplete,
            seq,
            filtered_for: None,
            resolve_asked: Default::default(),
        });
        self.refilter_completion();
    }

    /// Filter and rank the list for the text typed since the anchor.
    pub(crate) fn refilter_completion(&mut self) {
        let Some((g, v, d)) = self.active() else {
            self.completion = None;
            return;
        };
        let head = self.layout.group(g).map(|gr| gr.views[v].sel.primary().head).unwrap_or(0);
        let Some(rope) = self.docs.doc(d).map(|x| x.rope().clone()) else { return };
        let Some(c) = self.completion.as_mut() else { return };
        if c.doc != d || head < c.anchor || rope.char_to_line(head) != rope.char_to_line(c.anchor.min(rope.len_chars()))
        {
            self.completion = None;
            return;
        }
        let typed: String = rope.slice(c.anchor..head).into();
        if typed.chars().any(|ch| !is_word_char(ch)) {
            self.completion = None;
            return;
        }
        // Typing narrows: only what matched the shorter prefix can match.
        let candidates: Vec<usize> = match &c.filtered_for {
            Some(prev) if !prev.is_empty() && typed.starts_with(prev.as_str()) => c.shown.clone(),
            _ => (0..c.items.len()).collect(),
        };
        let shown = rank_completions(&c.items, &candidates, &typed);
        c.filtered_for = Some(typed);
        c.shown = shown;
        c.selected = c.shown.iter().position(|i| c.items[*i].preselect == Some(true)).unwrap_or(0);
        c.scroll = 0;
        if c.shown.is_empty() {
            self.completion = None;
        }
        self.resolve_selected_completion();
    }

    /// Re-filter as typing does (tests and benchmarks).
    pub fn refilter_for_tests(&mut self) {
        self.refilter_completion();
    }

    pub(crate) fn completion_move(&mut self, delta: isize) {
        let Some(c) = self.completion.as_mut() else { return };
        let n = c.shown.len() as isize;
        if n == 0 {
            return;
        }
        c.selected = (c.selected as isize + delta).rem_euclid(n) as usize;
        self.resolve_selected_completion();
    }

    /// The details panel shows the selected item's documentation: ask the
    /// server for it once (`completionItem/resolve`) when the list didn't
    /// include it.
    pub(crate) fn resolve_selected_completion(&mut self) {
        if !self.suggest_details {
            return;
        }
        let Some(c) = self.completion.as_mut() else { return };
        let Some(&i) = c.shown.get(c.selected) else { return };
        let item = &c.items[i];
        if item.documentation.is_some() || item.raw.is_empty() || !c.resolve_asked.insert(i) {
            return;
        }
        if !self.lsp.server(c.server).is_some_and(|s| s.caps.completion_resolve) {
            return;
        }
        let (server, seq, label, d) = (c.server, c.seq, item.label.clone(), c.doc);
        let params = serde_json::from_str(&item.raw).unwrap_or(Value::Null);
        // Only the newest selection matters.
        self.lsp.cancel_where(|k| matches!(k, ReqKind::CompletionResolve { apply_edits: false, .. }));
        let ver = self.docs.doc(d).map(|x| x.version()).unwrap_or(0);
        let kind = ReqKind::CompletionResolve { seq, label, apply_edits: false };
        self.lsp.request(server, "completionItem/resolve", params, kind, Some(d), ver);
    }

    /// `Ctrl+Space` while the list is open: show or hide the details panel.
    pub(crate) fn toggle_suggestion_details(&mut self) {
        self.suggest_details = !self.suggest_details;
        self.resolve_selected_completion();
    }

    /// Accept the selected item: its text (snippets expanded) replaces the
    /// typed word; extra edits (auto-imports) come with it, or after a
    /// `completionItem/resolve` when the server sends them lazily.
    pub(crate) fn accept_completion(&mut self) {
        let Some(c) = self.completion.take() else { return };
        self.completion_deadline = None;
        let Some(item) = c.current().cloned() else { return };
        let Some((g, v, d)) = self.active() else { return };
        if d != c.doc {
            return;
        }
        let Some(rope) = self.docs.doc(d).map(|x| x.rope().clone()) else { return };
        let head = self.layout.group(g).map(|gr| gr.views[v].sel.primary().head).unwrap_or(0);
        let (raw_text, start) = match &item.text_edit {
            Some(te) => {
                let (t, r) = te.text_and_range();
                let s = from_lsp(&rope, r.start, c.enc).min(head);
                (t.to_string(), s)
            }
            None => (item.insert_text.clone().unwrap_or_else(|| item.label.clone()), c.anchor.min(head)),
        };
        let (text, sel_a, sel_b) = if item.insert_text_format == Some(2) {
            expand_snippet(&raw_text)
        } else {
            let n = raw_text.chars().count();
            (raw_text, n, n)
        };
        let mut changes = vec![Change::replace(start, head, text)];
        for e in item.additional_text_edits.iter().flatten() {
            let (a, b) = (from_lsp(&rope, e.range.start, c.enc), from_lsp(&rope, e.range.end, c.enc));
            if b <= start || a >= head {
                changes.push(Change::replace(a, b, e.new_text.clone()));
            }
        }
        let cs = ChangeSet::new(changes, rope.len_chars());
        let base = cs.map_pos(start, yace_buffer::Assoc::Before);
        let selection = Selection::single(Range::new(base + sel_a, base + sel_b));
        self.apply_edit(Edit { changes: cs, selection, kind: EditKind::Other });
        if let Some(doc) = self.docs.doc_mut(d) {
            doc.seal_history();
        }
        // Lazily resolved auto-imports.
        let resolve = self.lsp.server(c.server).is_some_and(|s| s.caps.completion_resolve);
        if item.additional_text_edits.is_none() && resolve && !item.raw.is_empty() {
            let ver = self.docs.doc(d).map(|x| x.version()).unwrap_or(0);
            self.lsp.request(
                c.server,
                "completionItem/resolve",
                serde_json::from_str(&item.raw).unwrap_or(Value::Null),
                ReqKind::CompletionResolve { seq: c.seq, label: item.label.clone(), apply_edits: true },
                Some(d),
                ver,
            );
        }
        self.run_completion_command(item.command.as_ref());
    }

    fn run_completion_command(&mut self, cmd: Option<&Value>) {
        match cmd.and_then(|c| c.get("command")).and_then(Value::as_str) {
            Some("editor.action.triggerParameterHints") => self.request_signature_help(),
            Some("editor.action.triggerSuggest") => self.request_completion(None, true),
            _ => {}
        }
    }

    pub(crate) fn on_completion_resolved(
        &mut self,
        server: u32,
        seq: u64,
        label: &str,
        apply_edits: bool,
        item: CompletionItem,
    ) {
        if !apply_edits {
            // Documentation for the details panel.
            if let Some(c) = self.completion.as_mut().filter(|c| c.seq == seq)
                && let Some(it) = c.items.iter_mut().find(|it| it.label == label)
            {
                if item.documentation.is_some() {
                    it.documentation = item.documentation;
                }
                if item.detail.is_some() {
                    it.detail = item.detail;
                }
                self.needs_render = true;
            }
            return;
        }
        let Some((_, _, d)) = self.active() else { return };
        let Some(edits) = item.additional_text_edits.filter(|e| !e.is_empty()) else { return };
        let enc = self.lsp.server(server).map(|s| s.encoding()).unwrap_or_default();
        // The extra edits refer to text before the accepted word (imports),
        // which typing after it has not moved.
        self.apply_text_edits(d, &edits, enc);
        if let Some(doc) = self.docs.doc_mut(d) {
            doc.seal_history();
        }
    }

    // ------------------------------------------------------------------
    // Signature help and hover

    pub(crate) fn request_signature_help(&mut self) {
        let Some((s, d, params, ver)) = self.lsp_position_params(|c| c.signature_help, "Parameter hints") else {
            return;
        };
        self.lsp.seq += 1;
        let seq = self.lsp.seq;
        self.signature_seq = seq;
        self.lsp.request(s, "textDocument/signatureHelp", params, ReqKind::SignatureHelp { seq }, Some(d), ver);
    }

    pub(crate) fn on_signature_help(&mut self, seq: u64, sh: Option<SignatureHelp>) {
        if seq != self.signature_seq {
            return;
        }
        let Some((_, _, d)) = self.active() else { return };
        let Some(sh) = sh.filter(|s| !s.signatures.is_empty()) else {
            self.signature = None;
            return;
        };
        let si = sh.active_signature.unwrap_or(0) as usize;
        let sig = &sh.signatures[si.min(sh.signatures.len() - 1)];
        let ap = sig.active_parameter.or(sh.active_parameter).unwrap_or(0) as usize;
        let active = sig.parameters.get(ap).and_then(|p| match &p.label {
            Value::String(s) => sig.label.find(s.as_str()).map(|b| {
                let a = sig.label[..b].chars().count();
                (a, a + s.chars().count())
            }),
            Value::Array(a) if a.len() == 2 => {
                let (x, y) = (a[0].as_u64()? as usize, a[1].as_u64()? as usize);
                // Offsets are UTF-16 code units into the label.
                let mut units = 0usize;
                let mut start = None;
                let mut end = None;
                for (ci, ch) in sig.label.chars().enumerate() {
                    if units == x {
                        start = Some(ci);
                    }
                    if units == y {
                        end = Some(ci);
                    }
                    units += ch.len_utf16();
                }
                let n = sig.label.chars().count();
                Some((start.unwrap_or(n), end.unwrap_or(n)))
            }
            _ => None,
        });
        self.signature = Some(SignaturePopup { doc: d, label: sig.label.clone(), active });
    }

    pub(crate) fn lsp_hover(&mut self) {
        let Some((s, d, params, ver)) = self.lsp_position_params(|c| c.hover, "Hover") else { return };
        self.lsp.request(s, "textDocument/hover", params, ReqKind::Hover, Some(d), ver);
    }

    pub(crate) fn show_hover(&mut self, doc: Option<DocId>, text: String) {
        let Some(d) = doc else { return };
        let lines = markdown_to_lines(&text);
        if lines.is_empty() {
            self.notify(super::Severity::Info, "No hover information");
            return;
        }
        self.hover = Some(HoverPopup { doc: d, lines });
    }
}

/// What the details panel beside the completion list shows for an item:
/// its `detail` (signature) and its documentation as plain lines.
pub fn completion_details(item: &CompletionItem) -> Vec<String> {
    let mut out = Vec::new();
    if let Some(d) = item.detail.as_deref().filter(|d| !d.trim().is_empty()) {
        out.extend(d.lines().map(clean));
    }
    if let Some(doc) = &item.documentation {
        let lines = markdown_to_lines(&yace_lsp::markup_text(doc));
        if !lines.is_empty() && !out.is_empty() {
            out.push(String::new());
        }
        out.extend(lines);
    }
    out
}

/// Markdown → plain lines for the hover popup and the completion details
/// (fences, emphasis, link targets and heading marks dropped; code kept as
/// is). Tabs become spaces and other control characters are removed: the
/// terminal buffer drops them silently, which glued words together.
pub fn markdown_to_lines(md: &str) -> Vec<String> {
    let md = md.replace("\r\n", "\n").replace('\r', "\n");
    let mut out: Vec<String> = Vec::new();
    let mut in_code = false;
    // A blank row separates a code block from the text around it.
    let mut gap_before_next = false;
    for line in md.lines() {
        if line.trim_start().starts_with("```") {
            in_code = !in_code;
            if in_code {
                if out.last().is_some_and(|l| !l.is_empty()) {
                    out.push(String::new());
                }
            } else {
                gap_before_next = true;
            }
            continue;
        }
        let t = if in_code { clean(line) } else { prose(line) };
        if gap_before_next && !in_code && !t.is_empty() {
            out.push(String::new());
        }
        gap_before_next = false;
        if !in_code && (t == "---" || t == "***" || t == "___") {
            out.push(String::new());
            continue;
        }
        out.push(t);
    }
    out.dedup_by(|a, b| a.is_empty() && b.is_empty());
    while out.last().is_some_and(|l| l.is_empty()) {
        out.pop();
    }
    while out.first().is_some_and(|l| l.is_empty()) {
        out.remove(0);
    }
    out
}

/// Tabs to spaces (4 columns), other control characters removed.
fn clean(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    for c in line.chars() {
        if c == '\t' {
            out.push_str(&" ".repeat(4 - out.chars().count() % 4));
        } else if !c.is_control() {
            out.push(c);
        }
    }
    out.trim_end().to_string()
}

/// One line of Markdown text as plain text: `# ` heading marks, `**`,
/// `__`, backticks, `*emphasis*` and `\` escapes dropped, `[text](url)`
/// shown as its text.
fn prose(line: &str) -> String {
    let line = clean(line);
    let trimmed = line.trim_start();
    let hashes = trimmed.chars().take_while(|&c| c == '#').count();
    let line = if (1..=6).contains(&hashes) && trimmed[hashes..].starts_with(' ') {
        trimmed[hashes + 1..].to_string()
    } else {
        line
    };
    let line = line.replace("**", "").replace("__", "").replace('`', "");
    let chars: Vec<char> = line.chars().collect();
    let mut out = String::with_capacity(line.len());
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        let next = chars.get(i + 1).copied();
        if c == '\\' && next.is_some_and(|n| n.is_ascii_punctuation()) {
            out.push(next.unwrap_or_default());
            i += 2;
            continue;
        }
        // [text](url) → text
        if c == '['
            && let Some(close) = chars[i + 1..].iter().position(|&x| x == ']').map(|k| i + 1 + k)
            && chars.get(close + 1) == Some(&'(')
            && let Some(end) = chars[close + 2..].iter().position(|&x| x == ')').map(|k| close + 2 + k)
        {
            out.extend(&chars[i + 1..close]);
            i = end + 1;
            continue;
        }
        // *emphasis*: a `*` that opens (no space after) and has a closing
        // `*` (no space before) later on the line. List bullets (`* x`)
        // have a space after and stay.
        if c == '*'
            && next.is_some_and(|n| !n.is_whitespace() && n != '*')
            && let Some(close) = chars[i + 1..].iter().position(|&x| x == '*').map(|k| i + 1 + k)
            && !chars[close - 1].is_whitespace()
        {
            out.extend(&chars[i + 1..close]);
            i = close + 1;
            continue;
        }
        out.push(c);
        i += 1;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snippets() {
        assert_eq!(expand_snippet("beta(${1:x})$0"), ("beta(x)".into(), 5, 6));
        assert_eq!(expand_snippet("fn $1() {\n\t$0\n}"), ("fn () {\n\t\n}".into(), 3, 3));
        assert_eq!(expand_snippet("${1|let,const|} a"), ("let a".into(), 0, 3));
        assert_eq!(expand_snippet("plain"), ("plain".into(), 5, 5));
        assert_eq!(expand_snippet("\\$x ${TM_FILENAME:file}"), ("$x file".into(), 7, 7));
        assert_eq!(expand_snippet("${1:outer ${2:inner}}"), ("outer inner".into(), 0, 11));
    }

    #[test]
    fn markdown_lines() {
        assert_eq!(markdown_to_lines("```ts\nlet x: number\n```\n\n**Doc** `x`"), vec!["let x: number", "", "Doc x"]);
        // TypeScript 7's hover: the code block is followed directly by text.
        assert_eq!(
            markdown_to_lines(
                "```typescript\nfunction add(a: number): number\n```\nAdds.\n\n- one\n\n*@param* `a` — the first"
            ),
            vec!["function add(a: number): number", "", "Adds.", "", "- one", "", "@param a — the first"]
        );
        // Lone \r, \r\n and tabs are line breaks and spaces, not glue.
        assert_eq!(markdown_to_lines("one\rtwo\r\nthree\tfour"), vec!["one", "two", "three   four"]);
        assert_eq!(markdown_to_lines("```\n\tindented\n```"), vec!["    indented"]);
        // Links show their text; headings, escapes and bullets.
        assert_eq!(
            markdown_to_lines("# Examples\n[MDN Reference](https://x)\n* item\nsnake\\_case 2 * 3"),
            vec!["Examples", "MDN Reference", "* item", "snake_case 2 * 3"]
        );
        // Text before a code block gets a gap too; no double gaps.
        assert_eq!(markdown_to_lines("Text\n```\ncode\n```\n\nMore"), vec!["Text", "", "code", "", "More"]);
        // MarkedString[] (joined by the decoder) renders the same way.
        let joined = yace_lsp::markup_text(&serde_json::json!([{"language": "ts", "value": "let x"}, "Doc"]));
        assert_eq!(markdown_to_lines(&joined), vec!["let x", "", "Doc"]);
    }
}

/// Lists at least this long are scored on several threads.
const PARALLEL_MIN: usize = 4_096;

/// Rank `candidates` (indices into `items`) for `typed`: best fuzzy score
/// first, then `sortText`. Items arrive sorted by `sortText` (the language
/// server reader sorts them), so the tie-break is the index and sorting
/// compares numbers only.
pub fn rank_completions(items: &[CompletionItem], candidates: &[usize], typed: &str) -> Vec<usize> {
    if typed.is_empty() {
        let mut v = candidates.to_vec();
        v.sort_unstable();
        return v;
    }
    let q = yace_search::fuzzy::Query::new(typed);
    // One character: the scorer's single scan is the filter.
    let single = typed.chars().count() == 1;
    let first = typed.chars().next().filter(|_| !single).map(|ch| ch.to_ascii_lowercase());
    let score = |part: &[usize]| -> Vec<(i32, u32)> {
        part.iter()
            .filter_map(|&i| {
                let it = &items[i];
                let key = it.filter_text.as_deref().unwrap_or(&it.label);
                // Cheap pre-filter before the fuzzy scorer.
                if let Some(f) = first
                    && !key.chars().any(|k| k.to_ascii_lowercase() == f)
                {
                    return None;
                }
                Some((yace_search::fuzzy::score_label(&q, key)?, i as u32))
            })
            .collect()
    };
    // Long lists are scored on several threads (the scratch buffers of the
    // scorer are per thread), then sorted once.
    let threads = std::thread::available_parallelism().map(|n| n.get().min(8)).unwrap_or(1);
    // One character is too cheap to be worth threads.
    let mut scored = if candidates.len() >= PARALLEL_MIN && threads > 1 && !single {
        let chunk = candidates.len().div_ceil(threads);
        std::thread::scope(|s| {
            let handles: Vec<_> = candidates.chunks(chunk).map(|part| s.spawn(move || score(part))).collect();
            handles.into_iter().flat_map(|h| h.join().unwrap_or_default()).collect::<Vec<_>>()
        })
    } else {
        score(candidates)
    };
    scored.sort_unstable_by_key(|&(score, i)| (std::cmp::Reverse(score), i));
    scored.into_iter().map(|(_, i)| i as usize).collect()
}

#[cfg(test)]
mod rank_tests {
    use super::*;

    fn item(label: &str) -> CompletionItem {
        serde_json::from_value(serde_json::json!({ "label": label })).unwrap()
    }

    /// Best score first; equal scores keep list (sortText) order.
    #[test]
    fn ranking_order() {
        let items: Vec<CompletionItem> = ["aRead", "read", "bRead", "x"].iter().map(|l| item(l)).collect();
        let all = [0, 1, 2, 3];
        assert_eq!(rank_completions(&items, &all, "read"), vec![1, 0, 2], "a prefix match wins; ties keep order");
        assert_eq!(rank_completions(&items, &[3, 1, 0], ""), vec![0, 1, 3], "nothing typed: list order");
    }
}
