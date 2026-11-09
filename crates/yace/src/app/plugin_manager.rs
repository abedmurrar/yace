//! The plugin manager: "Extensions: …" commands.

use std::path::PathBuf;

use yace_plugin_host::{TrustRecord, registry, store};

use super::{App, Effect, Severity};
use crate::{
    actions::{ConfirmAction, PickerAction, PromptAction},
    overlay::{Confirm, Overlay, Picker, PickerItem},
    plugins::PluginState,
};

/// What can be done with one plugin, from the manager picker.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PluginOp {
    Details,
    Enable,
    Disable,
    EnableInWorkspace,
    DisableInWorkspace,
    Trust,
    Settings,
    Log,
    Uninstall,
}

impl PluginOp {
    const ALL: [PluginOp; 9] = [
        PluginOp::Details,
        PluginOp::Enable,
        PluginOp::Disable,
        PluginOp::EnableInWorkspace,
        PluginOp::DisableInWorkspace,
        PluginOp::Trust,
        PluginOp::Settings,
        PluginOp::Log,
        PluginOp::Uninstall,
    ];
    fn label(self) -> &'static str {
        match self {
            PluginOp::Details => "Show Details",
            PluginOp::Enable => "Enable",
            PluginOp::Disable => "Disable",
            PluginOp::EnableInWorkspace => "Enable in This Workspace",
            PluginOp::DisableInWorkspace => "Disable in This Workspace",
            PluginOp::Trust => "Approve Its Code (Trust)",
            PluginOp::Settings => "Open Plugin Settings",
            PluginOp::Log => "Show Plugin Log",
            PluginOp::Uninstall => "Uninstall",
        }
    }
}

impl App {
    /// Extensions: List Installed — then an action picker for the chosen one.
    pub(crate) fn plugins_list(&mut self) {
        self.plugin_picker("Installed Plugins", |_| true, PickerAction::PluginList);
    }

    fn plugin_picker(&mut self, title: &str, keep: impl Fn(&PluginState) -> bool, action: PickerAction) {
        let mut items = Vec::new();
        self.plugin_pick.clear();
        for e in &self.plugins.entries {
            if !keep(&e.state) {
                continue;
            }
            let version = e.manifest.as_ref().map(|m| m.version.clone()).unwrap_or_default();
            items.push(PickerItem::new(
                format!("{} {version}", e.name()),
                format!("{} · {}", e.id(), e.state.label()),
                "",
                self.plugin_pick.len(),
            ));
            self.plugin_pick.push(e.id().to_string());
        }
        if items.is_empty() {
            let msg = if self.plugins.entries.is_empty() {
                format!("No plugins installed (folder: {})", self.plugins_dir().display())
            } else {
                "No plugins in that state".into()
            };
            self.notify(Severity::Info, msg);
            return;
        }
        let mut p = Picker::new(title, items, action);
        if self.plugins.safe_mode {
            p.title = format!("{title} — safe mode: no plugin code is loaded");
        }
        self.open_overlay(Overlay::Picker(p));
    }

    pub(crate) fn plugins_pick(&mut self, action: PickerAction, data: usize) {
        let Some(id) = self.plugin_pick.get(data).cloned() else { return };
        match action {
            PickerAction::PluginList => {
                let items =
                    PluginOp::ALL.iter().enumerate().map(|(i, op)| PickerItem::new(op.label(), "", "", i)).collect();
                let name =
                    self.plugins.find(&id).map(|i| self.plugins.entries[i].name().to_string()).unwrap_or_default();
                self.open_overlay(Overlay::Picker(Picker::new(name, items, PickerAction::PluginOp(id))));
            }
            PickerAction::PluginEnable => self.plugin_op(&id, PluginOp::Enable),
            PickerAction::PluginDisable => self.plugin_op(&id, PluginOp::Disable),
            PickerAction::PluginUninstall => self.plugin_op(&id, PluginOp::Uninstall),
            _ => {}
        }
    }

