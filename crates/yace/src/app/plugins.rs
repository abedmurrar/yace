//! Native plugins in the app (spec §18): discovery, contributions, lazy
//! activation, commands, events, views, status items, decorations, Quick
//! Open providers, and the editor side of every host function.

use std::{
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

use serde_json::{Value, json};
use yace_buffer::{Change, ChangeSet};
use yace_keymap::BindingSpec;
use yace_plugin_api::{YACE_E_NOT_FOUND, YACE_E_PANIC, YACE_E_STALE, YACE_E_UNSUPPORTED, YACE_OK, YaceStatus};
use yace_plugin_host::{Loaded, TrustRecord, store};

use super::{App, Effect, Focus, Severity, SidebarView};
use crate::{
    commands::PluginCmd,
    plugins::{
        self, DecorationIn, EditIn, Observed, PluginEntry, PluginState, QuickOpenIn, StatusContent, TerminalIn,
        ViewNode, ViewUi,
    },
};

/// A callback slower than this gets a (one-time) warning per plugin.
pub const SLOW_WARNING: Duration = Duration::from_millis(100);
/// Failed calls before a plugin is stopped for the session.
pub const MAX_FAILURES: u32 = 3;
/// Coalescing delay for high-frequency events (selection, typing).
pub const COALESCE: Duration = Duration::from_millis(50);

impl App {
    pub fn plugins_dir(&self) -> PathBuf {
        self.paths.config_dir.join("plugins")
    }

    /// The YACE version plugins are checked against.
    pub fn yace_version() -> &'static str {
        env!("CARGO_PKG_VERSION")
    }

    /// Start (or refresh) plugin discovery on a worker: only manifests are
    /// read; no plugin code is loaded.
    pub fn discover_plugins(&mut self) {
        self.effects.push(Effect::DiscoverPlugins(self.plugins_dir()));
    }

    /// `--safe-mode` / `--no-plugins`.
    pub fn set_safe_mode(&mut self, on: bool) {
        self.plugins.safe_mode = on;
    }

    fn workspace_disabled(&self, id: &str) -> Option<bool> {
        if self.plugin_ws_enabled.iter().any(|x| x == id) {
            Some(false)
        } else if self.plugin_ws_disabled.iter().any(|x| x == id) {
            Some(true)
        } else {
            None
        }
    }

    fn is_disabled(&self, id: &str) -> bool {
        self.workspace_disabled(id).unwrap_or_else(|| self.settings.plugins.disabled.iter().any(|x| x == id))
    }

    /// Discovery finished: rebuild the plugin list, keeping plugins that
    /// are already running and unchanged.
    pub fn on_plugins_discovered(&mut self, found: Vec<store::Discovered>) {
        let trust = TrustRecord::load(&self.plugins_dir());
        let mut old: Vec<PluginEntry> = std::mem::take(&mut self.plugins.entries);
        let mut entries = Vec::new();
        for d in found {
            let index = entries.len();
            let keep = old
                .iter()
                .position(|e| e.dir == d.dir && e.loaded.is_some() && e.manifest.as_ref() == d.manifest.as_ref().ok());
            if let Some(k) = keep {
                let mut e = old.remove(k);
                if e.ctx.shared.index != index {
                    // Indices changed: such a plugin must be restarted to
                    // get a new context; simplest is to stop it here.
                    if let Some(mut l) = e.loaded.take() {
                        l.shutdown();
                    }
                    e.state = PluginState::Inactive;
                    e.ctx = self.plugins.new_ctx(index);
                }
                entries.push(e);
                continue;
            }
            let (manifest, problems) = match d.manifest {
                Ok(m) => (Some(m), Vec::new()),
                Err(p) => (None, p),
            };
            let state = match &manifest {
                None => PluginState::Invalid,
                Some(m) => {
                    if self.plugins.safe_mode {
                        PluginState::Disabled
                    } else if let Err(why) = m.compatible(Self::yace_version()) {
                        PluginState::Incompatible(why)
                    } else if self.is_disabled(&m.id) {
                        PluginState::Disabled
                    } else if !m.code_files().is_empty() && !trust.plugins.contains_key(&m.id) {
                        PluginState::Untrusted(
                            "not installed through YACE (use Extensions: Install from Directory…)".into(),
                        )
                    } else {
                        PluginState::Inactive
                    }
                }
            };
            entries.push(PluginEntry {
                folder: d.folder,
                dir: d.dir,
                manifest,
                problems,
                state,
                loaded: None,
                ctx: self.plugins.new_ctx(index),
                activation_time: None,
                status: Default::default(),
                views: Default::default(),
                decorations: Default::default(),
                log: Default::default(),
                failures: 0,
                slow_warned: false,
            });
        }
        // Plugins that disappeared or changed: stop them.
        for mut e in old {
            if let Some(mut l) = e.loaded.take() {
                self.plugins.jobs.cancel_plugin(e.ctx.shared.index);
                l.shutdown();
            }
        }
        self.plugins.entries = entries;
        self.plugins.discovered = true;
        self.register_plugin_contributions();
        self.activate_plugins_for_startup();
    }

    /// Commands, keybindings, themes and languages of every plugin that
    /// contributes, from manifests alone.
    pub(crate) fn register_plugin_contributions(&mut self) {
        let mut cmds = Vec::new();
        for (i, e) in self.plugins.entries.iter().enumerate() {
            if !e.state.contributes() {
                continue;
            }
            let Some(m) = &e.manifest else { continue };
            for c in &m.commands {
                let category = if c.category.is_empty() { m.name.clone() } else { c.category.clone() };
                cmds.push((c.id.clone(), category, c.title.clone(), c.palette, PluginCmd::Plugin(i)));
            }
            for v in &m.views {
                cmds.push((
                    format!("{}.focus", v.id),
                    "View".to_string(),
                    format!("Show {}", v.title),
                    true,
                    PluginCmd::ShowView(i, v.id.clone()),
                ));
            }
        }
        let clashes = self.commands.set_plugin_commands(cmds);
        for c in clashes {
            self.notify(
                Severity::Warning,
                format!("A plugin command `{c}` clashes with a built-in command and was skipped"),
            );
        }
        self.reload_keymap();
        self.invalidate_theme();
        let e = self.load_theme_from_settings();
        self.report_config_errors(e);
        self.register_plugin_grammars();
        self.register_plugin_servers();
        self.needs_render = true;
    }

    /// Language servers of enabled plugins (none in safe mode). Like
    /// grammars, they need no approval of code: the program isn't part of
    /// the plugin, and the install prompt names it.
    fn register_plugin_servers(&mut self) {
        let mut defs = Vec::new();
        if !self.plugins.safe_mode {
            for e in &self.plugins.entries {
                if !matches!(e.state, PluginState::Inactive | PluginState::Active) {
                    continue;
                }
                let Some(m) = &e.manifest else { continue };
                defs.extend(m.lsp_servers.iter().map(|s| crate::lsp::plugin_def(&m.id, s)));
            }
        }
        self.set_plugin_servers(defs);
    }

    /// Keybindings plugins contribute (applied between defaults and the
    /// user's file).
    pub(crate) fn plugin_keybinding_specs(&self) -> Vec<BindingSpec> {
        let mut out = Vec::new();
        for e in &self.plugins.entries {
            if !e.state.contributes() {
                continue;
            }
            let Some(m) = &e.manifest else { continue };
            for k in &m.keybindings {
                out.push(BindingSpec {
                    key: Some(k.key.clone()),
                    command: k.command.clone(),
                    when: (!k.when.is_empty()).then(|| k.when.clone()),
                    args: k.args.as_ref().and_then(|a| serde_json::to_value(a).ok()),
                });
            }
        }
        out
    }

    /// Theme files plugins contribute.
    pub(crate) fn plugin_theme_files(&self) -> Vec<PathBuf> {
        let mut out = Vec::new();
        for e in &self.plugins.entries {
            if !e.state.contributes() && e.state != PluginState::Disabled {
                continue;
            }
            if self.plugins.safe_mode || e.state == PluginState::Disabled {
                continue;
            }
            if let Some(m) = &e.manifest {
                out.extend(m.themes.iter().filter_map(|t| store_safe(&e.dir, &t.path)));
            }
        }
        out
    }

    // ------------------------------------------------------------------
    // Activation (spec §18.5, §18.6)

    fn activate_plugins_for_startup(&mut self) {
        let roots: Vec<PathBuf> = self.workspace.roots().iter().map(|r| r.path.clone()).collect();
        let mut todo = Vec::new();
        for (i, e) in self.plugins.entries.iter().enumerate() {
            if e.state != PluginState::Inactive {
                continue;
            }
            let Some(m) = &e.manifest else { continue };
            let hit = m.activation.iter().any(|a| {
                a == "*"
                    || a == "onStartupFinished"
                    || a.strip_prefix("workspaceContains:")
                        .is_some_and(|p| store_rel(p).is_some_and(|rel| roots.iter().any(|r| r.join(&rel).exists())))
            });
            if hit {
                todo.push(i);
            }
        }
        for i in todo {
            self.activate_plugin(i);
        }
        // Languages of documents already open.
        let langs: Vec<String> = self.docs.ids().into_iter().map(|d| self.syntax.language(d).id.to_string()).collect();
        for l in langs {
            self.activate_plugins_on(&format!("onLanguage:{l}"));
        }
    }

    /// Activate every inactive plugin listening for `event` (e.g.
    /// `onCommand:x`, `onLanguage:rust`, `onView:v`, `onEvent:documentSaved`).
    pub(crate) fn activate_plugins_on(&mut self, event: &str) {
        let todo: Vec<usize> = self
            .plugins
            .entries
            .iter()
            .enumerate()
            .filter(|(_, e)| e.state == PluginState::Inactive)
            .filter(|(_, e)| e.manifest.as_ref().is_some_and(|m| m.activation.iter().any(|a| a == event)))
            .map(|(i, _)| i)
            .collect();
        for i in todo {
            self.activate_plugin(i);
        }
    }

    /// Load and activate a plugin. Returns true if it is active.
    pub(crate) fn activate_plugin(&mut self, i: usize) -> bool {
        let Some(e) = self.plugins.entries.get(i) else { return false };
        match &e.state {
            PluginState::Active => return true,
            PluginState::Inactive => {}
            _ => return false,
        }
        let Some(m) = e.manifest.clone() else { return false };
        let Some(lib) = m.library.as_deref().and_then(|l| store_safe(&e.dir, l)) else {
            return false; // data-only plugins have nothing to activate
        };
        let dir = e.dir.clone();
        let start = Instant::now();
        // The code must still be what the user approved.
        if let Err(why) = store::verify_code(&dir, &m, &TrustRecord::load(&self.plugins_dir())) {
            self.plugin_failed(i, PluginState::Untrusted(why.clone()), &why);
            return false;
        }
        self.crash_marker_add(&m.id);
        let host = plugins::host_table(&self.plugins.entries[i].ctx);
        let (loaded, _) = match Loaded::open(&lib, host, m.unloadable) {
            Ok(l) => l,
            Err(why) => {
                self.plugin_failed(i, PluginState::Failed(why.clone()), &why);
                return false;
            }
        };
        self.plugins.entries[i].loaded = Some(loaded);
        self.plugins.entries[i].state = PluginState::Active;
        let r = plugins::call(self, i, |l| l.activate());
        let (status, took) = r.unwrap_or((YACE_E_UNSUPPORTED, Duration::ZERO));
        self.after_call(i, status, took, "activate");
        if status != YACE_OK {
            let why = format!("activation failed ({})", yace_plugin_host::status_name(status));
            self.stop_plugin(i, PluginState::Failed(why.clone()));
            self.notify(Severity::Error, format!("Plugin {}: {why}", m.name));
            return false;
        }
        self.plugins.entries[i].activation_time = Some(start.elapsed());
        self.plugin_log(
            i,
            yace_plugin_api::YACE_LEVEL_DEBUG,
            format!("activated in {:.1} ms", start.elapsed().as_secs_f64() * 1e3),
        );
        self.run_deferred_plugin_commands();
        self.needs_render = true;
        true
    }

    fn plugin_failed(&mut self, i: usize, state: PluginState, why: &str) {
        let name = self.plugins.entries[i].name().to_string();
        self.plugins.entries[i].state = state;
        self.plugins.entries[i].push_log(format!("not loaded: {why}"));
        self.notify_sticky(Severity::Error, format!("Plugin {name} was not loaded: {why}"));
    }

    /// Deactivate a plugin, cancel its jobs and remove what it showed.
    pub(crate) fn stop_plugin(&mut self, i: usize, state: PluginState) {
        self.plugins.jobs.cancel_plugin(i);
        if self.plugins.entries.get(i).is_some_and(|e| e.loaded.is_some()) {
            let r = plugins::call(self, i, |l| l.shutdown());
            if let Some(d) = r {
                self.after_call(i, YACE_OK, d, "deactivate");
            }
            if let Some(e) = self.plugins.entries.get_mut(i) {
                e.loaded = None;
            }
        }
        if let Some(e) = self.plugins.entries.get_mut(i) {
            e.state = state;
            e.status.clear();
            e.views.clear();
            e.decorations.clear();
        }
        self.rebuild_decorations();
        if self.plugins.active_view.as_ref().is_some_and(|(p, _)| *p == i) {
            self.plugins.active_view = None;
            if self.sidebar_view == SidebarView::Plugin {
                self.sidebar_view = SidebarView::Explorer;
            }
        }
        self.needs_render = true;
    }

    /// Bookkeeping after a call into a plugin: slow-call warning, failures.
    fn after_call(&mut self, i: usize, status: YaceStatus, took: Duration, what: &str) {
        let Some(e) = self.plugins.entries.get_mut(i) else { return };
        let name = e.name().to_string();
        if took > SLOW_WARNING && !e.slow_warned {
            e.slow_warned = true;
            e.push_log(format!("{what} took {:.0} ms", took.as_secs_f64() * 1e3));
            self.notify_sticky(
                Severity::Warning,
                format!(
                    "Plugin {name} blocked the editor for {:.0} ms ({what}). \"Extensions: Show Performance\" has details; \"Extensions: Disable\" turns it off.",
                    took.as_secs_f64() * 1e3
                ),
            );
        }
        let Some(e) = self.plugins.entries.get_mut(i) else { return };
        if status == YACE_E_PANIC || (status != YACE_OK && status != YACE_E_UNSUPPORTED && what == "activate") {
            e.failures += 1;
        }
        if status == YACE_E_PANIC {
            let failures = e.failures;
            self.notify(
                Severity::Error,
                format!("Plugin {name} crashed in {what} (recovered; see Extensions: Show Plugin Log)"),
            );
            if failures >= MAX_FAILURES {
                self.stop_plugin(i, PluginState::Failed(format!("stopped after {failures} crashes")));
                self.notify_sticky(Severity::Error, format!("Plugin {name} was stopped after repeated crashes"));
            }
        }
    }

    // ------------------------------------------------------------------
    // Commands

    pub(crate) fn run_plugin_command(&mut self, id: &str, kind: PluginCmd, args: Option<Value>) {
        match kind {
            PluginCmd::ShowView(i, view) => self.show_plugin_view(i, &view),
            PluginCmd::Plugin(i) => {
                if self.plugins.entries.get(i).is_some_and(|e| e.state == PluginState::Inactive) {
                    self.activate_plugins_on(&format!("onCommand:{id}"));
                    self.activate_plugin(i);
                }
                let state = self.plugins.entries.get(i).map(|e| e.state.clone());
                match state {
                    Some(PluginState::Active) => {}
                    Some(s) => {
                        let name = self.plugins.entries[i].name().to_string();
                        self.notify(Severity::Warning, format!("Plugin {name} is {}", s.label()));
                        return;
                    }
                    None => return,
                }
                let args_text = args.map(|a| a.to_string()).unwrap_or_default();
                let Some((status, took)) = plugins::call(self, i, |l| l.command(id, &args_text)) else {
                    // The plugin is busy (it called back into the editor).
                    self.plugins.deferred.push((id.to_string(), serde_json::from_str(&args_text).ok(), None));
                    return;
                };
                self.after_call(i, status, took, id);
                if status != YACE_OK && status != YACE_E_PANIC {
                    let name = self.plugins.entries[i].name().to_string();
                    let detail = self.plugins.entries[i].log.back().cloned().unwrap_or_default();
                    let what = if status == YACE_E_UNSUPPORTED {
                        "does not handle it".to_string()
                    } else {
                        format!("failed ({})", yace_plugin_host::status_name(status))
                    };
                    self.notify(Severity::Error, format!("{id}: plugin {name} {what}. {detail}"));
                }
                self.run_deferred_plugin_commands();
            }
        }
    }

    pub(crate) fn run_deferred_plugin_commands(&mut self) {
        for _ in 0..16 {
            let todo = std::mem::take(&mut self.plugins.deferred);
            if todo.is_empty() {
                return;
            }
            for (id, args, from) in todo {
                if let Err(e) = self.run_command(&id, args.as_ref()) {
                    // The asking plugin sees why in its log.
                    if let Some(i) = from {
                        self.plugin_log(i, yace_plugin_api::YACE_LEVEL_ERROR, format!("executeCommand {id}: {e}"));
                    }
                    self.report_command_error(&id, &e, None);
                }
            }
        }
    }

    // ------------------------------------------------------------------
    // Events (spec §18.4)

    /// Queue an event for subscribed plugins (and activate on it).
    pub(crate) fn plugin_event(&mut self, kind: &str, mut payload: Value) {
        if self.plugins.entries.is_empty() || !self.plugins.wants(kind) {
            return;
        }
        payload["type"] = json!(kind);
        if matches!(kind, "selectionChanged" | "documentChanged") {
            // Coalesced: only the latest, delivered after a short pause.
            self.plugins.pending.insert(kind.to_string(), payload);
            self.plugins.pending_deadline.get_or_insert(Instant::now() + COALESCE);
            return;
        }
        self.activate_plugins_on(&format!("onEvent:{kind}"));
        self.plugins.queue.push((kind.to_string(), payload));
    }

    /// Deliver queued events; coalesced ones once their delay passed.
    pub(crate) fn deliver_plugin_events(&mut self, now: Instant) {
        if self.plugins.pending_deadline.is_some_and(|d| now >= d) {
            self.plugins.pending_deadline = None;
            let pending = std::mem::take(&mut self.plugins.pending);
            for (k, v) in pending {
                self.activate_plugins_on(&format!("onEvent:{k}"));
                self.plugins.queue.push((k, v));
            }
        }
        if self.plugins.queue.is_empty() {
            return;
        }
        let queue = std::mem::take(&mut self.plugins.queue);
        for (kind, payload) in queue {
            let text = payload.to_string();
            let targets: Vec<usize> = self
                .plugins
                .entries
                .iter()
                .enumerate()
                .filter(|(_, e)| e.state == PluginState::Active)
                .filter(|(_, e)| e.manifest.as_ref().is_some_and(|m| m.events.contains(&kind)))
                .map(|(i, _)| i)
                .collect();
            for i in targets {
                if let Some(took) = plugins::call(self, i, |l| l.event(&text)) {
                    self.after_call(i, YACE_OK, took, &format!("event {kind}"));
                }
            }
        }
        self.run_deferred_plugin_commands();
    }

    /// Compare observable state with the last input and emit events for
    /// what changed (cheap; skipped when no plugin is installed).
    pub(crate) fn observe_for_plugins(&mut self) {
        if self.plugins.entries.is_empty() {
            return;
        }
        let active = self.active().and_then(|(_, _, d)| self.docs.doc(d).map(|doc| (d, doc.version())));
        let selection = self.active().and_then(|(g, v, d)| {
            let r = self.layout.group(g)?.views.get(v)?.sel.primary();
            Some((d, r.anchor, r.head))
        });
        let mut docs = self.docs.ids();
        docs.sort_unstable();
        let mut langs: Vec<String> = docs.iter().map(|&d| self.syntax.language(d).id.to_string()).collect();
        langs.sort();
        langs.dedup();
        let now = Observed {
            active,
            selection,
            roots: self.workspace.roots().iter().map(|r| r.id.0).collect(),
            theme: self.theme.id.to_string(),
            terminals: self.panel.tabs.iter().map(|t| t.id).collect(),
            languages: langs,
            docs,
        };
        if now == self.plugins.observed {
            return;
        }
        let before = std::mem::replace(&mut self.plugins.observed, now.clone());
        if now.active.map(|a| a.0) != before.active.map(|a| a.0) {
            self.plugin_event("activeDocumentChanged", json!({ "doc": now.active.map(|a| a.0) }));
        } else if let (Some(a), Some(b)) = (now.active, before.active)
            && a.1 != b.1
        {
            self.plugin_event("documentChanged", json!({ "doc": a.0, "version": a.1 }));
        }
        if now.selection != before.selection
            && let Some((d, anchor, head)) = now.selection
        {
            self.plugin_event("selectionChanged", json!({ "doc": d, "anchor": anchor, "head": head }));
        }
        for r in now.roots.iter().filter(|r| !before.roots.contains(r)) {
            self.plugin_event("rootAdded", json!({ "root": r }));
        }
        for r in before.roots.iter().filter(|r| !now.roots.contains(r)) {
            self.plugin_event("rootRemoved", json!({ "root": r }));
        }
        if now.theme != before.theme && !before.theme.is_empty() {
            self.plugin_event("themeChanged", json!({ "theme": now.theme }));
        }
        for t in now.terminals.iter().filter(|t| !before.terminals.contains(t)) {
            self.plugin_event("terminalCreated", json!({ "terminal": t }));
        }
        for t in before.terminals.iter().filter(|t| !now.terminals.contains(t)) {
            self.plugin_event("terminalExited", json!({ "terminal": t }));
        }
        for d in now.docs.iter().filter(|d| !before.docs.contains(d)) {
            let path = self.docs.doc(*d).and_then(|x| x.path()).map(|p| p.display().to_string());
            self.plugin_event("documentOpened", json!({ "doc": d, "path": path }));
        }
        for d in before.docs.iter().filter(|d| !now.docs.contains(d)) {
            self.plugin_event("documentClosed", json!({ "doc": d }));
        }
    }

    /// Messages posted by plugin jobs.
    pub fn on_plugin_message(&mut self, msg: plugins::PluginMsg) {
        if self.plugins.entries.get(msg.plugin).is_none_or(|e| e.state != PluginState::Active) {
            return;
        }
        if let Some(took) = plugins::call(self, msg.plugin, |l| l.message(&msg.data)) {
            self.after_call(msg.plugin, YACE_OK, took, "message");
        }
        self.run_deferred_plugin_commands();
        self.needs_render = true;
    }

    /// Process messages already waiting (tests; the runtime forwards them).
    pub fn drain_plugin_messages(&mut self) {
        for m in self.plugins.try_messages() {
            self.on_plugin_message(m);
        }
    }

    // ------------------------------------------------------------------
    // Views (spec §18.12)

    pub(crate) fn show_plugin_view(&mut self, i: usize, view: &str) {
        self.activate_plugins_on(&format!("onView:{view}"));
        if self.plugins.entries.get(i).is_some_and(|e| e.state == PluginState::Inactive) {
            self.activate_plugin(i);
        }
        self.plugins.active_view = Some((i, view.to_string()));
        self.sidebar_view = SidebarView::Plugin;
        self.sidebar_visible = true;
        self.focus_changed(Focus::Explorer);
        self.needs_render = true;
    }

    /// Rows of the active view, flattened with expansion applied:
    /// (depth, node, has children, expanded).
    pub fn plugin_view_rows(&self) -> Vec<(usize, ViewNode, bool, bool)> {
        let Some((i, view)) = &self.plugins.active_view else { return Vec::new() };
        let Some(items) = self.plugins.entries.get(*i).and_then(|e| e.views.get(view)) else { return Vec::new() };
        let ui = self.plugins.view_ui.get(&(*i, view.clone()));
        let mut out = Vec::new();
        fn walk(nodes: &[ViewNode], depth: usize, ui: Option<&ViewUi>, out: &mut Vec<(usize, ViewNode, bool, bool)>) {
            for n in nodes {
                let has = !n.children.is_empty();
                let open = has
                    && match ui {
                        Some(u) if u.collapsed.contains(&n.id) => false,
                        Some(u) if u.expanded.contains(&n.id) => true,
                        _ => n.expanded,
                    };
                let mut shown = n.clone();
                shown.children.clear();
                out.push((depth, shown, has, open));
                if open {
                    walk(&n.children, depth + 1, ui, out);
                }
            }
        }
        walk(items, 0, ui, &mut out);
        out
    }

    /// Status bar items from plugins: (left side, right side), highest
    /// priority first, each with the command a click runs (the one the
    /// plugin set, else the manifest's).
    pub fn plugin_status_items(&self) -> (Vec<PluginStatusItem>, Vec<PluginStatusItem>) {
        let mut items: Vec<(bool, i32, PluginStatusItem)> = Vec::new();
        for e in &self.plugins.entries {
            let Some(m) = &e.manifest else { continue };
            for decl in &m.status_items {
                let declared = Some(decl.command.clone()).filter(|c| !c.is_empty());
                let (text, command) = match e.status.get(&decl.id) {
                    Some(c) => (c.text.clone(), Some(c.command.clone()).filter(|c| !c.is_empty()).or(declared)),
                    None if e.state.contributes() && !decl.text.is_empty() => (decl.text.clone(), declared),
                    None => continue,
                };
                if !text.is_empty() {
                    items.push((decl.alignment == "left", decl.priority, PluginStatusItem { text, command }));
                }
            }
        }
        items.sort_by_key(|i| std::cmp::Reverse(i.1));
        let (left, right): (Vec<_>, Vec<_>) = items.into_iter().partition(|i| i.0);
        (left.into_iter().map(|i| i.2).collect(), right.into_iter().map(|i| i.2).collect())
    }

    /// Status bar texts from plugins (left side, right side).
    pub fn plugin_status_texts(&self) -> (Vec<String>, Vec<String>) {
        let (l, r) = self.plugin_status_items();
        (l.into_iter().map(|i| i.text).collect(), r.into_iter().map(|i| i.text).collect())
    }

    pub fn plugin_view_title(&self) -> String {
        let Some((i, view)) = &self.plugins.active_view else { return String::new() };
        self.plugins
            .entries
            .get(*i)
            .and_then(|e| e.manifest.as_ref())
            .and_then(|m| m.views.iter().find(|v| v.id == *view))
            .map(|v| v.title.to_uppercase())
            .unwrap_or_default()
    }

    pub fn plugin_view_empty_text(&self) -> String {
        let Some((i, view)) = &self.plugins.active_view else { return String::new() };
        let Some(e) = self.plugins.entries.get(*i) else { return String::new() };
        match &e.state {
            PluginState::Active => e
                .manifest
                .as_ref()
                .and_then(|m| m.views.iter().find(|v| v.id == *view))
                .map(|v| if v.empty_text.is_empty() { "Nothing to show".into() } else { v.empty_text.clone() })
                .unwrap_or_default(),
            s => format!("Plugin {} is {}", e.name(), s.label()),
        }
    }

    pub fn plugin_view_ui(&self) -> ViewUi {
        self.plugins
            .active_view
            .as_ref()
            .and_then(|(i, v)| self.plugins.view_ui.get(&(*i, v.clone())))
            .cloned()
            .unwrap_or_default()
    }

    fn view_ui_mut(&mut self) -> Option<&mut ViewUi> {
        let key = self.plugins.active_view.clone()?;
        Some(self.plugins.view_ui.entry(key).or_default())
    }

    /// Keys while a plugin view has focus. Returns true if handled.
    pub(crate) fn plugin_view_key(&mut self, key: yace_keymap::KeyPress) -> bool {
        use yace_keymap::KeyCode as K;
        if !key.mods.is_empty() {
            return false;
        }
        let rows = self.plugin_view_rows();
        let n = rows.len();
        let Some(ui) = self.view_ui_mut() else { return false };
        let sel = ui.selected.min(n.saturating_sub(1));
        match key.code {
            K::Up => ui.selected = sel.saturating_sub(1),
            K::Down => ui.selected = (sel + 1).min(n.saturating_sub(1)),
            K::PageUp => ui.selected = sel.saturating_sub(10),
            K::PageDown => ui.selected = (sel + 10).min(n.saturating_sub(1)),
            K::Home => ui.selected = 0,
            K::End => ui.selected = n.saturating_sub(1),
            K::Left => {
                if let Some((_, node, has, open)) = rows.get(sel) {
                    if *has && *open {
                        ui.collapsed.insert(node.id.clone());
                        ui.expanded.remove(&node.id);
                    } else if let Some(depth) = rows.get(sel).map(|r| r.0).filter(|d| *d > 0) {
                        // Go to the parent.
                        if let Some(p) = (0..sel).rev().find(|&k| rows[k].0 < depth) {
                            ui.selected = p;
                        }
                    }
                }
            }
            K::Right => {
                if let Some((_, node, has, open)) = rows.get(sel)
                    && *has
                    && !*open
                {
                    ui.expanded.insert(node.id.clone());
                    ui.collapsed.remove(&node.id);
                }
            }
            K::Enter | K::Space => {
                self.open_plugin_view_row(sel);
            }
            _ => return false,
        }
        self.needs_render = true;
        true
    }

    /// Open (activate) a row: toggle a folder, run its command or tell the
    /// plugin.
    pub(crate) fn open_plugin_view_row(&mut self, row: usize) {
        let rows = self.plugin_view_rows();
        let Some((_, node, has, open)) = rows.get(row).cloned() else { return };
        let Some((i, view)) = self.plugins.active_view.clone() else { return };
        if let Some(ui) = self.view_ui_mut() {
            ui.selected = row;
        }
        if !node.command.is_empty() {
            self.execute(&node.command, node.args.as_ref());
            return;
        }
        if has {
            if let Some(ui) = self.view_ui_mut() {
                if open {
                    ui.collapsed.insert(node.id.clone());
                    ui.expanded.remove(&node.id);
                } else {
                    ui.expanded.insert(node.id.clone());
                    ui.collapsed.remove(&node.id);
                }
            }
            return;
        }
        if let Some((status, took)) = plugins::call(self, i, |l| l.view_action(&view, &node.id, "open")) {
            self.after_call(i, status, took, "view action");
        }
        self.run_deferred_plugin_commands();
    }

    // ------------------------------------------------------------------
    // Quick Open providers

    /// The plugin provider whose prefix starts `text`: (plugin, prefix).
    pub(crate) fn plugin_quick_open_provider(&self, text: &str) -> Option<(usize, String)> {
        let mut best: Option<(usize, String)> = None;
        for (i, e) in self.plugins.entries.iter().enumerate() {
            if !e.state.contributes() {
                continue;
            }
            for q in e.manifest.iter().flat_map(|m| &m.quick_open) {
                if text.starts_with(q.prefix.as_str()) && best.as_ref().is_none_or(|(_, p)| q.prefix.len() > p.len()) {
                    best = Some((i, q.prefix.clone()));
                }
            }
        }
        best
    }

    /// Providers for Quick Open's `?` help: (prefix, title).
    pub(crate) fn plugin_quick_open_help(&self) -> Vec<(String, String)> {
        self.plugins
            .entries
            .iter()
            .filter(|e| e.state.contributes())
            .flat_map(|e| {
                e.manifest.iter().flat_map(|m| m.quick_open.iter().map(|q| (q.prefix.clone(), q.title.clone())))
            })
            .collect()
    }

    /// Ask the plugin for results; they arrive via `quick_open_results`.
    pub(crate) fn plugin_quick_open(&mut self, i: usize, prefix: &str, query: &str) {
        self.activate_plugins_on(&format!("onQuickOpen:{prefix}"));
        if self.plugins.entries.get(i).is_some_and(|e| e.state == PluginState::Inactive) {
            self.activate_plugin(i);
        }
        self.plugins.next_request += 1;
        let req = self.plugins.next_request;
        self.plugins.quick_open = Some((req, i, prefix.to_string()));
        if self.plugins.entries.get(i).is_none_or(|e| e.state != PluginState::Active) {
            let label = self.plugins.entries.get(i).map(|e| format!("Plugin {} is {}", e.name(), e.state.label()));
            self.plugin_quick_open_results(i, req, Vec::new());
            if let Some(l) = label {
                self.quick_open_plugin_note(&l);
            }
            return;
        }
        if let Some((status, took)) = plugins::call(self, i, |l| l.quick_open(prefix, query, req)) {
            self.after_call(i, status, took, "quick open");
        }
    }

    // ------------------------------------------------------------------
    // Host-function implementations (called from `plugins.rs`)

    pub(crate) fn plugin_log(&mut self, i: usize, level: u32, msg: String) {
        let lvl = match level {
            0 => "debug",
            1 => "info",
            2 => "warning",
            _ => "error",
        };
        if let Some(e) = self.plugins.entries.get_mut(i) {
            let t = chrono_like_now();
            e.push_log(format!("{t} [{lvl}] {msg}"));
            tracing::debug!(plugin = %e.id(), "{msg}");
        }
    }

    pub(crate) fn plugin_notify(&mut self, i: usize, level: u32, msg: String) {
        let name = self.plugins.entries.get(i).map(|e| e.name().to_string()).unwrap_or_default();
        let sev = match level {
            0 | 1 => Severity::Info,
            2 => Severity::Warning,
            _ => Severity::Error,
        };
        self.notify(sev, format!("{name}: {msg}"));
    }

    pub(crate) fn plugin_setting(&mut self, i: usize, key: &str) -> Result<Value, String> {
        let e = self.plugins.entries.get(i).ok_or("unknown plugin")?;
        let m = e.manifest.as_ref().ok_or("unknown plugin")?;
        let decl = m.settings.get(key).ok_or_else(|| format!("no setting `{key}` declared in the manifest"))?.clone();
        let id = m.id.clone();
        let user = self.settings.plugin.get(&id).and_then(|t| t.get(key)).cloned();
        let value = match user {
            Some(v) => match decl.validate(&v) {
                Ok(()) => v,
                Err(why) => {
                    let msg = format!("plugin.\"{id}\".{key}: {why}; using the default");
                    if !self.config_errors.iter().any(|e| e.message == msg) {
                        self.report_config_errors(vec![yace_config::ConfigError {
                            origin: self.paths.user_settings().display().to_string(),
                            key: Some(format!("plugin.\"{id}\".{key}")),
                            message: msg,
                        }]);
                    }
                    decl.default.clone()
                }
            },
            None => decl.default.clone(),
        };
        serde_json::to_value(&value).map_err(|e| e.to_string())
    }

    pub(crate) fn plugin_roots_json(&self) -> Value {
        Value::Array(
            self.workspace
                .roots()
                .iter()
                .map(|r| {
                    json!({
                        "id": r.id.0,
                        "name": r.display_name(),
                        "path": r.path.display().to_string(),
                        "available": r.is_available(),
                    })
                })
                .collect(),
        )
    }

    pub(crate) fn plugin_active_document_json(&self) -> Value {
        let Some((g, v, d)) = self.active() else { return Value::Null };
        let Some(doc) = self.docs.doc(d) else { return Value::Null };
        let sels: Vec<Value> = self
            .layout
            .group(g)
            .map(|gr| gr.views[v].sel.ranges().iter().map(|r| json!({"anchor": r.anchor, "head": r.head})).collect())
            .unwrap_or_default();
        json!({
            "doc": d,
            "version": doc.version(),
            "path": doc.path().map(|p| p.display().to_string()),
            "language": self.syntax.language(d).id,
            "selections": sels,
        })
    }

    pub(crate) fn plugin_document_text(&self, doc: u64, start: u64, end: u64) -> Result<String, String> {
        let d = u32::try_from(doc).map_err(|_| "no such document".to_string())?;
        let rope = self.docs.doc(d).map(|x| x.rope()).ok_or("no such document")?;
        let len = rope.len_chars() as u64;
        let (a, b) = (start.min(len) as usize, end.min(len) as usize);
        Ok(rope.slice(a..b.max(a)).to_string())
    }

    pub(crate) fn plugin_apply_edits(
        &mut self,
        doc: u64,
        version: u64,
        edits: &[EditIn],
    ) -> Result<(), (YaceStatus, String)> {
        let d = u32::try_from(doc).map_err(|_| (YACE_E_NOT_FOUND, "no such document".to_string()))?;
        let (len, current) = match self.docs.doc(d) {
            Some(x) => (x.rope().len_chars() as u64, x.version()),
            None => return Err((YACE_E_NOT_FOUND, "no such document".into())),
        };
        if current != version {
            return Err((YACE_E_STALE, format!("the document is at version {current}, not {version}")));
        }
        let mut changes = Vec::with_capacity(edits.len());
        for e in edits {
            if e.start > e.end || e.end > len {
                return Err((
                    yace_plugin_api::YACE_E_INVALID,
                    format!("edit {}..{} is outside 0..{len}", e.start, e.end),
                ));
            }
            changes.push(Change::replace(e.start as usize, e.end as usize, e.text.clone()));
        }
        self.apply_doc_changes(d, ChangeSet::new(changes, len as usize));
        self.on_document_edited();
        Ok(())
    }

    pub(crate) fn plugin_open_file(&mut self, path: &str, line: u32, col: u32) -> Result<(), String> {
        let p = PathBuf::from(path);
        let p = if p.is_absolute() {
            p
        } else {
            self.workspace.roots().first().map(|r| r.path.join(&p)).ok_or("no workspace folder for a relative path")?
        };
        if !p.is_file() {
            return Err(format!("{} is not a file", p.display()));
        }
        self.open_file(&p, false);
        if line > 0 {
            self.set_cursor_line_col(line as usize - 1, col.saturating_sub(1) as usize);
            self.reveal_cursor = true;
        }
        self.focus_editor();
        Ok(())
    }

    fn declared(&self, i: usize, what: &str, id: &str) -> bool {
        let Some(m) = self.plugins.entries.get(i).and_then(|e| e.manifest.as_ref()) else { return false };
        match what {
            "status" => m.status_items.iter().any(|s| s.id == id),
            "view" => m.views.iter().any(|v| v.id == id),
            _ => false,
        }
    }

    pub(crate) fn plugin_set_status(&mut self, i: usize, id: &str, item: Option<StatusContent>) -> Result<(), String> {
        if !self.declared(i, "status", id) {
            return Err(format!("status item `{id}` is not declared in the manifest"));
        }
        let e = &mut self.plugins.entries[i];
        match item {
            Some(mut it) => {
                it.text = it.text.chars().filter(|c| !c.is_control()).take(80).collect();
                e.status.insert(id.to_string(), it);
            }
            None => {
                e.status.remove(id);
            }
        }
        self.needs_render = true;
        Ok(())
    }

    pub(crate) fn plugin_set_view(&mut self, i: usize, view: &str, items: Vec<ViewNode>) -> Result<(), String> {
        if !self.declared(i, "view", view) {
            return Err(format!("view `{view}` is not declared in the manifest"));
        }
        self.plugins.entries[i].views.insert(view.to_string(), items);
        self.needs_render = true;
        Ok(())
    }

    pub(crate) fn plugin_set_decorations(
        &mut self,
        i: usize,
        root: u64,
        items: Vec<DecorationIn>,
    ) -> Result<(), String> {
        if !self.workspace.roots().iter().any(|r| u64::from(r.id.0) == root) {
            return Err(format!("no workspace folder {root}"));
        }
        self.plugins.entries[i].decorations.insert(root, items);
        self.rebuild_decorations();
        Ok(())
    }

    /// Merge every plugin's decorations into a path → (badge, colour) map.
    pub(crate) fn rebuild_decorations(&mut self) {
        let mut map = std::collections::HashMap::new();
        for e in &self.plugins.entries {
            for (root, items) in &e.decorations {
                let Some(r) = self.workspace.roots().iter().find(|r| u64::from(r.id.0) == *root) else { continue };
                for d in items {
                    let p = PathBuf::from(&d.path);
                    let p = if p.is_absolute() { p } else { r.path.join(p) };
                    let badge: String = d.badge.chars().filter(|c| !c.is_control()).take(2).collect();
                    map.insert(crate::editor::path_key(&p), (badge, d.color.clone()));
                }
            }
        }
        self.plugin_decorations = map;
        self.needs_render = true;
    }

    pub(crate) fn plugin_quick_open_results(&mut self, i: usize, request: u64, items: Vec<QuickOpenIn>) {
        let Some((req, plugin, _)) = &self.plugins.quick_open else { return };
        if *req != request || *plugin != i {
            return; // a late answer for an older query
        }
        self.set_plugin_quick_open_items(items);
    }

    pub(crate) fn plugin_create_terminal(&mut self, _i: usize, spec: TerminalIn) -> Result<u64, String> {
        let root = spec.root.and_then(|r| self.workspace.roots().iter().find(|x| u64::from(x.id.0) == r).cloned());
        if spec.root.is_some() && root.is_none() {
            return Err("no such workspace folder".into());
        }
        let base = root.as_ref().map(|r| r.path.clone()).unwrap_or_else(|| self.terminal_target().1);
        let cwd = if spec.cwd.is_empty() {
            base
        } else {
            let p = PathBuf::from(&spec.cwd);
            if p.is_absolute() { p } else { base.join(p) }
        };
        if !cwd.is_dir() {
            return Err(format!("{} is not a folder", cwd.display()));
        }
        let startup = (!spec.command.is_empty()).then(|| {
            std::iter::once(spec.command.as_str())
                .chain(spec.args.iter().map(String::as_str))
                .map(shell_quote)
                .collect::<Vec<_>>()
                .join(" ")
        });
        let name = (!spec.name.is_empty()).then(|| spec.name.clone());
        let id = self.new_terminal_with(root.map(|r| r.id), cwd, name, startup);
        Ok(u64::from(id))
    }

    /// A workspace was opened or its folders changed.
    pub(crate) fn plugins_workspace_changed(&mut self, opened: bool) {
        if !self.plugins.discovered {
            return;
        }
        if opened {
            self.plugin_event("workspaceOpened", json!({ "name": self.workspace.name() }));
        }
        let roots: Vec<PathBuf> = self.workspace.roots().iter().map(|r| r.path.clone()).collect();
        let todo: Vec<usize> = self
            .plugins
            .entries
            .iter()
            .enumerate()
            .filter(|(_, e)| e.state == PluginState::Inactive)
            .filter(|(_, e)| {
                e.manifest.as_ref().is_some_and(|m| {
                    m.activation.iter().any(|a| {
                        a.strip_prefix("workspaceContains:")
                            .and_then(store_rel)
                            .is_some_and(|rel| roots.iter().any(|r| r.join(&rel).exists()))
                    })
                })
            })
            .map(|(i, _)| i)
            .collect();
        for i in todo {
            self.activate_plugin(i);
        }
    }

    /// Deactivate every plugin (clean exit).
    pub fn shutdown_plugins(&mut self) {
        for i in 0..self.plugins.entries.len() {
            if self.plugins.entries[i].state == PluginState::Active {
                self.stop_plugin(i, PluginState::Inactive);
            }
        }
        self.crash_marker_clear();
    }

    /// Languages and grammars plugins contribute. Languages are registered
    /// now (detection and editing rules); a grammar library is loaded only
    /// when a document of its language is open (hybrid grammars, D29).
    pub(crate) fn register_plugin_grammars(&mut self) {
        self.pending_grammars.clear();
        if self.plugins.safe_mode {
            return;
        }
        for (i, e) in self.plugins.entries.iter().enumerate() {
            if !matches!(e.state, PluginState::Inactive | PluginState::Active) {
                continue;
            }
            let Some(m) = &e.manifest else { continue };
            for l in &m.languages {
                let pair = |v: &[String]| (v.len() == 2).then(|| (v[0].clone(), v[1].clone()));
                yace_syntax::register_language(&yace_syntax::LanguageDef {
                    id: l.id.clone(),
                    name: l.name.clone(),
                    extensions: l.extensions.clone(),
                    filenames: l.filenames.clone(),
                    shebangs: l.shebangs.clone(),
                    line_comment: (!l.line_comment.is_empty()).then(|| l.line_comment.clone()),
                    block_comment: pair(&l.block_comment),
                    brackets: l
                        .brackets
                        .iter()
                        .filter_map(|b| Some((b.first()?.chars().next()?, b.get(1)?.chars().next()?)))
                        .collect(),
                });
            }
            for g in &m.grammars {
                if !yace_syntax::has_external_grammar(&g.language) {
                    self.pending_grammars.insert(g.language.clone(), (i, g.clone()));
                }
            }
        }
        // Languages already open (including ones detected before discovery).
        self.languages_seen.clear();
        self.check_new_languages();
    }

    /// For each language newly open: activate `onLanguage` plugins, load
    /// a packaged grammar, or explain once that it is not installed.
    pub(crate) fn check_new_languages(&mut self) {
        let langs: Vec<&'static yace_syntax::LanguageConfig> =
            self.docs.ids().into_iter().map(|d| self.syntax.language(d)).collect();
        for l in langs {
            if !self.languages_seen.insert(l.id.to_string()) {
                continue;
            }
            if !l.has_grammar() {
                if self.pending_grammars.contains_key(l.id) {
                    self.load_plugin_grammar(l.id);
                } else if l.grammar_is_packaged() && self.plugins.discovered && !self.plugins.safe_mode {
                    self.notify_sticky(
                        Severity::Info,
                        format!(
                            "{} highlighting comes in the YACE Grammars package. Build it with scripts/build_grammars.py (or download yace-grammars from the release), then run \"Extensions: Install from Package…\".",
                            l.name
                        ),
                    );
                }
            }
            if self.plugins.discovered {
                self.activate_plugins_on(&format!("onLanguage:{}", l.id));
            }
        }
    }

    fn load_plugin_grammar(&mut self, lang: &str) {
        let Some((i, g)) = self.pending_grammars.remove(lang) else { return };
        let Some(e) = self.plugins.entries.get(i) else { return };
        let (dir, id, name) = (e.dir.clone(), e.id().to_string(), e.name().to_string());
        let start = Instant::now();
        let result = (|| -> Result<(), String> {
            let rel = yace_plugin_host::manifest::safe_relative(&g.library).ok_or("unsafe grammar path")?;
            let path = dir.join(&rel);
            // The library must be what the user approved.
            let trust = TrustRecord::load(&self.plugins_dir());
            let want = trust.plugins.get(&id).and_then(|t| t.files.get(&g.library)).ok_or("not approved")?;
            let have = store::sha256_file(&path).map_err(|e| e.to_string())?;
            if &have != want {
                return Err(format!("{} changed on disk since it was installed", g.library));
            }
            let f = yace_plugin_host::open_grammar(&path, &g.symbol())?;
            let language = plugins::grammar_language(f);
            let read = |files: &[String]| -> Result<String, String> {
                let mut out = String::new();
                for q in files {
                    let p = store_safe(&dir, q).ok_or("unsafe query path")?;
                    out.push_str(&std::fs::read_to_string(&p).map_err(|e| format!("{q}: {e}"))?);
                    out.push('\n');
                }
                Ok(out)
            };
            yace_syntax::register_grammar(lang, language, &read(&g.highlights)?, &read(&g.injections)?)
        })();
        match result {
            Ok(()) => {
                self.plugin_log(
                    i,
                    yace_plugin_api::YACE_LEVEL_DEBUG,
                    format!("grammar {lang} loaded in {:.1} ms", start.elapsed().as_secs_f64() * 1e3),
                );
                self.syntax.reattach(lang);
                self.sync_documents();
            }
            Err(why) => {
                self.plugin_log(i, yace_plugin_api::YACE_LEVEL_ERROR, format!("grammar {lang}: {why}"));
                self.notify_sticky(Severity::Error, format!("{name}: the {lang} grammar could not be loaded: {why}"));
            }
        }
    }

    // ------------------------------------------------------------------
    // Crash diagnosis (spec §18.9)

    fn crash_marker_path(&self) -> PathBuf {
        self.paths.state_dir.join("running").join(format!("{}.plugins", std::process::id()))
    }

    fn crash_marker_add(&mut self, id: &str) {
        if !self.plugins.activated.iter().any(|x| x == id) {
            self.plugins.activated.push(id.to_string());
        }
        let text = self.plugins.activated.join("\n");
        let _ = yace_config::atomic_write(&self.crash_marker_path(), text.as_bytes());
    }

    /// Clean exit: no plugin is suspected of anything.
    pub fn crash_marker_clear(&mut self) {
        let _ = std::fs::remove_file(self.crash_marker_path());
    }

    /// At startup: markers left by processes that no longer exist mean
    /// YACE ended abnormally while those plugins were active.
    pub fn check_crash_markers(&mut self) {
        let dir = self.paths.state_dir.join("running");
        let Ok(rd) = std::fs::read_dir(&dir) else { return };
        let mut suspects: Vec<String> = Vec::new();
        for e in rd.flatten() {
            let p = e.path();
            let Some(pid) = p.file_stem().and_then(|s| s.to_str()).and_then(|s| s.parse::<u32>().ok()) else {
                continue;
            };
            if pid == std::process::id() || Path::new(&format!("/proc/{pid}")).exists() {
                continue;
            }
            if let Ok(t) = std::fs::read_to_string(&p) {
                suspects.extend(t.lines().filter(|l| !l.trim().is_empty()).map(str::to_string));
            }
            let _ = std::fs::remove_file(&p);
        }
        suspects.sort();
        suspects.dedup();
        if !suspects.is_empty() {
            self.notify_sticky(
                Severity::Warning,
                format!(
                    "YACE closed unexpectedly last time while these plugins were active: {}. If it happens again, disable them (Extensions: Disable) or start with --safe-mode.",
                    suspects.join(", ")
                ),
            );
        }
    }
}

