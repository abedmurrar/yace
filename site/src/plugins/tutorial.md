# Tutorial: write a plugin

**In this tutorial you write "TODO Counter", a plugin that shows how many
lines of the active file contain `TODO` in the status bar, and adds a
command and a key to count them.** It takes about fifteen minutes and uses
the Rust SDK, `yace-plugin`. You'll need Rust, a YACE checkout to build
against, and YACE itself on your `PATH`.

The finished plugin uses the main parts of the SDK: a command, a key
binding, a setting, a status bar item, events, and reading a document. The
bundled examples go further: `plugins/hello` edits documents and runs a
background job; `plugins/tasks` contributes a side bar view, a Quick Open
provider and terminals.

## 1. Create the crate

Next to your YACE checkout (so `../yace` is the repository):

```sh
cargo new --lib todo-count
cd todo-count
```

A plugin is a C-compatible dynamic library. Edit `Cargo.toml`:

```toml
[package]
name = "todo-count"
version = "0.1.0"
edition = "2024"

[lib]
crate-type = ["cdylib"]

[dependencies]
yace-plugin = { path = "../yace/crates/yace-plugin" }
serde_json = "1"
```

## 2. Describe it in a manifest

YACE reads `yace-plugin.toml` before it loads any code: it is how the
command, the key, the status item and the setting appear before the plugin
runs, and what the install prompt shows. Create it next to `Cargo.toml`:

```toml
id = "dev.example.todo-count"
name = "TODO Counter"
version = "0.1.0"
description = "Counts TODO comments in the active editor."
publisher = "Example"
license = "MIT"
yace = ">=0.10"
library = "lib/libtodo_count.so"
activation = ["onStartupFinished"]
capabilities = ["documents.read"]
events = ["activeDocumentChanged", "documentSaved"]

[[commands]]
id = "todo.count"
title = "Count TODO Comments"
category = "TODO"

[[keybindings]]
key = "ctrl+alt+t"
command = "todo.count"

[[status_items]]
id = "todo.count"
alignment = "right"
priority = 10
text = "TODO"

[settings.marker]
type = "string"
default = "TODO"
description = "The word to count."
```

What the keys mean:

- `id` names the plugin (lowercase dotted words) and its folder;
  `library` is where the compiled library will be inside the package.
- `activation` says when to load the code. A plugin's own commands
  activate it anyway; `onStartupFinished` loads it once YACE has
  started, so the status item can show a count straight away.
- `capabilities` declares what the plugin will do through the editor's
  API. Reading document text needs `documents.read`; a call the plugin
  didn't declare is refused.
- `events` are the notifications the plugin wants.
- The status item's `text` is shown until the plugin replaces it.