    pub(crate) fn plugins_enable_picker(&mut self) {
        self.plugin_picker("Enable Plugin", |s| *s == PluginState::Disabled, PickerAction::PluginEnable);
    }
    pub(crate) fn plugins_disable_picker(&mut self) {
        self.plugin_picker("Disable Plugin", |s| *s != PluginState::Disabled, PickerAction::PluginDisable);
    }
    pub(crate) fn plugins_uninstall_picker(&mut self) {
        self.plugin_picker("Uninstall Plugin", |_| true, PickerAction::PluginUninstall);
    }

    pub(crate) fn plugin_op_pick(&mut self, id: String, data: usize) {
        if let Some(op) = PluginOp::ALL.get(data).copied() {
            self.plugin_op(&id, op);
        }
    }

    pub(crate) fn plugin_op(&mut self, id: &str, op: PluginOp) {
        let Some(i) = self.plugins.find(id) else { return };
        match op {
            PluginOp::Details => self.plugin_details(i),
            PluginOp::Log => self.plugin_show_log(i),
            PluginOp::Settings => self.plugin_open_settings(i),
            PluginOp::Enable | PluginOp::Disable => {
                let disable = op == PluginOp::Disable;
                let path = self.paths.user_settings();
                let mut list = self.settings.plugins.disabled.clone();
                list.retain(|x| x != id);
                if disable {
                    list.push(id.to_string());
                }
                let arr: yace_config::toml_edit::Array = list.iter().map(|s| s.as_str()).collect();
                if let Err(e) = yace_config::edit_setting(&path, "plugins", "disabled", Some(arr.into())) {
                    self.notify(Severity::Error, format!("Could not save: {e}"));
                    return;
                }
                // Workspace overrides would hide the change: drop them.
                self.plugin_ws_enabled.retain(|x| x != id);
                self.plugin_ws_disabled.retain(|x| x != id);
                self.reload_settings();
                self.after_enablement_change(i, disable);
            }
            PluginOp::EnableInWorkspace | PluginOp::DisableInWorkspace => {
                let disable = op == PluginOp::DisableInWorkspace;
                self.plugin_ws_enabled.retain(|x| x != id);
                self.plugin_ws_disabled.retain(|x| x != id);
                if disable {
                    self.plugin_ws_disabled.push(id.to_string());
                } else {
                    self.plugin_ws_enabled.push(id.to_string());
                }
                self.save_session();
                self.after_enablement_change(i, disable);
            }
            PluginOp::Trust => {
                let dir = self.plugins.entries[i].dir.clone();
                let plan = match store::plan_install(&dir, &self.plugins_dir(), Self::yace_version()) {
                    Ok(p) => p,
                    Err(e) => {
                        self.notify(Severity::Error, e);
                        return;
                    }
                };
                let mut message = crate::plugin_cli::trust_summary(&plan);
                message.insert(0, "Approve the code of this plugin, as it is now?".into());
                self.open_overlay(Overlay::Confirm(Confirm {
                    title: "Trust Plugin".into(),
                    message,
                    buttons: vec![
                        ("Trust and Load".into(), ConfirmAction::TrustPlugin(dir)),
                        ("Cancel".into(), ConfirmAction::Nothing),
                    ],
                    selected: 1,
                    cancel: ConfirmAction::Nothing,
                }));
            }
            PluginOp::Uninstall => {
                let name = self.plugins.entries[i].name().to_string();
                self.open_overlay(Overlay::Confirm(Confirm {
                    title: "Uninstall Plugin".into(),
                    message: vec![format!("Uninstall {name} ({id})?"), "Its folder and approval are removed.".into()],
                    buttons: vec![
                        ("Uninstall".into(), ConfirmAction::UninstallPlugin(id.to_string())),
                        ("Cancel".into(), ConfirmAction::Nothing),
                    ],
                    selected: 1,
                    cancel: ConfirmAction::Nothing,
                }));
            }
        }
    }

    fn after_enablement_change(&mut self, i: usize, disabled: bool) {
        let name = self.plugins.entries[i].name().to_string();
        if disabled && self.plugins.entries[i].state == PluginState::Active {
            self.stop_plugin(i, PluginState::Disabled);
        }
        self.discover_plugins();
        self.notify(Severity::Info, format!("{name} {}", if disabled { "disabled" } else { "enabled" }));
    }