/// A plugin-relative file, refusing paths that leave the plugin folder.
fn store_safe(dir: &Path, rel: &str) -> Option<PathBuf> {
    yace_plugin_host::manifest::safe_relative(rel).map(|r| dir.join(r))
}

fn store_rel(rel: &str) -> Option<PathBuf> {
    yace_plugin_host::manifest::safe_relative(rel)
}

/// Quote a word for a POSIX shell (only when needed).
pub fn shell_quote(s: &str) -> String {
    if !s.is_empty() && s.chars().all(|c| c.is_ascii_alphanumeric() || "-_./=:@%+,".contains(c)) {
        s.to_string()
    } else {
        format!("'{}'", s.replace('\'', "'\\''"))
    }
}

fn chrono_like_now() -> String {
    let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let (h, m, s) = ((secs / 3600) % 24, (secs / 60) % 60, secs % 60);
    format!("{h:02}:{m:02}:{s:02}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shell_quoting() {
        assert_eq!(shell_quote("npm"), "npm");
        assert_eq!(shell_quote("run-script:dev"), "run-script:dev");
        assert_eq!(shell_quote("a b"), "'a b'");
        assert_eq!(shell_quote("it's"), "'it'\\''s'");
        assert_eq!(shell_quote(""), "''");
    }
}

/// A plugin's status bar item as shown.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluginStatusItem {
    pub text: String,
    pub command: Option<String>,
}
