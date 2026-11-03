//! Project Task Runner (spec §18.16): finds the tasks of every workspace
//! folder in a background job, shows them in a sidebar view and in Quick
//! Open (`!`), and runs them in an integrated terminal of that folder.

use std::path::Path;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use yace_plugin::{
    Error, Host, Job, Plugin, QuickOpenItem, Result, Root, StatusItem, TerminalSpec, ViewItem, export_plugin,
};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Task {
    pub root: u64,
    pub root_name: String,
    /// `npm`, `pnpm`, `yarn`, `bun`, `cargo` or `make`.
    pub kind: String,
    pub name: String,
    /// What runs, e.g. `next build` for a script.
    pub detail: String,
}

impl Task {
    fn command(&self) -> (String, Vec<String>) {
        match self.kind.as_str() {
            "cargo" => ("cargo".into(), vec![self.name.clone()]),
            "make" => ("make".into(), vec![self.name.clone()]),
            pm => (pm.into(), vec!["run".into(), self.name.clone()]),
        }
    }
    fn id(&self) -> String {
        format!("{}|{}|{}", self.root, self.kind, self.name)
    }
}

/// The package manager a folder uses, from its lock file.
fn package_manager(dir: &Path) -> &'static str {
    if dir.join("pnpm-lock.yaml").exists() {
        "pnpm"
    } else if dir.join("yarn.lock").exists() {
        "yarn"
    } else if dir.join("bun.lockb").exists() || dir.join("bun.lock").exists() {
        "bun"
    } else {
        "npm"
    }
}

/// Every task of one folder (reads files; runs on a worker).
pub fn scan(root: &Root) -> Vec<Task> {
    let dir = Path::new(&root.path);
    let mut out = Vec::new();
    let task = |kind: &str, name: &str, detail: &str| Task {
        root: root.id,
        root_name: root.name.clone(),
        kind: kind.into(),
        name: name.into(),
        detail: detail.into(),
    };
    if let Ok(text) = std::fs::read_to_string(dir.join("package.json"))
        && let Ok(v) = serde_json::from_str::<Value>(&text)
        && let Some(scripts) = v.get("scripts").and_then(Value::as_object)
    {
        let pm = package_manager(dir);
        for (name, cmd) in scripts {
            out.push(task(pm, name, cmd.as_str().unwrap_or("")));
        }
    }
    if dir.join("Cargo.toml").exists() {
        for (name, detail) in [
            ("build", "compile"),
            ("test", "run the tests"),
            ("run", "build and run"),
            ("check", "type-check"),
            ("clippy", "lint"),
        ] {
            out.push(task("cargo", name, detail));
        }
    }
    for f in ["Makefile", "makefile", "GNUmakefile"] {
        if let Ok(text) = std::fs::read_to_string(dir.join(f)) {
            for name in make_targets(&text) {
                out.push(task("make", &name, ""));
            }
            break;
        }
    }
    out
}

/// Explicit targets of a Makefile (no patterns, variables or specials).
pub fn make_targets(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    for line in text.lines() {
        if line.starts_with(['\t', ' ', '#', '.']) {
            continue;
        }
        let Some((head, rest)) = line.split_once(':') else { continue };
        if rest.starts_with('=') || head.contains(['=', '%', '$']) {
            continue;
        }
        for t in head.split_whitespace() {
            if !out.iter().any(|x| x == t) {
                out.push(t.to_string());
            }
        }
    }
    out
}

pub struct Tasks {
    host: Host,
    tasks: Vec<Task>,
}

impl Tasks {
    fn refresh(&mut self) -> Result<()> {
        let roots = self.host.roots()?;
        self.host.spawn_job(move |job: Job| {
            let mut all = Vec::new();
            for r in roots.iter().filter(|r| r.available) {
                if job.cancelled() {
                    return;
                }
                all.extend(scan(r));
            }
            job.post(&serde_json::to_string(&all).unwrap_or_default());
        })?;
        Ok(())
    }

    fn show(&self) -> Result<()> {
        let mut roots: Vec<ViewItem> = Vec::new();
        for t in &self.tasks {
            let item = ViewItem {
                id: t.id(),
                label: t.name.clone(),
                description: if t.detail.is_empty() { t.kind.clone() } else { format!("{} · {}", t.kind, t.detail) },
                icon: "▶".into(),
                color: "accent".into(),
                ..Default::default()
            };
            match roots.iter_mut().find(|r| r.id == t.root.to_string()) {
                Some(r) => r.children.push(item),
                None => roots.push(ViewItem {
                    id: t.root.to_string(),
                    label: t.root_name.clone(),
                    expanded: true,
                    children: vec![item],
                    ..Default::default()
                }),
            }
        }
        self.host.set_view_items("tasks.view", &roots)?;
        let text = if self.tasks.is_empty() { String::new() } else { format!("{} tasks", self.tasks.len()) };
        self.host
            .set_status_item("tasks.count", (!text.is_empty()).then_some(&StatusItem { text, ..Default::default() }))
    }