    pub(crate) fn trust_plugin(&mut self, dir: PathBuf) {
        match store::trust_in_place(&dir, &self.plugins_dir(), Self::yace_version()) {
            Ok(m) => {
                self.notify(Severity::Info, format!("{} approved", m.name));
                self.discover_plugins();
            }
            Err(e) => self.notify(Severity::Error, e),
        }
    }

    pub(crate) fn uninstall_plugin(&mut self, id: &str) {
        if let Some(i) = self.plugins.find(id) {
            self.stop_plugin(i, PluginState::Disabled);
        }
        match store::uninstall(&self.plugins_dir(), id) {
            Ok(()) => {
                self.notify(Severity::Info, format!("Uninstalled {id}"));
                self.discover_plugins();
            }
            Err(e) => self.notify(Severity::Error, e),
        }
    }

    /// Extensions: Install from Directory… / from Package…
    pub(crate) fn plugins_install_prompt(&mut self, package: bool) {
        let initial = self.cwd_with_slash();
        let (title, hint) = if package {
            ("Install Plugin from Package", "Path of a plugin .tar.gz package (Tab completes)")
        } else {
            ("Install Plugin from Directory", "Path of a folder with yace-plugin.toml (Tab completes)")
        };
        self.prompt(title, hint, initial, PromptAction::InstallPlugin, true);
    }

    /// Validate the source and ask for confirmation with what it will do.
    pub(crate) fn plugins_install_from(&mut self, text: &str) -> Result<(), String> {
        let path = yace_workspace::paths::expand_tilde(text);
        let plan = store::plan_install(&path, &self.plugins_dir(), Self::yace_version())?;
        self.confirm_install(plan);
        Ok(())
    }

    /// Extensions: Browse Plugins… — download the plugin index (the only
    /// network use, and only on request), then list it.
    pub(crate) fn plugins_browse(&mut self) {
        if self.plugin_fetching {
            self.notify(Severity::Info, "Still downloading…");
            return;
        }
        self.plugin_fetching = true;
        self.notify(Severity::Info, "Getting the plugin list…");
        self.effects.push(Effect::FetchPluginIndex(self.plugin_index_url.clone()));
    }

    pub(crate) fn on_plugin_index(&mut self, result: Result<registry::Index, String>) {
        self.plugin_fetching = false;
        let index = match result {
            Ok(i) => i,
            Err(e) => {
                self.notify(Severity::Error, format!("Could not get the plugin list: {e}"));
                return;
            }
        };
        if index.plugins.is_empty() {
            self.notify(Severity::Info, "The plugin index lists no plugins yet");
            return;
        }
        let version = Self::yace_version();
        let items = index
            .plugins
            .iter()
            .enumerate()
            .map(|(k, e)| {
                let installed = self.installed_version(&e.id);
                let state = if let Err(why) = e.compatible(version) {
                    why
                } else {
                    match installed {
                        Some(v) if v == e.version => "installed".into(),
                        Some(v) => format!("installed {v}, update to {}", e.version),
                        None if e.code => "native code".into(),
                        None => String::new(),
                    }
                };
                let mut detail = e.description.clone();
                if !e.publisher.is_empty() {
                    detail = format!("{detail} · {}", e.publisher);
                }
                PickerItem::new(format!("{} {}", e.name, e.version), detail, state, k)
            })
            .collect();
        self.plugin_index = index.plugins;
        self.open_overlay(Overlay::Picker(Picker::new("Browse Plugins", items, PickerAction::PluginBrowse)));
    }

    fn installed_version(&self, id: &str) -> Option<String> {
        let i = self.plugins.find(id)?;
        self.plugins.entries[i].manifest.as_ref().map(|m| m.version.clone())
    }

