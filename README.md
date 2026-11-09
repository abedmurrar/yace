# YACE — Yet Another Code Editor

A fast, terminal-native code editor written in Rust. Its defining feature is
**first-class multi-root workspaces**: open an existing VS Code / Cursor
`.code-workspace` whose folders live in completely unrelated directories, and
work across all of them in one Explorer with VS Code keyboard shortcuts.

No Electron, no Node.js, no JavaScript runtime, no cloud account, no
telemetry.

> **Status:** milestones M1–M9 are implemented: multi-root workspaces,
> Explorer, full editing, fuzzy Quick Open and workspace search, the
> integrated terminal, themes and a keybinding editor, tree-sitter syntax
> highlighting, folding, word wrap, language servers (completion,
> diagnostics, go to definition, rename, symbols, code actions,
> formatting), native plugins and Git. Next: preparing the public release. See [`TODO.md`](TODO.md) for the complete, itemised status.

## Install

On Ubuntu 22.04+ and Debian 12+:

**One line** (adds YACE's signed APT repository, then installs):

```sh
curl -fsSL https://abedmurrar.github.io/yace/install.sh | sudo sh
```

**Or download the package** and open it (double-click), or install it from
a terminal:
[amd64](https://github.com/abedmurrar/yace/releases/latest/download/yace_amd64.deb)
·
[arm64](https://github.com/abedmurrar/yace/releases/latest/download/yace_arm64.deb)

```sh
sudo apt install ./yace_amd64.deb
```

Either way, the package sets up YACE's repository, so `sudo apt upgrade`
keeps it up to date, and YACE appears in your app grid.

Release archives and `.deb` files are on the
[releases page](https://github.com/abedmurrar/yace/releases). Or build from
source:

## Build and run

```sh
cargo build --release
ln -s "$PWD/target/release/yace" ~/.local/bin/yace   # or: cargo install --path crates/yace

yace                          # reopen the most recent workspace
yace .                        # open the current folder
yace ~/Dev/api-folder         # open a folder
yace file.js                  # open just that file (no folder)
yace src/main.rs:42:8         # a file at line 42, column 8
yace ~/work.code-workspace    # open a multi-root workspace
yace api --add-root ../web    # combine unrelated folders
yace --safe-mode              # start without loading any plugin code
yace --install-plugin pkg.tar.gz   # install a plugin (asks first)
```

Requires a Rust stable toolchain (edition 2024, rustc ≥ 1.88).

## What works today

- **Multi-root workspaces.** Reads `.code-workspace` files (JSONC with
  comments and trailing commas, absolute and relative paths, custom names,
  `file://` URIs). Edits (add/remove/rename/reorder folders) touch only the
  `folders` array. Comments, formatting, `settings`, `launch` and every
  other field are preserved. A missing folder is shown as unavailable and
  never blocks the others. Removing a folder never deletes anything on disk.
- **Explorer.** One lazily loaded tree per root. Each directory is read only
  when it's expanded, on worker threads, and only expanded directories are
  watched. It supports type-to-filter, reveal active file, new file/folder,
  rename, move, copy/cut/paste (in the background), delete-to-trash, open
  to the side, reveal in the file manager, copy path / relative path, and
  the mouse. File-type icons: coloured devicons with a Nerd Font
  (`workbench.glyphs = "nerdFont"`), a coloured dot otherwise — no special
  font is ever required.
- **Editing.** VS Code-style, non-modal editing:
  - multiple cursors (Ctrl+D, Ctrl+Shift+L, Ctrl+Shift+Up/Down, Alt+Click)
  - word-wise undo/redo
  - auto-indent, auto-closing brackets and quotes, matching-bracket highlight
  - move/copy/delete lines and toggle comments
  - smart Home, plus multi-cursor-aware copy/cut/paste (system clipboard,
    OSC 52 or internal)
  - Unicode-correct cursor movement (graphemes, wide characters, emoji)
- **Files.** Atomic, permission-preserving saves (symlinks stay symlinks).
  Byte-exact line endings (LF, CRLF and mixed files are preserved). UTF-8
  with or without BOM. Binary and invalid-UTF-8 files are refused rather
  than corrupted. External changes are detected: clean buffers reload, dirty
  ones ask. Large files get a reduced-feature mode.
- **Tabs and splits.** Preview tabs, reopen closed, MRU switching, split
  right/down, move editors between groups, maximise, and resizing by mouse.
- **Settings.** Layered: defaults < `~/.config/yace/config.toml` <
  workspace settings < per-repository `.vscode/settings.json`. Invalid
  values are reported with file and key, and the default is used instead.
  Changes apply as soon as a file is saved (live reload); a file that stops
  parsing keeps its last good values.
- **Themes.** YACE Dark and Light, One Dark, Dracula, Catppuccin Mocha and
  Tokyo Night, switched live with a previewing picker (Ctrl+K Ctrl+T). Your
  own TOML themes can extend any of them; single colours can be overridden
  in settings with VS Code's keys; VS Code themes can be imported (data
  only). Works in true colour, 256 and 16 colours, and without colour
  (`NO_COLOR`). See [`docs/themes.md`](docs/themes.md).
- **Keyboard shortcuts editor** (Ctrl+K Ctrl+S): search by name or by
  pressing keys, rebind, remove, reset, see conflicts; changes go to
  `keybindings.toml` with your comments kept.
- **Sessions and layout.** Tabs, cursors, splits and their sizes, Explorer
  expansion, sidebar and panel state are restored per workspace; the side
  bar can sit left or right; small terminals degrade gracefully down to
  1×1.
- **Command palette** (Ctrl+Shift+P / F1) for every command.
- **Quick Open and search.** Fuzzy file search across every root (Ctrl+P)
  that stays live as files come and go; workspace search and replace
  (Ctrl+Shift+F) with results that follow your edits and a before/after
  preview of every replacement; find/replace in the editor.
- **Integrated terminal.** Real shells per root (Ctrl+`), several tabs,
  split panes side by side, and find in the scrollback (Ctrl+F).
- **Syntax.** Tree-sitter highlighting for Rust, TypeScript/TSX,
  JavaScript/JSX, JSON, YAML, TOML, HTML, CSS, Markdown, Python, Bash,
  Makefile, Lua and INI/git config built in, with embedded languages; C/C++, Go, Java, Ruby, SCSS, SQL and
  Dockerfile from the installable grammar package (loaded on first use).
  Code folding and word wrap (Alt+Z).
- **Git, per repository.** Each workspace folder shows its own branch in
  the status bar, changed files in the Explorer and change bars in the
  gutter (as you type); a Source Control view lists conflicts, staged
  changes, changes and untracked files of every repository, with diffs,
  stage/unstage, commit and branch switching. Uses your `git`; reads never
  lock the index and nothing changes a repository unless you ask. See
  [`docs/git.md`](docs/git.md).
- **Native plugins.** Rust (or C ABI) shared libraries with commands,
  keybindings, settings, sidebar views, status items, Explorer badges,
  Quick Open providers, terminals, themes, grammars and background jobs.
  Loaded lazily, code verified against what you approved, every call timed,
  safe mode. Not sandboxed — see [`docs/plugins.md`](docs/plugins.md).
  Example: a task runner for npm/pnpm/yarn scripts, Cargo and Makefiles.
- **Language servers.** Completion while you type, diagnostics (Problems:
  Ctrl+Shift+M, F8), hover, parameter hints, go to definition (F12, also
  into other roots), references (Shift+F12), rename (F2), document and
  workspace symbols (Ctrl+Shift+O, Ctrl+T), code actions (Ctrl+.) and
  formatting (Ctrl+Shift+I, format on save). Servers are the ones you
  install (`rust-analyzer`, `typescript-language-server`, `gopls`, …); YACE
  starts them on demand, one per workspace folder, and never installs
  anything. Node-based servers are external tools: their memory is
  reported separately ("Developer: Show Running Language Servers").

## Documentation

- The documentation website, with screenshots: <https://abedmurrar.github.io/yace/>
  once the repository is public; its source is [`site/`](site/README.md)
- [`docs/keybindings.md`](docs/keybindings.md): every shortcut, plus what
  each needs from your terminal and the fallbacks for older terminals
- [`docs/configuration.md`](docs/configuration.md): every setting
- [`docs/architecture.md`](docs/architecture.md): how the code is organised
- [`TODO.md`](TODO.md): the master checklist for the whole specification
- [`CHANGELOG.md`](CHANGELOG.md), [`CONTRIBUTING.md`](CONTRIBUTING.md), [`SECURITY.md`](SECURITY.md), [`docs/releasing.md`](docs/releasing.md)

## Development

```sh
cargo test --workspace                        # unit, integration and PTY end-to-end tests
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all --check
cargo bench -p yace-buffer                    # text-engine benchmarks
scripts/bench.sh 20 .                         # startup time and memory
scripts/bench_interactive.py 300              # idle CPU and typing latency
YACE_LOG=debug yace .                         # log to ~/.local/state/yace/yace.log
```

## Acknowledgements

YACE is built on excellent open-source work: alacritty_terminal (the integrated
terminal), ripgrep's crates (search), Ratatui and crossterm (UI), Ropey (text
buffer) and many more. It follows VS Code's keyboard shortcuts and its
`.code-workspace` format. See [`CREDITS.md`](CREDITS.md) and the full
[`THIRD_PARTY_LICENSES.md`](THIRD_PARTY_LICENSES.md).

## Licence

Dual-licensed under [MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE), at your option.
See [`CHANGELOG.md`](CHANGELOG.md) for release history and [`docs/releasing.md`](docs/releasing.md) for versioning.
