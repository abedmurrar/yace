//! Quick Open (`Ctrl+P`, spec §6) and the command palette (`Ctrl+Shift+P`,
//! spec §7), sharing one dynamic picker. Modes by prefix:
//! files (default, `path:line:col`), `>` commands, `:` line, `?` help,
//! `@`/`#` symbols (M7).

use std::{
    path::{Path, PathBuf},
    time::Duration,
};

use yace_search::{Hit, QueryContext, RootKey, WalkOptions};
use yace_workspace::{RootId, RootStatus};

use super::{App, Effect};
use crate::{
    actions::PickerAction,
    overlay::{Overlay, Picker, PickerItem, PickerKind},
    workers::IndexCmd,
};

const RESULT_LIMIT: usize = 200;
const REWALK_AFTER: Duration = Duration::from_secs(30);

#[derive(Debug, Clone, PartialEq)]
pub enum Target {
    File {
        path: PathBuf,
        rel: Option<(RootKey, String)>,
        line: Option<usize>,
        col: Option<usize>,
    },
    Command(&'static str),
    GotoLine(usize, Option<usize>),
    SetPrefix(&'static str),
    /// A symbol (`@` / `#` modes).
    Symbol(super::lsp::NavTarget),
    /// A plugin provider's result: run this command.
    Plugin {
        command: String,
        args: Option<serde_json::Value>,
    },
    Nothing,
}

#[derive(Debug, Default)]
pub struct QuickOpenState {
    pub seq: u64,
    pub targets: Vec<Target>,
    pub include_ignored: bool,
    pub indexed: usize,
    pub complete: bool,
    pub truncated: bool,
    /// Recently chosen results (root, rel), most recent first.
    pub picked: Vec<(RootKey, String)>,
    pub recent_commands: Vec<&'static str>,
    /// Line/column parsed from the current file query.
    pos: (Option<usize>, Option<usize>),
    /// Title of the plugin provider being queried.
    pub plugin_title: String,
}

impl QuickOpenState {
    pub fn note_command(&mut self, id: &'static str) {
        self.recent_commands.retain(|c| *c != id);
        self.recent_commands.insert(0, id);
        self.recent_commands.truncate(30);
    }
}

/// Split `query:line[:col]`.
pub fn split_position(q: &str) -> (&str, Option<usize>, Option<usize>) {
    let num = |s: &str| s.parse::<usize>().ok().filter(|n| *n > 0);
    let mut parts = q.rsplitn(3, ':');
    let last = parts.next().unwrap_or("");
    let mid = parts.next();
    let first = parts.next();
    match (first, mid, num(last)) {
        (Some(f), Some(m), Some(c)) if num(m).is_some() => (f, num(m), Some(c)),
        (None, Some(m), Some(l)) => (m, Some(l), None),
        (Some(f), Some(m), Some(l)) => (&q[..f.len() + 1 + m.len()], Some(l), None),
        _ => (q, None, None),
    }
}

impl App {
    fn quick_open_picker(&mut self) -> Option<&mut Picker> {
        match self.overlays.last_mut() {
            Some(Overlay::Picker(p)) if p.action == PickerAction::QuickOpen => Some(p),
            _ => None,
        }
    }

    pub(crate) fn walk_options(&self) -> WalkOptions {
        let s = &self.settings;
        let mut exclude = s.files.exclude.clone();
        exclude.extend(s.search.exclude.iter().cloned());
        let include_ignored = self.quick_open.include_ignored || s.search.quick_open_include_ignored;
        WalkOptions {
            use_ignore_files: s.search.use_ignore_files && !include_ignored,
            include_hidden: true,
            exclude,
            max_files: s.search.max_indexed_files.max(1),
        }
    }

    /// Ask the index actor to mirror the roots and refresh stale ones.
    pub(crate) fn index_sync(&mut self) {
        let roots = self
            .workspace
            .roots()
            .iter()
            .filter(|r| r.status == RootStatus::Available)
            .map(|r| (r.id.0, r.display_name(), r.path.clone()))
            .collect();
        let cache_dir = self.settings.search.cache_index.then(|| self.paths.index_cache_dir());
        let walk = self.walk_options();
        self.emit(Effect::Index(IndexCmd::Sync { roots, walk, cache_dir, max_age: REWALK_AFTER }));
    }

    fn query_context(&self) -> QueryContext {
        let rel =
            |p: &Path| self.workspace.relative(p).map(|(r, rel)| (r.id.0, rel.to_string_lossy().replace('\\', "/")));
        let active_root = self
            .active()
            .and_then(|(_, _, d)| self.docs.doc(d)?.path().map(Path::to_path_buf))
            .and_then(|p| self.workspace.root_for_path(&p).map(|r| r.id.0));
        QueryContext {
            recent: self.recent_files.iter().filter_map(|p| rel(p)).take(50).collect(),
            picked: self.quick_open.picked.clone(),
            active_root,
        }
    }

    /// Open Quick Open with an initial input (`">"` for the palette).
    pub fn open_quick_open(&mut self, prefix: &str) {
        self.index_sync();
        let mut p = Picker::dynamic("Quick Open", PickerAction::QuickOpen);
        p.input = crate::overlay::LineInput::new(prefix);
        self.open_overlay(Overlay::Picker(p));
        self.quick_open_query_changed();
    }

    fn set_quick_open(&mut self, title: &str, items: Vec<(PickerItem, Vec<usize>, Target)>, empty: &str) {
        let keep = false;
        let targets: Vec<Target> = items.iter().map(|(_, _, t)| t.clone()).collect();
        let mut list = Vec::with_capacity(items.len());
        let mut hl = Vec::with_capacity(items.len());
        for (i, (mut it, h, _)) in items.into_iter().enumerate() {
            it.data = i;
            list.push(it);
            hl.push(h);
        }
        self.quick_open.targets = targets;
        if let Some(p) = self.quick_open_picker() {
            p.title = title.to_string();
            p.empty_text = empty.to_string();
            p.set_items(list, hl, keep);
        }
    }

    /// Recompute items after the input changed.
    pub(crate) fn quick_open_query_changed(&mut self) {
        let Some(text) = self.quick_open_picker().map(|p| p.input.text.clone()) else { return };
        self.quick_open.seq += 1;
        self.plugins.quick_open = None;
        if let Some((i, prefix)) = self.plugin_quick_open_provider(&text) {
            let q = text[prefix.len()..].trim().to_string();
            let title = self.plugins.entries[i]
                .manifest
                .as_ref()
                .and_then(|m| m.quick_open.iter().find(|x| x.prefix == prefix))
                .map(|x| x.title.clone())
                .unwrap_or_default();
            self.quick_open.plugin_title = title.clone();
            self.set_quick_open(&title, Vec::new(), "Loading…");
            self.plugin_quick_open(i, &prefix, &q);
            return;
        }
        if let Some(rest) = text.strip_prefix('>') {
            self.commands_mode(rest.trim());
        } else if let Some(rest) = text.strip_prefix(':') {
            self.line_mode(rest.trim());
        } else if text.starts_with('?') {
            self.help_mode();
        } else if let Some(rest) = text.strip_prefix('@') {
            self.doc_symbol_mode(rest.trim());
        } else if let Some(rest) = text.strip_prefix('#') {
            let q = rest.trim().to_string();
            self.lsp_workspace_symbols(&q);
            self.workspace_symbol_mode(&q);
        } else {
            self.files_mode(&text);
        }
    }

    /// `@`: symbols of the active document (requested once per version,
    /// filtered locally as you type).
    fn doc_symbol_mode(&mut self, q: &str) {
        let Some((_, _, d)) = self.active() else {
            let it = PickerItem::new("Open a text editor first to go to a symbol", "", "", 0);
            self.set_quick_open("Go to Symbol in Editor", vec![(it, vec![], Target::Nothing)], "");
            return;
        };
        let Some(path) = self.docs.doc(d).and_then(|x| x.path()).map(PathBuf::from) else { return };
        let ver = self.docs.doc(d).map(|x| x.version()).unwrap_or(0);
        let Some((_, _, syms, server)) =
            self.doc_symbols.as_ref().filter(|(dd, v, _, _)| *dd == d && *v == ver).cloned()
        else {
            let msg = if self.lsp.docs.contains_key(&d) {
                self.lsp_document_symbols();
                "Loading symbols…"
            } else {
                "No language server is running for this file"
            };
            let it = PickerItem::new(msg, "", "", 0);
            self.set_quick_open("Go to Symbol in Editor", vec![(it, vec![], Target::Nothing)], "");
            return;
        };
        let enc = self.lsp.server(server).map(|s| s.encoding()).unwrap_or_default();
        let query = yace_search::fuzzy::Query::new(q);
        let mut scored: Vec<(i32, usize, PickerItem, Vec<usize>, Target)> = syms
            .iter()
            .enumerate()
            .filter_map(|(i, s)| {
                let m = yace_search::fuzzy::match_label(&query, &s.name)?;
                let label = format!("{}{}", "  ".repeat(s.depth.min(6)), s.name);
                let shift = s.depth.min(6) * 2;
                let item = PickerItem::new(label, s.detail.clone(), super::lsp::symbol_kind(s.kind), 0);
                let t = super::lsp::NavTarget { path: path.clone(), pos: s.range.start, enc };
                Some((m.score, i, item, m.indices.iter().map(|x| x + shift).collect(), Target::Symbol(t)))
            })
            .collect();
        if !q.is_empty() {
            scored.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
        }
        let items = scored.into_iter().map(|(_, _, it, h, t)| (it, h, t)).collect();
        self.set_quick_open("Go to Symbol in Editor", items, "No matching symbols");
    }

    /// `#`: workspace symbols from the running servers (they filter by the
    /// query; we rank).
    fn workspace_symbol_mode(&mut self, q: &str) {
        let running =
            self.lsp.servers.iter().any(|s| s.status == crate::lsp::Status::Running && s.caps.workspace_symbol);
        if !running {
            let it = PickerItem::new(
                "No language server with workspace symbols is running",
                "open a source file first",
                "",
                0,
            );
            self.set_quick_open("Go to Symbol in Workspace", vec![(it, vec![], Target::Nothing)], "");
            return;
        }
        let query = yace_search::fuzzy::Query::new(q);
        let results = self.workspace_symbols.1.clone();
        let mut scored: Vec<(i32, usize, PickerItem, Vec<usize>, Target)> = results
            .iter()
            .enumerate()
            .filter_map(|(i, (s, server))| {
                let m = yace_search::fuzzy::match_label(&query, &s.name)?;
                let path = s.uri.as_deref().and_then(yace_lsp::uri::to_path)?;
                let enc = self.lsp.server(*server).map(|x| x.encoding()).unwrap_or_default();
                let rel = match self.workspace.relative(&path) {
                    Some((root, rel)) if self.workspace.roots().len() > 1 => {
                        format!("{} · {}", root.display_name(), rel.display())
                    }
                    Some((_, rel)) => rel.display().to_string(),
                    None => path.display().to_string(),
                };
                let detail = if s.container.is_empty() { rel } else { format!("{} — {rel}", s.container) };
                let item = PickerItem::new(s.name.clone(), detail, super::lsp::symbol_kind(s.kind), 0);
                let t = super::lsp::NavTarget { path, pos: s.range.start, enc };
                Some((m.score, i, item, m.indices, Target::Symbol(t)))
            })
            .take(500)
            .collect();
        scored.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
        let items = scored.into_iter().map(|(_, _, it, h, t)| (it, h, t)).collect();
        self.set_quick_open(
            "Go to Symbol in Workspace",
            items,
            if q.is_empty() { "Type to search symbols" } else { "No matching symbols" },
        );
    }

    /// Symbols arrived: refresh Quick Open if it is in `@` / `#` mode.
    pub(crate) fn refresh_symbol_quick_open(&mut self) {
        let Some(text) = self.quick_open_picker().map(|p| p.input.text.clone()) else { return };
        if let Some(rest) = text.strip_prefix('@') {
            self.doc_symbol_mode(rest.trim());
        } else if let Some(rest) = text.strip_prefix('#') {
            self.workspace_symbol_mode(rest.trim());
        }
    }

    fn commands_mode(&mut self, q: &str) {
        let ctx = self.context();
        let query = yace_search::fuzzy::Query::new(q);
        let ids = self.commands.palette_ids().to_vec();
        let recent = self.quick_open.recent_commands.clone();
        let mut scored: Vec<(i32, usize, PickerItem, Vec<usize>, Target)> = Vec::new();
        for (order, id) in ids.iter().enumerate() {
            let Some(c) = self.commands.get(id) else { continue };
            let label = format!("{}: {}", c.category, c.title);
            let m = yace_search::fuzzy::match_label(&query, &label).or_else(|| {
                yace_search::fuzzy::match_label(&query, id)
                    .map(|m| yace_search::fuzzy::FuzzyMatch { indices: vec![], ..m })
            });
            let Some(m) = m else { continue };
            let keys = self.keymap.keys_for(id, Some(&ctx)).or_else(|| self.keymap.keys_for(id, None));
            let recent_rank = recent.iter().position(|r| r == id);
            let mut hint = keys.map(|k| k.to_string()).unwrap_or_default();
            if recent_rank.is_some() && q.is_empty() {
                hint = if hint.is_empty() { "recently used".into() } else { format!("recently used  {hint}") };
            }
            let bonus = recent_rank.map_or(0, |r| 1000 - r as i32 * 10);
            scored.push((m.score + bonus, order, PickerItem::new(label, "", hint, 0), m.indices, Target::Command(id)));
        }
        scored.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
        let items = scored.into_iter().map(|(_, _, it, h, t)| (it, h, t)).collect();
        self.set_quick_open("Command Palette", items, "No matching commands");
    }

    fn line_mode(&mut self, q: &str) {
        let Some((_, _, d)) = self.active() else {
            let it = PickerItem::new("Open a text editor first to go to a line", "", "", 0);
            self.set_quick_open("Go to Line", vec![(it, vec![], Target::Nothing)], "");
            return;
        };
        let lines = self.docs.doc(d).map(|x| x.rope().len_lines()).unwrap_or(1);
        let (_, line, col) = split_position(&format!("x:{q}"));
        let item = match line {
            Some(l) => {
                let l = l.min(lines);
                let label = match col {
                    Some(c) => format!("Go to line {l}, column {c}"),
                    None => format!("Go to line {l}"),
                };
                (PickerItem::new(label, "", "", 0), vec![], Target::GotoLine(l, col))
            }
            None => (
                PickerItem::new(
                    format!("Current line count: {lines}. Type a line number (e.g. :42 or :42:8)"),
                    "",
                    "",
                    0,
                ),
                vec![],
                Target::Nothing,
            ),
        };
        self.set_quick_open("Go to Line", vec![item], "");
    }

    /// Results from a plugin provider (in the plugin's order).
    pub(crate) fn set_plugin_quick_open_items(&mut self, items: Vec<crate::plugins::QuickOpenIn>) {
        let list = items
            .into_iter()
            .take(RESULT_LIMIT)
            .map(|it| {
                let item = PickerItem::new(it.label, it.description, "", 0);
                (item, Vec::new(), Target::Plugin { command: it.command, args: it.args })
            })
            .collect();
        let title = self.quick_open.plugin_title.clone();
        self.set_quick_open(&title, list, "No results");
    }

    /// A single informational row (e.g. the provider's plugin is disabled).
    pub(crate) fn quick_open_plugin_note(&mut self, text: &str) {
        let title = self.quick_open.plugin_title.clone();
        self.set_quick_open(&title, vec![(PickerItem::new(text, "", "", 0), vec![], Target::Nothing)], "");
    }

    fn help_mode(&mut self) {
        let modes: [(&'static str, &str, &str); 5] = [
            ("", "Go to File", "type a file name; add :line[:col] to jump"),
            (">", "Show and Run Commands", "Ctrl+Shift+P"),
            (":", "Go to Line", "Ctrl+G"),
            ("@", "Go to Symbol in Editor", ""),
            ("#", "Go to Symbol in Workspace", ""),
        ];
        let mut items: Vec<(PickerItem, Vec<usize>, Target)> = modes
            .iter()
            .map(|(p, label, hint)| {
                let shown = if p.is_empty() { "…".to_string() } else { p.to_string() };
                (PickerItem::new(format!("{shown}  {label}"), "", *hint, 0), vec![], Target::SetPrefix(p))
            })
            .collect();
        for (prefix, title) in self.plugin_quick_open_help() {
            let p = crate::commands::intern(&prefix);
            items.push((PickerItem::new(format!("{prefix}  {title}"), "", "plugin", 0), vec![], Target::SetPrefix(p)));
        }
        self.set_quick_open("Quick Open Help", items, "");
    }

    fn files_mode(&mut self, text: &str) {
        let (q, line, col) = split_position(text.trim());
        self.quick_open.pos = (line, col);
        let q = q.trim().to_string();
        if q.is_empty() {
            // Recently opened files, instantly.
            let mut items = Vec::new();
            for p in self.recent_files.clone().into_iter().filter(|p| p.is_file()).take(RESULT_LIMIT) {
                let rel = self.workspace.relative(&p).map(|(r, rel)| (r.id.0, rel.to_string_lossy().into_owned()));
                items.push(self.file_item(&p, rel, &[], line, col, "recently opened"));
            }
            let title = self.quick_open_title();
            self.set_quick_open(&title, items, "Type to search files across all workspace folders");
            return;
        }
        // A literal path that exists opens directly.
        let literal = yace_workspace::paths::expand_tilde(&q);
        let direct = if literal.is_absolute() && literal.is_file() { Some(literal) } else { None };
        if let Some(p) = &direct
            && let Some(picker) = self.quick_open_picker()
            && picker.items.is_empty()
        {
            let _ = picker;
            let item = self.file_item(p, None, &[], line, col, "path");
            let title = self.quick_open_title();
            self.set_quick_open(&title, vec![item], "");
        }
        let ctx = self.query_context();
        let seq = self.quick_open.seq;
        self.emit(Effect::Index(IndexCmd::Query { seq, query: q, ctx, limit: RESULT_LIMIT }));
    }

    fn file_item(
        &self,
        path: &Path,
        rel: Option<(RootKey, String)>,
        indices: &[usize],
        line: Option<usize>,
        col: Option<usize>,
        fallback_hint: &str,
    ) -> (PickerItem, Vec<usize>, Target) {
        let (label, detail, hint, hl) = match &rel {
            Some((root, r)) => {
                let name_start_bytes = r.rfind('/').map_or(0, |i| i + 1);
                let name_start = r[..name_start_bytes].chars().count();
                let name = r[name_start_bytes..].to_string();
                let parent = r[..name_start_bytes].trim_end_matches('/').to_string();
                let root_name = self.workspace.root(RootId(*root)).map(|x| x.display_name()).unwrap_or_default();
                let hl: Vec<usize> = indices.iter().filter(|i| **i >= name_start).map(|i| i - name_start).collect();
                let detail = if parent.is_empty() { String::new() } else { format!("{parent}/") };
                (name, detail, root_name.to_uppercase(), hl)
            }
            None => (
                path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
                path.parent().map(|p| p.display().to_string()).unwrap_or_default(),
                fallback_hint.to_string(),
                vec![],
            ),
        };
        (PickerItem::new(label, detail, hint, 0), hl, Target::File { path: path.to_path_buf(), rel, line, col })
    }

    fn quick_open_title(&self) -> String {
        let q = &self.quick_open;
        let mut t = String::from("Go to File");
        if !q.complete && q.indexed > 0 {
            t.push_str(&format!("  · indexing… {} files", group_digits(q.indexed)));
        } else if !q.complete {
            t.push_str("  · indexing…");
        } else {
            t.push_str(&format!("  · {} files", group_digits(q.indexed)));
        }
        if q.truncated {
            t.push_str(" (limit reached)");
        }
        if q.include_ignored {
            t.push_str("  · including ignored (Alt+I)");
        }
        t
    }

    pub(crate) fn on_quick_open_results(
        &mut self,
        seq: u64,
        hits: Vec<Hit>,
        indexed: usize,
        complete: bool,
        truncated: bool,
    ) {
        self.quick_open.indexed = indexed;
        self.quick_open.complete = complete;
        self.quick_open.truncated = truncated;
        if seq != self.quick_open.seq || self.quick_open_picker().is_none() {
            return;
        }
        let (line, col) = self.quick_open.pos;
        let mut items = Vec::with_capacity(hits.len());
        for h in &hits {
            let Some(root) = self.workspace.root(RootId(h.root)) else { continue };
            let path = root.path.join(&*h.rel);
            items.push(self.file_item(&path, Some((h.root, h.rel.to_string())), &h.indices, line, col, ""));
        }
        let title = self.quick_open_title();
        let empty = if complete { "No matching files" } else { "Searching… (indexing in progress)" };
        self.set_quick_open(&title, items, empty);
    }

    pub(crate) fn on_index_progress(&mut self, files: usize, complete: bool) {
        self.quick_open.indexed = files;
        self.quick_open.complete = complete;
        let Some(text) =
            self.quick_open_picker().filter(|p| p.kind == PickerKind::Dynamic).map(|p| p.input.text.clone())
        else {
            return;
        };
        let is_files = !text.starts_with(['>', ':', '?', '@', '#']);
        if is_files && !split_position(text.trim()).0.trim().is_empty() {
            self.quick_open_query_changed();
        } else if let Some(p) = self.quick_open_picker()
            && is_files
        {
            let _ = p;
            let title = self.quick_open_title();
            if let Some(p) = self.quick_open_picker() {
                p.title = title;
            }
        }
    }

    pub(crate) fn quick_open_toggle_ignored(&mut self) {
        self.quick_open.include_ignored = !self.quick_open.include_ignored;
        self.index_sync();
        self.quick_open_query_changed();
    }

    pub(crate) fn quick_open_is_file(&self, data: usize) -> bool {
        matches!(self.quick_open.targets.get(data), Some(Target::File { .. }))
    }

    pub(crate) fn quick_open_pick(&mut self, data: usize) {
        let Some(t) = self.quick_open.targets.get(data).cloned() else { return };
        match t {
            Target::File { path, rel, line, col } => {
                if let Some(r) = rel {
                    self.quick_open.picked.retain(|p| *p != r);
                    self.quick_open.picked.insert(0, r);
                    self.quick_open.picked.truncate(50);
                }
                self.record_jump();
                self.open_file(&path, false);
                if let Some(l) = line {
                    self.set_cursor_line_col(l - 1, col.unwrap_or(1) - 1);
                }
            }
            Target::Command(id) => {
                self.quick_open.note_command(id);
                self.execute(id, None);
            }
            Target::GotoLine(l, c) => {
                self.record_jump();
                self.set_cursor_line_col(l.saturating_sub(1), c.unwrap_or(1).saturating_sub(1));
                self.focus_editor();
            }
            Target::SetPrefix(p) => self.open_quick_open(p),
            Target::Symbol(t) => self.jump_to(&t),
            Target::Nothing => {}
            Target::Plugin { command, args } => self.execute(&command, args.as_ref()),
        }
    }

    /// Patch the index from filesystem events (bounded; spec §23.4).
    pub(crate) fn index_fs_changes(&mut self, paths: &[PathBuf]) {
        for p in paths {
            let Some((root, rel)) = self.workspace.relative(p) else { continue };
            let rel = rel.to_string_lossy().replace('\\', "/");
            if rel.is_empty() {
                continue;
            }
            let meta = std::fs::symlink_metadata(p);
            let exists = meta.is_ok();
            let is_dir = meta.map(|m| m.is_dir()).unwrap_or(false);
            let root = root.id.0;
            self.emit(Effect::Index(IndexCmd::FsChange { root, rel, exists, is_dir }));
        }
    }
}

fn group_digits(n: usize) -> String {
    let s = n.to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn positions() {
        assert_eq!(split_position("index.ts"), ("index.ts", None, None));
        assert_eq!(split_position("index.ts:42"), ("index.ts", Some(42), None));
        assert_eq!(split_position("index.ts:42:8"), ("index.ts", Some(42), Some(8)));
        assert_eq!(split_position("a:b:7"), ("a:b", Some(7), None));
        assert_eq!(group_digits(1234567), "1,234,567");
    }
}