    pub(crate) fn plugin_browse_pick(&mut self, data: usize) {
        let Some(entry) = self.plugin_index.get(data).cloned() else { return };
        if let Err(why) = entry.compatible(Self::yace_version()) {
            self.notify(Severity::Error, format!("{}: {why}", entry.name));
            return;
        }
        if self.installed_version(&entry.id).as_deref() == Some(entry.version.as_str()) {
            self.notify(Severity::Info, format!("{} {} is already installed", entry.name, entry.version));
            return;
        }
        if self.plugin_fetching {
            self.notify(Severity::Info, "Still downloading…");
            return;
        }
        self.plugin_fetching = true;
        self.notify(Severity::Info, format!("Downloading {} {}…", entry.name, entry.version));
        self.effects.push(Effect::DownloadPlugin { entry: Box::new(entry), plugins_dir: self.plugins_dir() });
    }

    pub(crate) fn on_plugin_downloaded(&mut self, result: Result<Box<yace_plugin_host::InstallPlan>, String>) {
        self.plugin_fetching = false;
        match result {
            Ok(plan) => self.confirm_install(*plan),
            Err(e) => self.notify(Severity::Error, format!("Could not install: {e}")),
        }
    }

    /// Ask to install `plan`, showing what it is and what it can do.
    fn confirm_install(&mut self, plan: yace_plugin_host::InstallPlan) {
        let mut message = crate::plugin_cli::trust_summary(&plan);
        if self.plugins.safe_mode {
            message.push("(Safe mode: it will be loaded after a restart without --safe-mode.)".into());
        }
        self.pending_install = Some(plan);
        self.open_overlay(Overlay::Confirm(Confirm {
            title: "Install Plugin".into(),
            message,
            buttons: vec![("Install".into(), ConfirmAction::InstallPlugin), ("Cancel".into(), ConfirmAction::Nothing)],
            selected: 1,
            cancel: ConfirmAction::Nothing,
        }));
    }

    pub(crate) fn plugins_install_confirmed(&mut self) {
        let Some(plan) = self.pending_install.take() else { return };
        let id = plan.manifest.id.clone();
        let was_loaded = self.plugins.find(&id).is_some_and(|i| self.plugins.entries[i].loaded.is_some());
        if let Some(i) = self.plugins.find(&id) {
            self.stop_plugin(i, PluginState::Inactive);
        }
        match store::install(&plan, &self.plugins_dir()) {
            Ok(_) => {
                let mut msg = format!("Installed {} {}", plan.manifest.name, plan.manifest.version);
                if was_loaded && !plan.manifest.unloadable {
                    msg.push_str(" — restart YACE to load the new version of its code");
                }
                self.notify(Severity::Info, msg);
                self.discover_plugins();
            }
            Err(e) => self.notify(Severity::Error, format!("Could not install: {e}")),
        }
    }

    /// Extensions: Reload — stop every plugin and read the plugins folder
    /// again; plugins load on their next activation event.
    pub(crate) fn plugins_reload(&mut self) {
        for i in 0..self.plugins.entries.len() {
            if self.plugins.entries[i].state == PluginState::Active {
                self.stop_plugin(i, PluginState::Inactive);
            }
        }
        self.plugins.entries.clear();
        self.discover_plugins();
        self.notify(
            Severity::Info,
            "Plugins reloaded (code of plugins already loaded stays until restart unless they are unloadable)",
        );
    }

    /// A picker of every plugin view.
    pub(crate) fn plugin_views_picker(&mut self) {
        let views = self.plugins.views();
        if views.is_empty() {
            self.notify(Severity::Info, "No plugin contributes a view");
            return;
        }
        let items: Vec<PickerItem> = views
            .iter()
            .enumerate()
            .map(|(k, v)| PickerItem::new(v.title.clone(), self.plugins.entries[v.plugin].name().to_string(), "", k))
            .collect();
        self.open_overlay(Overlay::Picker(Picker::new("Show Plugin View", items, PickerAction::PluginView)));
    }

    pub(crate) fn plugin_view_pick(&mut self, data: usize) {
        if let Some(v) = self.plugins.views().get(data).cloned() {
            self.show_plugin_view(v.plugin, &v.id);
        }
    }

