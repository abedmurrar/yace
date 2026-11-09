# Plugins

YACE is extended with **native plugins**: shared libraries (`.so` on Linux)
loaded into the editor, written in Rust (with the `yace-plugin` SDK) or any
language that can export a C function. There is no JavaScript runtime.

> **Security.** Native plugins run **inside YACE, with your user's
> permissions, and are not sandboxed**. A plugin can read and change your
> files, start programs, and crash the editor. The capabilities a plugin
> declares are shown before you install it and are checked when it calls
> the editor, but they cannot stop code that ignores the editor's API.
> Install only plugins you trust. See [Safety](#safety).

- [Using plugins](#using-plugins)
- [Grammar package](#grammar-package)
- [Writing a plugin](#writing-a-plugin)
- [Manifest reference](#manifest-reference-yace-plugintoml)
- [ABI reference](#abi-reference)
- [Safety](#safety)
- [Releasing a plugin](#releasing-a-plugin)

## Using plugins

Plugins live in `~/.config/yace/plugins/<id>/`. Commands (also in the
Command Palette under "Extensions"):

| Command | |
|---|---|
| Extensions: Install from Package… / from Directory… | shows what the plugin is, its publisher, the capabilities it declares, the SHA-256 of its code and the security note; installs after you confirm |
| Extensions: List Installed | every plugin with its state; pick one for: Show Details, Enable, Disable, Enable/Disable in This Workspace, Approve Its Code, Open Plugin Settings, Show Plugin Log, Uninstall |
| Extensions: Enable… / Disable… / Uninstall… | |
| Extensions: Reload | stops every plugin and reads the plugins folder again |
| Extensions: Show Performance | activation time, calls and time spent per callback, background jobs |
| Extensions: Show Plugin Views… | open a view a plugin contributes (each view also has a "View: Show …" command) |

From a shell: `yace --install-plugin <package-or-folder> [--yes]`,
`yace --list-plugins`.

**Lazy loading.** Installing reads only the manifest: a plugin's commands,
keybindings, themes, views and settings appear immediately, but its code is
loaded the first time it is needed (one of its commands runs, a file of its
language opens, its view is shown, its Quick Open prefix is typed, a folder
with a given file is opened, …). Reading the manifests of installed plugins
happens off the UI thread: 20 installed plugins add nothing measurable to
startup.

**Approval.** YACE records the SHA-256 of every file of a plugin when you
install it (`~/.config/yace/plugins/installed.toml`) and loads code only if
it still matches. A plugin changed on disk, or copied into the folder by
hand, is listed but not loaded until you install it again or choose "Approve
Its Code".

**Enabling and disabling.** "Disable" adds the id to `plugins.disabled` in
your `config.toml`. "Disable in This Workspace" is remembered in the
workspace's session state — the `.code-workspace` file is never changed.

**Settings.** A plugin's settings go in your `config.toml`:

```toml
[plugin."dev.yace.hello"]
greeting = "Howdy"
```

"Open Plugin Settings" adds the section with every default and its
description. Values are checked against the plugin's declared types; a bad
value is reported and the default used.

**Safe mode.** `yace --safe-mode` (or `--no-plugins`) starts without loading
any plugin code or plugin grammar.

**When something goes wrong.**
- A callback that blocks the editor for more than 100 ms is reported once
  per session, naming the plugin.
- A plugin that panics is reported; after three crashes it is stopped for
  the session.
- If YACE exits abnormally while plugins are loaded, the next start says
  which ones were active.
- "Show Plugin Log" shows what a plugin logged; set `YACE_LOG=debug` for
  YACE's own log (`~/.local/state/yace/yace.log`).

### Example plugins

| Plugin | Shows |
|---|---|
| `plugins/hello` | commands, a keybinding, settings, a status item, events, reading and editing the active document, a background job |
| `plugins/tasks` | **Task Runner**: lists every folder's `package.json` scripts (npm, pnpm, yarn or bun, from the lock file), Cargo commands and Makefile targets in a sidebar view and in Quick Open (`!`), and runs them in an integrated terminal of that folder |

Build one with `scripts/build_plugin.sh plugins/tasks` (writes
`dist/dev.yace.tasks/` and `dist/dev.yace.tasks-1.0.0.tar.gz`), then install
the package.

## Grammar package

Release builds include syntax highlighting for Rust, TypeScript/TSX,
JavaScript/JSX, JSON, YAML, TOML, HTML, CSS, Markdown, Python and Bash.
**C, C++, Go, Java, Ruby, SCSS, SQL and Dockerfile** come in the
`yace-grammars` package (it keeps the editor binary at 17.6 MB instead of
25.4 MB):

```sh
scripts/build_grammars.py            # needs a C compiler; writes dist/
yace --install-plugin dist/yace-grammars-<version>-<arch>.tar.gz
```

Until it is installed, those files open as plain text (comment toggling,
brackets and indentation still work) and YACE says once how to get it. A
grammar library is loaded the first time a file of its language is open
(C++: about 9 ms to verify its hash, then the same query compilation a
built-in grammar needs). Building YACE with `--features
yace-syntax/all-grammars` compiles every grammar in instead.

Any plugin can contribute grammars in the Helix/Neovim layout (a parser
library exporting `tree_sitter_<lang>` and `.scm` queries) — for a language
YACE knows, or for a new one declared with `[[languages]]`.

## Writing a plugin

```sh
cargo new --lib my-plugin
```

`Cargo.toml`:

```toml
[lib]
crate-type = ["cdylib"]

[dependencies]
yace-plugin = { path = "…/yace/crates/yace-plugin" }   # the SDK
serde_json = "1"
```

`src/lib.rs`:

```rust
use serde_json::Value;
use yace_plugin::{Error, Host, Plugin, Result, export_plugin};

struct MyPlugin { host: Host }

impl Plugin for MyPlugin {
    fn new(host: Host) -> Self { MyPlugin { host } }

    fn command(&mut self, id: &str, _args: Option<Value>) -> Result<()> {
        match id {
            "my.hello" => {
                let name: String = self.host.setting("name")?;
                self.host.info(&format!("Hello, {name}!"));
                Ok(())
            }
            _ => Err(Error::unsupported()),
        }
    }
}

export_plugin!(MyPlugin);
```

`yace-plugin.toml` next to `Cargo.toml`:

```toml
id = "dev.example.my-plugin"
name = "My Plugin"
version = "0.1.0"
library = "lib/libmy_plugin.so"

[[commands]]
id = "my.hello"
title = "Say Hello"
category = "My Plugin"

[settings.name]
type = "string"
default = "world"
description = "Who to greet."
```

Build and package with `scripts/build_plugin.sh path/to/my-plugin` (or
build with cargo, put the library under `lib/`, and run `yace
--package-plugin <folder>`, which validates the manifest and prints the
package's SHA-256).

**Rules of thumb.**
- Callbacks run on the editor's UI thread: return quickly. Read files, run
  programs or compute in `host.spawn_job(|job| …)`; check `job.cancelled()`
  and send results with `job.post(…)`, which arrive in `Plugin::message`.
- Ask for document text when you need it (`host.document_text`); events
  carry ids and versions, not text. `selectionChanged` and
  `documentChanged` are coalesced (at most one per 50 ms).
- Edit with `host.apply_edits(doc, version, …)`: refused if the document
  changed since `version`; applied as one undo step.
- Never draw: contribute views (trees of items), status items, Explorer
  decorations and Quick Open results; YACE renders them with the user's
  theme.

**Testing.** `yace_plugin::testing::MockHost` is a fake editor: set
settings, roots and documents, call your plugin's methods, and inspect what
it did (notifications, status items, views, edits, terminals, messages).
Jobs run immediately. See the tests of `plugins/hello` and `plugins/tasks`.

**Other languages.** Include `crates/yace-plugin-api/include/yace_plugin.h`
and export `yace_plugin_init`. A test keeps the header byte-for-byte
compatible with the Rust definitions.

## Manifest reference (`yace-plugin.toml`)

Validated before any code is loaded; every problem is reported.

| Key | Type | |
|---|---|---|
| `id` | string | lowercase dotted words, e.g. `dev.example.tool`; also the folder name |
| `name`, `version` | string | `version` is SemVer |
| `description`, `publisher`, `license`, `homepage` | string | shown in the manager |
| `yace` | string | required YACE version, e.g. `">=0.6"` (default any) |
| `abi` | integer | plugin ABI version (default 1) |
| `os`, `arch` | [string] | e.g. `["linux"]`, `["x86_64", "aarch64"]`; empty = any |
| `library` | string | the plugin library, relative to the plugin folder; omit for data-only plugins (themes, grammars) |
| `activation` | [string] | `onCommand:<id>`, `onLanguage:<language>`, `onView:<view>`, `workspaceContains:<path>`, `onEvent:<event>`, `onQuickOpen:<prefix>`, `onStartupFinished`, `*`. A plugin's own commands, views and Quick Open prefixes activate it without being listed |
| `capabilities` | [string] | `workspace.read`, `documents.read`, `documents.write`, `terminal`, `process`, `network`, `filesystem` |
| `events` | [string] | events delivered to `on_event`: `workspaceOpened`, `workspaceClosed`, `rootAdded`, `rootRemoved`, `documentOpened`, `documentClosed`, `documentSaved`, `documentChanged`, `activeDocumentChanged`, `selectionChanged`, `themeChanged`, `terminalCreated`, `terminalExited`, `configurationChanged`, `gitStatusChanged` (`{repo, roots, branch}`) |
| `unloadable` | bool | the library may be unloaded and loaded again in one session (default false: an updated plugin's code loads after a restart) |
| `[[commands]]` | `id`, `title`, `category`, `palette` | ids must be namespaced and must not start with `workbench.`, `editor.`, `yace.`, `explorer.`, `search.` or `terminal.` |
| `[[keybindings]]` | `key`, `command`, `when`, `args` | VS Code key syntax; between the defaults and the user's `keybindings.toml` |
| `[[status_items]]` | `id`, `alignment` (`left`/`right`), `priority`, `text`, `command` | `text` is shown before the plugin loads; clicking the item runs `command` (the one passed to `set_status_item`, else the manifest's), which also activates a plugin that hasn't loaded yet |
| `[[views]]` | `id`, `title`, `location` (`sidebar`), `empty_text` | a tree in the side bar |
| `[[quick_open]]` | `prefix`, `title` | 1–3 characters, not `>` `:` `?` `@` `#` |
| `[[themes]]` | `path` | a theme file ([themes.md](themes.md)) |
| `[[grammars]]` | `language`, `library`, `symbol`, `highlights`, `injections`, `locals` | `symbol` defaults to `tree_sitter_<language>`; query files are concatenated in order |
| `[[languages]]` | `id`, `name`, `extensions`, `filenames`, `shebangs`, `line_comment`, `block_comment`, `brackets` | a new language |
| `[settings.<key>]` | `type`, `default`, `description`, `enum`, `minimum`, `maximum` | `type`: `string`, `boolean`, `integer`, `number`, `array`, `object` |
| `[dependencies]` | `"<id>" = "<version req>"` | shown in Show Details |
| `external_programs` | [string] | programs it runs (shown before installing) |

## ABI reference

The contract is `crates/yace-plugin-api` (Rust) and
`include/yace_plugin.h` (C). Version **1**.

- **Entry point.** `YaceStatus yace_plugin_init(uint32_t abi, const YaceHost
  *host, YacePlugin *out)`. Called once, when the plugin first activates,
  after its manifest and file hashes were checked. Return `YACE_E_ABI`
  for an ABI you don't support, without writing to `out`.
- **Versioned tables.** `YaceHost` and `YacePlugin` begin with `size` and
  `abi_version`. Fields are only appended within a major version. The host
  passes `out` zeroed with `out->size` set; write at most that many bytes
  and set `size` to what you wrote. A table smaller than version 1's is
  refused. A NULL callback means "not implemented".
- **Strings** (`YaceStr`): UTF-8, not NUL-terminated, borrowed for the call.
  Strings from plugins are checked: NULL with a length, more than 16 MiB,
  or invalid UTF-8 → `YACE_E_INVALID`.
- **Structured data** is UTF-8 JSON (settings, roots, documents, edits,
  items, events); document text is raw UTF-8. Positions are character
  offsets.
- **Memory.** A `YaceBuf` the host fills belongs to the plugin until it
  calls `host->free`. The plugin's instance pointer belongs to the plugin
  (freed in `destroy`).
- **Handles** are opaque `uint64_t` ids; nothing exposes editor memory.
- **Threading.** All callbacks run on the UI thread, one at a time. Host
  functions may be called only from inside a callback on that thread (else
  `YACE_E_UNSUPPORTED`) — except `post_message` and `job_cancelled`, which
  are safe from any thread. `execute_command` runs after the current
  callback returns.
- **Jobs and cancellation.** `spawn_job(fn, data, &id)` runs `fn(data, id)`
  on one of two host worker threads. Poll `job_cancelled(id)`; every job of
  a plugin is cancelled when it is deactivated.
- **Errors.** Every host function returns a status; `last_error` describes
  the last failure (valid until the next call). Codes: `OK`, `FAILED`,
  `ABI`, `INVALID`, `NOT_FOUND`, `STALE` (document changed), `DENIED`
  (capability not declared), `UNSUPPORTED`, `CANCELLED`, `PANIC`.
- **Unwinding** across the boundary is forbidden; the SDK catches panics
  and returns `YACE_E_PANIC`.
- **Lifetimes.** `deactivate` then `destroy` run when the plugin is
  disabled, uninstalled or the editor exits. The library stays mapped for
  the rest of the session unless the manifest says `unloadable = true`.

**Compatibility policy.** The plugin ABI is versioned separately from
YACE's version ([releasing.md](releasing.md)): adding host functions or
callbacks at the end of a table is compatible; anything else is a new major
ABI, and YACE refuses plugins built for an ABI it doesn't support with a
clear message.

**Cost.** Calling a plugin callback takes about 76 ns; a command that
makes three host calls with JSON in both directions about 0.6 µs; delivering
an event about 0.4 µs (i5-1135G7 laptop, release build:
`cargo run --release --example bench_m8`).

## Safety

What YACE does:
- validates the manifest, OS, architecture, YACE version and ABI before
  loading anything;
- shows identity, publisher, capabilities, external programs and the code's
  SHA-256 before installing, and requires confirmation;
- loads code only if its hash matches what you approved;
- checks every pointer, length, string and JSON value a plugin passes, and
  each call against the declared capabilities;
- times every callback and reports plugins that block the editor;
- catches panics at the boundary; stops plugins that keep crashing;
- names the plugins that were active when YACE exited abnormally;
- offers `--safe-mode`.

What it cannot do: stop a native plugin from using the operating system
directly (files, network, processes) or from corrupting the editor's
memory. A plugin is as trustworthy as its author. An out-of-process plugin
host (better fault isolation, at the cost of IPC overhead and memory) is a
possible later addition.

## Releasing a plugin

1. Bump `version` in `yace-plugin.toml` (and `Cargo.toml`).
2. `cargo test` (use `MockHost`), then build with
   `scripts/build_plugin.sh <dir>` for each OS/architecture you support;
   set `os` / `arch` in the manifest accordingly.
3. Install the package in YACE and try it (Show Plugin Log, Show
   Performance).
4. Publish the `.tar.gz` and its SHA-256 (printed by `--package-plugin`);
   users can compare it with what the install prompt shows.
