# YACE — Yet Another Code Editor

A fast, terminal-native code editor written in Rust. Its defining feature is
**first-class multi-root workspaces**: open an existing VS Code / Cursor
`.code-workspace` whose folders live in completely unrelated directories, and
work across all of them in one Explorer with VS Code keyboard shortcuts.

No Electron, no Node.js, no JavaScript runtime, no cloud account, no
telemetry.

> **Status:** milestones M1–M3 (foundation, multi-root workspaces + Explorer,
> full basic editing) are implemented. Fuzzy Quick Open, search, the
> integrated terminal, themes, syntax highlighting, LSP, plugins and Git are
> planned. See [`TODO.md`](TODO.md) for the complete, itemised status.

## Build and run

```sh
cargo build --release
./target/release/yace                         # reopen the most recent workspace
./target/release/yace .                       # open the current folder
./target/release/yace ~/work.code-workspace   # open a multi-root workspace
./target/release/yace --goto src/main.rs:42:8 # open a file at a position
./target/release/yace api --add-root ../web   # combine unrelated folders
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
  rename, move, delete-to-trash, copy path / relative path, and the mouse.
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
  Custom keybindings go in `keybindings.toml`.
- **Sessions.** Tabs, cursors, splits, Explorer expansion and sidebar state
  are restored per workspace.
- **Command palette** (Ctrl+Shift+P / F1) for every command.

## Documentation

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