    /// Extensions: Show Performance (spec §18.10).
    pub(crate) fn plugins_performance(&mut self) {
        let mut lines = vec![format!(
            "{:<28} {:<10} {:>9} {:>7} {:>9} {:>9} {:>5}",
            "Plugin", "State", "activate", "calls", "total ms", "max ms", "slow"
        )];
        for (i, e) in self.plugins.entries.iter().enumerate() {
            let state = match &e.state {
                PluginState::Active => "active",
                PluginState::Inactive => "inactive",
                PluginState::Disabled => "disabled",
                PluginState::Failed(_) => "failed",
                PluginState::Untrusted(_) => "untrusted",
                PluginState::Incompatible(_) => "incompat.",
                PluginState::Invalid => "invalid",
            };
            let stats = e.loaded.as_ref().map(|l| l.stats.clone()).unwrap_or_default();
            let calls: u64 = stats.values().map(|s| s.calls).sum();
            let total: f64 = stats.values().map(|s| s.total.as_secs_f64() * 1e3).sum();
            let max: f64 = stats.values().map(|s| s.max.as_secs_f64() * 1e3).fold(0.0, f64::max);
            let slow: u64 = stats.values().map(|s| s.slow).sum();
            let act =
                e.activation_time.map(|d| format!("{:.1} ms", d.as_secs_f64() * 1e3)).unwrap_or_else(|| "—".into());
            let name: String = e.name().chars().take(28).collect();
            lines.push(format!("{name:<28} {state:<10} {act:>9} {calls:>7} {total:>9.2} {max:>9.2} {slow:>5}"));
            for (cb, st) in &stats {
                lines.push(format!(
                    "    {:<24} {:>7} calls, {:>8.2} ms total, {:>8.2} ms max",
                    format!("{cb:?}"),
                    st.calls,
                    st.total.as_secs_f64() * 1e3,
                    st.max.as_secs_f64() * 1e3
                ));
            }
            let jobs = self.plugins.jobs.running(i);
            if jobs > 0 {
                lines.push(format!("    {jobs} background job(s) running"));
            }
        }
        lines.push(String::new());
        lines.push(
            "Plugins run inside YACE's process; their memory is part of YACE's (Developer: Show Performance Metrics)."
                .into(),
        );
        self.open_overlay(Overlay::Info { title: "Plugin Performance".into(), lines });
    }

    fn plugin_show_log(&mut self, i: usize) {
        let e = &self.plugins.entries[i];
        let mut text = format!("# Log of plugin {} ({})\n", e.name(), e.id());
        for l in &e.log {
            text.push_str(l);
            text.push('\n');
        }
        if e.log.is_empty() {
            text.push_str("(empty)\n");
        }
        self.new_untitled_with(&text);
    }

    fn plugin_details(&mut self, i: usize) {
        let e = &self.plugins.entries[i];
        let mut lines = vec![format!("{} — {}", e.name(), e.state.label()), format!("Folder: {}", e.dir.display())];
        for p in &e.problems {
            lines.push(format!("Problem: {p}"));
        }
        if let Some(m) = &e.manifest {
            lines.push(format!("Id: {}   Version: {}   ABI: {}   Needs YACE {}", m.id, m.version, m.abi, m.yace));
            if !m.publisher.is_empty() {
                lines.push(format!("Publisher: {}", m.publisher));
            }
            if !m.description.is_empty() {
                lines.push(m.description.clone());
            }
            let count = |n: usize, what: &str| (n > 0).then(|| format!("{n} {what}"));
            let contrib: Vec<String> = [
                count(m.commands.len(), "commands"),
                count(m.keybindings.len(), "keybindings"),
                count(m.views.len(), "views"),
                count(m.status_items.len(), "status items"),
                count(m.quick_open.len(), "Quick Open providers"),
                count(m.themes.len(), "themes"),
                count(m.grammars.len(), "grammars"),
                count(m.languages.len(), "languages"),
                count(m.lsp_servers.len(), "language servers"),
                count(m.settings.len(), "settings"),
            ]
            .into_iter()
            .flatten()
            .collect();
            if !contrib.is_empty() {
                lines.push(format!("Contributes: {}", contrib.join(", ")));
            }
            for s in &m.lsp_servers {
                let running = self
                    .lsp
                    .servers
                    .iter()
                    .filter(|x| x.def.origin.as_deref() == Some(m.id.as_str()) && x.def.name == s.name)
                    .map(|x| format!("{} in {}", x.status_text(), x.root_name))
                    .collect::<Vec<_>>();
                let state = if running.is_empty() { "not running".to_string() } else { running.join(", ") };
                lines.push(format!("Language server {}: {} ({state})", s.name, s.command_line()));
            }
            if !m.activation.is_empty() {
                lines.push(format!("Activates on: {}", m.activation.join(", ")));
            }
            if !m.capabilities.is_empty() {
                lines.push(format!("Capabilities: {}", m.capabilities.join(", ")));
            }
            for (dep, req) in &m.dependencies {
                let ok = self
                    .plugins
                    .entries
                    .iter()
                    .filter_map(|x| x.manifest.as_ref())
                    .find(|x| x.id == *dep)
                    .is_some_and(|x| semver_matches(req, &x.version));
                lines.push(format!("Depends on {dep} {req}: {}", if ok { "installed" } else { "MISSING" }));
            }
            if !m.external_programs.is_empty() {
                lines.push(format!("Uses programs: {}", m.external_programs.join(", ")));
            }
            let trust = TrustRecord::load(&self.plugins_dir());
            match trust.plugins.get(&m.id) {
                Some(t) => {
                    lines.push(format!("Approved from {} ({} files)", t.source, t.files.len()));
                    for f in m.code_files() {
                        if let Some(h) = t.files.get(&f) {
                            lines.push(format!("  {f} sha256:{h}"));
                        }
                    }
                }
                None if m.library.is_some() => lines.push("Not approved (not installed through YACE)".into()),
                None => {}
            }
            if m.library.is_some() {
                lines.push("Native code runs inside YACE and is not sandboxed.".into());
            }
        }
        self.open_overlay(Overlay::Info { title: "Plugin Details".into(), lines });
    }