Every key is listed in the [manifest reference](../docs/plugins.md#manifest-reference-yace-plugintoml).

## 3. Write the code

Replace `src/lib.rs`:

```rust
use serde_json::Value;
use yace_plugin::{Error, Host, Plugin, Result, StatusItem, export_plugin};

pub struct TodoCount {
    host: Host,
}

impl TodoCount {
    /// Count the lines of the active document that contain the marker.
    fn count(&self) -> Result<Option<(String, usize)>> {
        let Some(doc) = self.host.active_document()? else {
            return Ok(None);
        };
        let marker: String = self.host.setting("marker")?;
        let text = self.host.document_text(doc.doc, 0, u64::MAX)?;
        let n = text.lines().filter(|line| line.contains(&marker)).count();
        Ok(Some((marker, n)))
    }

    /// Show the count in the status bar (or clear it with no editor open).
    fn refresh(&self) -> Result<()> {
        let item = self.count()?.map(|(marker, n)| StatusItem {
            text: format!("{marker} {n}"),
            tooltip: format!("Lines containing {marker} in this file"),
            command: "todo.count".into(),
            ..Default::default()
        });
        self.host.set_status_item("todo.count", item.as_ref())
    }
}

impl Plugin for TodoCount {
    fn new(host: Host) -> Self {
        TodoCount { host }
    }

    fn activate(&mut self) -> Result<()> {
        self.refresh()
    }

    fn command(&mut self, id: &str, _args: Option<Value>) -> Result<()> {
        match id {
            "todo.count" => {
                let (marker, n) = self.count()?.ok_or_else(|| Error::new("no editor is open"))?;
                self.host.info(&format!("{n} {marker} comments in this file"));
                self.refresh()
            }
            _ => Err(Error::unsupported()),
        }
    }

    fn event(&mut self, _event: Value) {
        // activeDocumentChanged or documentSaved: count again.
        let _ = self.refresh();
    }
}

export_plugin!(TodoCount);
```

How it fits together:

- `Plugin::new` receives a `Host`, the plugin's handle to the editor.
  Every method runs on the editor's UI thread, so each one must return
  quickly. For slow work (reading many files, running programs), use
  `host.spawn_job` and send the result back with `job.post`; it arrives
  in `Plugin::message`.
- `host.setting("marker")` reads the setting declared in the manifest,
  already checked against its type. Users change it in their
  `config.toml` under `[plugin."dev.example.todo-count"]`.
- Events carry ids, not text: ask for the text with
  `host.document_text` when you need it.
- A command id the plugin doesn't handle returns `Error::unsupported()`.
- `export_plugin!` exports the C entry point YACE calls. Panics are caught
  there and reported; they never unwind into the editor.

## 4. Test it without YACE

`yace_plugin::testing::MockHost` is a fake editor: set its settings and
documents, call your plugin, then check what it did. Add this to the end
of `src/lib.rs`:

```rust
#[cfg(test)]
mod tests {
    use serde_json::json;
    use yace_plugin::{ActiveDocument, testing::MockHost};

    use super::*;

    #[test]
    fn counts_the_marker() {
        let mock = MockHost::new();
        mock.with(|s| {
            s.settings.insert("marker".into(), json!("TODO"));
            s.documents.insert(1, "fn main() {}\n// TODO: tests\n// TODO: docs\n".into());
            s.active = Some(ActiveDocument {
                doc: 1,
                version: 1,
                path: Some("src/main.rs".into()),
                language: "rust".into(),
                selections: vec![],
            });
        });
        let mut plugin = TodoCount::new(mock.host());
        plugin.command("todo.count", None).unwrap();
        assert_eq!(mock.with(|s| s.notifications.last().unwrap().1.clone()), "2 TODO comments in this file");
        assert_eq!(mock.with(|s| s.status_items["todo.count"].clone().unwrap().text), "TODO 2");
    }
}
```

```sh
cargo test
```

## 5. Build and package

Build the library, then put it where the manifest's `library` key says,
next to a copy of the manifest:

```sh
cargo build --release
mkdir -p package/lib
cp yace-plugin.toml package/
cp target/release/libtodo_count.so package/lib/
yace --package-plugin package -o todo-count-0.1.0.tar.gz
```

`--package-plugin` checks the manifest and the files, writes the
package, and prints its SHA-256.

> Plugins that live inside the YACE repository, under `plugins/`, are
> members of its Cargo workspace. For those, `scripts/build_plugin.sh
> plugins/<name>` does the build and the packaging in one step; that is how
> `plugins/hello` and `plugins/tasks` are built.

## 6. Install and try it

```sh
yace --install-plugin todo-count-0.1.0.tar.gz
```

YACE shows what you are installing and asks first:

```text
TODO Counter 0.1.0 (dev.example.todo-count)
Publisher: Example
From: todo-count-0.1.0.tar.gz
Counts TODO comments in the active editor.

Native code: runs inside YACE with your user's permissions.
It is NOT sandboxed: it can read and change your files, start
programs and crash the editor. Install only code you trust.
  lib/libtodo_count.so  sha256:…
Declares it will:
  - read the text of open documents (documents.read)

Install? [y/N]
```

Answer `y`, then open a file in YACE. The right side of the status bar
shows `TODO` and the number of matching lines; `Ctrl+Alt+T` (or "TODO:
Count TODO Comments" in the Command Palette) shows a notification.
In "Extensions: List Installed", "Show Plugin Log" shows what the plugin
logged; "Extensions: Show Performance" shows how long its callbacks took.

## 7. Change it and reinstall

YACE loads a plugin's code only while its files match the hashes recorded
at install time. After rebuilding, package and install the new version
the same way; it replaces the installed one. Unless the manifest says `unloadable = true`,
the new code loads the next time YACE starts.

## Where to go next

- [Plugin reference](../docs/plugins.md): the manifest, every host
  function and event, the C ABI for other languages, and how to release a
  plugin.
- `plugins/hello/src/lib.rs`: editing documents and background jobs.
- `plugins/tasks/src/lib.rs`: views, Quick Open providers and terminals.
