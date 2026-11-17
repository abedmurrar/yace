//! Inlay hints (`textDocument/inlayHint`): types and parameter names shown
//! inside the code. Asked for the visible lines plus a screen above and
//! below, 250 ms after an edit, a scroll or a resize; edits move the hints
//! already shown until the new answer arrives, so nothing flickers.

use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
    time::{Duration, Instant},
};

use serde_json::json;
use unicode_width::UnicodeWidthChar;
use yace_buffer::{Assoc, ChangeSet};
use yace_lsp::{InlayHint, from_lsp, to_lsp};

use super::App;
use crate::{
    display::{DisplayMap, Inlay},
    editor::DocId,
    lsp::{ReqKind, Status},
};

/// Wait after an edit or a scroll before asking.
pub const INLAY_DELAY: Duration = Duration::from_millis(250);
/// Longer hints are cut (VS Code's `editor.inlayHints.maximumLength`).
const MAX_CELLS: usize = 43;

/// One document's hints.
#[derive(Debug, Default)]
pub struct DocInlays {
    by_server: HashMap<u32, Vec<Inlay>>,
    /// Every server's hints, sorted by position (what the display shows;
    /// shared, so building a display map doesn't copy them).
    pub merged: Arc<Vec<Inlay>>,
    /// The document version and lines last asked for.
    asked: Option<(u64, usize, usize)>,
    in_flight: HashSet<u32>,
    /// Asked again while a request was in flight.
    again: bool,
}

impl DocInlays {
    fn merge(&mut self) {
        let mut all: Vec<Inlay> = self.by_server.values().flatten().cloned().collect();
        all.sort_by_key(|h| h.pos);
        self.merged = Arc::new(all);
    }

    /// Follow an edit: hints inside replaced text go, the rest move (in
    /// place: mapping keeps their order).
    pub fn map(&mut self, cs: &ChangeSet) {
        let follow = |hints: &mut Vec<Inlay>| {
            hints.retain(|h| !cs.changes().iter().any(|c| c.from < h.pos && h.pos < c.to));
            for h in hints.iter_mut() {
                h.pos = cs.map_pos(h.pos, Assoc::After);
            }
        };
        self.by_server.values_mut().for_each(follow);
        follow(Arc::make_mut(&mut self.merged));
    }
}

/// A hint's label, cut to `MAX_CELLS`.
fn clip(label: &str) -> String {
    let mut out = String::new();
    let mut cells = 0;
    for c in label.chars() {
        let w = c.width().unwrap_or(0);
        if cells + w > MAX_CELLS - 1 {
            out.push('…');
            return out;
        }
        cells += w;
        out.push(c);
    }
    out
}

impl App {
    /// The hints shown in a document.
    pub fn inlays_of(&self, d: DocId) -> &[Inlay] {
        self.inlays.get(&d).map_or(&[], |x| x.merged.as_slice())
    }

    /// The same, shared (for a display map built later).
    pub(crate) fn inlays_shared(&self, d: DocId) -> Arc<Vec<Inlay>> {
        self.inlays.get(&d).map(|x| x.merged.clone()).unwrap_or_default()
    }

    fn inlays_enabled(&self, d: DocId) -> bool {
        self.docs.doc(d).is_some_and(|doc| self.settings_for(doc.path()).editor.inlay_hints_enabled)
    }

    /// Running servers of `d` that give inlay hints.
    fn inlay_servers(&self, d: DocId) -> Vec<u32> {
        self.lsp
            .attachments(d)
            .iter()
            .map(|l| l.server)
            .filter(|id| {
                self.lsp
                    .server(*id)
                    .is_some_and(|s| s.status == Status::Running && s.caps.inlay_hint && s.docs.contains(&d))
            })
            .collect()
    }

    /// The documents shown in the groups, with their visible lines.
    fn inlay_views(&self) -> Vec<(DocId, usize, usize)> {
        let wl = crate::ui::workbench_layout(self);
        let mut out = Vec::new();
        for gl in &wl.groups {
            let Some(view) = self.layout.group(gl.id).and_then(|g| g.active_view()) else { continue };
            let d = view.doc;
            if self.inlay_servers(d).is_empty() || !self.inlays_enabled(d) {
                continue;
            }
            let Some(doc) = self.docs.doc(d) else { continue };
            let tab = self.settings_for(doc.path()).editor.tab_size.max(1);
            let wrap = self.wrap_width(d, gl.text_width as usize);
            let map = DisplayMap::new(doc.rope(), &view.folds, wrap, tab).with_inlays(self.inlays_of(d));
            let rows = map.rows(view.top(), (gl.content.height as usize).max(1));
            let first = rows.first().map_or(0, |r| r.line);
            let last = rows.last().map_or(first, |r| r.line);
            out.push((d, first, last));
        }
        out
    }