    /// Extensions: Open Plugin Settings — adds the plugin's section with
    /// its defaults to `config.toml` if missing, then opens the file.
    fn plugin_open_settings(&mut self, i: usize) {
        let Some(m) = self.plugins.entries[i].manifest.clone() else { return };
        if m.settings.is_empty() {
            self.notify(Severity::Info, format!("{} has no settings", m.name));
            return;
        }
        let path = self.paths.user_settings();
        let present = self.settings.plugin.contains_key(&m.id);
        if !present {
            let res = yace_config::edit_toml(&path, |doc| {
                use yace_config::toml_edit::{Item, Table};
                if !doc.contains_table("plugin") {
                    let mut t = Table::new();
                    t.set_implicit(true);
                    doc.insert("plugin", Item::Table(t));
                }
                let mut t = Table::new();
                for (k, decl) in &m.settings {
                    if let Ok(v) = toml_to_edit(&decl.default) {
                        t.insert(k, Item::Value(v));
                    }
                    if !decl.description.is_empty()
                        && let Some(mut key) = t.key_mut(k)
                    {
                        key.leaf_decor_mut().set_prefix(format!("# {}\n", decl.description));
                    }
                }
                if let Some(plugin) = doc.get_mut("plugin").and_then(Item::as_table_mut) {
                    plugin.insert(&m.id, Item::Table(t));
                }
            });
            if let Err(e) = res {
                self.notify(Severity::Error, e);
                return;
            }
        }
        self.open_file(&path, false);
    }
}

fn semver_matches(req: &str, version: &str) -> bool {
    match (semver_req(req), semver_ver(version)) {
        (Some(r), Some(v)) => r.matches(&v),
        _ => false,
    }
}

fn semver_req(s: &str) -> Option<yace_plugin_host::manifest::VersionReq> {
    yace_plugin_host::manifest::VersionReq::parse(s).ok()
}

fn semver_ver(s: &str) -> Option<yace_plugin_host::manifest::Version> {
    yace_plugin_host::manifest::Version::parse(s).ok()
}

fn toml_to_edit(v: &toml::Value) -> Result<yace_config::toml_edit::Value, String> {
    let text = format!("x = {v}");
    let doc: yace_config::toml_edit::DocumentMut =
        text.parse().map_err(|e: yace_config::toml_edit::TomlError| e.to_string())?;
    doc.get("x").and_then(|i| i.as_value()).cloned().ok_or_else(|| "not a value".into())
}