    fn run(&self, t: &Task) -> Result<()> {
        let (command, args) = t.command();
        self.host.create_terminal(&TerminalSpec {
            name: format!("{} {}", t.kind, t.name),
            root: Some(t.root),
            command,
            args,
            ..Default::default()
        })?;
        Ok(())
    }

    fn find(&self, id: &str) -> Option<&Task> {
        self.tasks.iter().find(|t| t.id() == id)
    }
}

impl Plugin for Tasks {
    fn new(host: Host) -> Self {
        Tasks { host, tasks: Vec::new() }
    }

    fn activate(&mut self) -> Result<()> {
        self.refresh()
    }

    fn command(&mut self, id: &str, args: Option<Value>) -> Result<()> {
        match id {
            "tasks.refresh" => self.refresh(),
            "tasks.run" => {
                let id = args
                    .as_ref()
                    .and_then(|a| a["task"].as_str())
                    .ok_or_else(|| Error::new("tasks.run needs {\"task\": id}"))?;
                let t = self.find(id).cloned().ok_or_else(|| Error::new(format!("no task {id}")))?;
                self.run(&t)
            }
            _ => Err(Error::unsupported()),
        }
    }

    fn event(&mut self, event: Value) {
        let refresh = match event["type"].as_str() {
            // A branch switch can change package.json, Cargo.toml or the Makefile.
            Some("rootAdded" | "rootRemoved" | "gitStatusChanged") => true,
            Some("documentSaved") => event["path"]
                .as_str()
                .is_some_and(|p| ["package.json", "Cargo.toml", "Makefile"].iter().any(|f| p.ends_with(f))),
            _ => false,
        };
        if refresh {
            let _ = self.refresh();
        }
    }

    fn message(&mut self, data: &str) {
        if let Ok(tasks) = serde_json::from_str(data) {
            self.tasks = tasks;
            let _ = self.show();
        }
    }

    fn view_action(&mut self, _view: &str, item: &str, _action: &str) -> Result<()> {
        let t = self.find(item).cloned().ok_or_else(|| Error::new("unknown task"))?;
        self.run(&t)
    }

    fn quick_open(&mut self, _provider: &str, query: &str, request: u64) -> Result<()> {
        let words: Vec<String> = query.to_lowercase().split_whitespace().map(str::to_string).collect();
        let items: Vec<QuickOpenItem> = self
            .tasks
            .iter()
            .filter(|t| {
                let hay = format!("{} {} {}", t.root_name, t.kind, t.name).to_lowercase();
                words.iter().all(|w| hay.contains(w))
            })
            .map(|t| QuickOpenItem {
                label: t.name.clone(),
                description: format!("{} · {}", t.root_name, t.kind),
                command: "tasks.run".into(),
                args: Some(json!({ "task": t.id() })),
            })
            .collect();
        self.host.quick_open_results(request, &items)
    }
}

export_plugin!(Tasks);

#[cfg(test)]
mod tests {
    use yace_plugin::testing::MockHost;

    use super::*;

    #[test]
    fn finds_tasks_and_runs_them() {
        let d = std::env::temp_dir().join(format!("yace-tasks-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join("package.json"), r#"{"scripts": {"dev": "next dev", "build": "next build"}}"#).unwrap();
        std::fs::write(d.join("pnpm-lock.yaml"), "").unwrap();
        std::fs::write(d.join("Makefile"), "all: build\n\ttouch x\nVAR = 1\n%.o: %.c\nclean deploy:\n.PHONY: all\n")
            .unwrap();
        let mock = MockHost::new();
        let root = Root { id: 1, name: "web".into(), path: d.display().to_string(), available: true };
        mock.with(|s| s.roots = vec![root.clone()]);
        let mut p = Tasks::new(mock.host());
        p.activate().unwrap();
        let posted = mock.with(|s| s.messages.clone());
        p.message(&posted[0]);
        let names: Vec<String> = p.tasks.iter().map(|t| format!("{}:{}", t.kind, t.name)).collect();
        assert_eq!(names, ["pnpm:build", "pnpm:dev", "make:all", "make:clean", "make:deploy"]);
        mock.with(|s| {
            let v = &s.views["tasks.view"];
            assert_eq!(v[0].label, "web");
            assert_eq!(v[0].children.len(), 5);
            assert_eq!(s.status_items["tasks.count"].as_ref().unwrap().text, "5 tasks");
        });
        p.quick_open("!", "dev", 9).unwrap();
        let (req, items) = mock.with(|s| s.quick_open[0].clone());
        assert_eq!(req, 9);
        assert_eq!(items.len(), 1);
        p.command("tasks.run", items[0].args.clone()).unwrap();
        mock.with(|s| {
            let t = &s.terminals[0];
            assert_eq!(
                (t.command.as_str(), t.args.clone(), t.root),
                ("pnpm", vec!["run".to_string(), "dev".into()], Some(1))
            );
        });
        let _ = std::fs::remove_dir_all(&d);
    }
}