    /// After each batch of events (on tick): ask again soon when a shown
    /// document changed or scrolled past the lines already asked for.
    pub(crate) fn inlay_track(&mut self) {
        // Hints of documents no longer shown, or with the setting off, go.
        let shown: HashSet<DocId> =
            self.layout.groups().iter().filter_map(|g| g.active_view()).map(|v| v.doc).collect();
        let off: Vec<DocId> =
            self.inlays.keys().copied().filter(|d| !shown.contains(d) || !self.inlays_enabled(*d)).collect();
        for d in off {
            self.inlays.remove(&d);
        }
        if self.inlay_deadline.is_some() || !self.lsp.servers.iter().any(|s| s.caps.inlay_hint) {
            return;
        }
        for (d, first, last) in self.inlay_views() {
            let version = self.docs.doc(d).map_or(0, |x| x.version());
            let covered = self
                .inlays
                .get(&d)
                .and_then(|x| x.asked)
                .is_some_and(|(v, a, b)| v == version && a <= first && last <= b);
            if !covered {
                self.inlay_deadline = Some(Instant::now() + INLAY_DELAY);
                return;
            }
        }
    }

    pub(crate) fn inlay_on_tick(&mut self, now: Instant) {
        if !self.inlay_deadline.is_some_and(|t| now >= t) {
            return;
        }
        self.inlay_deadline = None;
        self.sync_documents();
        for (d, first, last) in self.inlay_views() {
            let height = last - first + 1;
            let Some(doc) = self.docs.doc(d) else { continue };
            let version = doc.version();
            let rope = doc.rope().clone();
            let lo = first.saturating_sub(height);
            let hi = (last + height).min(rope.len_lines().saturating_sub(1));
            let state = self.inlays.entry(d).or_default();
            if state.asked.is_some_and(|(v, a, b)| v == version && a <= first && last <= b) {
                continue;
            }
            state.asked = Some((version, lo, hi));
            for id in self.inlay_servers(d) {
                let state = self.inlays.entry(d).or_default();
                if !state.in_flight.insert(id) {
                    state.again = true;
                    continue;
                }
                let Some(uri) = self.lsp.attachments(d).into_iter().find(|l| l.server == id).map(|l| l.uri) else {
                    continue;
                };
                let enc = self.lsp.server(id).map(|s| s.encoding()).unwrap_or_default();
                let end = if hi + 1 < rope.len_lines() { rope.line_to_char(hi + 1) } else { rope.len_chars() };
                let range = json!({
                    "start": to_lsp(&rope, rope.line_to_char(lo), enc),
                    "end": to_lsp(&rope, end, enc),
                });
                let params = json!({ "textDocument": { "uri": uri }, "range": range });
                self.lsp.request(id, "textDocument/inlayHint", params, ReqKind::InlayHints, Some(d), version);
            }
        }
    }

    /// A server answered (or failed: `hints` is None).
    pub(crate) fn on_inlay_hints(&mut self, server: u32, d: DocId, fresh: bool, hints: Option<Vec<InlayHint>>) {
        let enc = self.lsp.server(server).map(|s| s.encoding()).unwrap_or_default();
        let rope = self.docs.doc(d).map(|x| x.rope().clone());
        let Some(state) = self.inlays.get_mut(&d) else { return };
        state.in_flight.remove(&server);
        let again = std::mem::take(&mut state.again);
        if let (true, Some(hints), Some(rope)) = (fresh, hints, rope) {
            let list = hints.iter().map(|h| Inlay::new(from_lsp(&rope, h.position, enc), clip(&h.label))).collect();
            state.by_server.insert(server, list);
            state.merge();
        }
        if again || !fresh {
            // Ask again for the current text and lines.
            state.asked = None;
            self.inlay_deadline.get_or_insert(Instant::now() + INLAY_DELAY);
        }
    }

    /// The server's hints are out of date (`workspace/inlayHint/refresh`).
    pub(crate) fn inlay_refresh(&mut self, server: u32) {
        for d in self.lsp.docs_of(server) {
            if let Some(x) = self.inlays.get_mut(&d) {
                x.asked = None;
            }
        }
        self.inlay_deadline.get_or_insert(Instant::now() + INLAY_DELAY);
    }

    /// A server stopped serving `d` (or at all, with `d` None).
    pub(crate) fn inlay_drop_server(&mut self, server: u32, d: Option<DocId>) {
        for (doc, x) in self.inlays.iter_mut() {
            if d.is_none_or(|d| d == *doc) {
                x.in_flight.remove(&server);
                if x.by_server.remove(&server).is_some() {
                    x.merge();
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn long_labels_are_cut() {
        assert_eq!(clip(": i32"), ": i32");
        let long = clip(&"x".repeat(80));
        assert_eq!(long.chars().count(), MAX_CELLS);
        assert!(long.ends_with('…'));
    }
}
