# Changelog

All notable changes to YACE are documented here.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and YACE uses [Semantic Versioning](https://semver.org/spec/v2.0.0.html)
(see [`docs/releasing.md`](docs/releasing.md) for what "breaking" means
before 1.0).

Every user-visible change adds a line under **Unreleased** in the same
commit. Use these sections: Added, Changed, Deprecated, Removed, Fixed,
Security, Performance.

## [Unreleased]

## [0.1.0] - 2026-11-03

First tagged version. It covers milestones M1–M3 (native foundation,
multi-root workspaces with the Explorer, and full basic editing). It isn't
published anywhere yet.

### Added

- **Multi-root workspaces**
  - Open VS Code / Cursor `.code-workspace` files: JSONC with comments and
    trailing commas, absolute and relative paths (resolved against the
    workspace file), `~`, custom folder names, `file://` URIs.
  - Folders can live in completely unrelated directories.
  - Unavailable folders are shown with the reason and never block the
    others. They can be located or removed.
  - Duplicate and nested folders are detected.
  - Add, remove, rename and reorder folders. Edits change only the
    `folders` array. Comments, formatting and every other field are kept.
    Removing a folder never touches the disk.
  - New Empty Workspace, Open Folder, Open Workspace from File, Save
    Workspace As, Open Recent, Close Workspace. Multi-folder sessions that
    were never saved are kept automatically.
- **Explorer**
  - One lazily loaded tree per root, read on worker threads.
  - Directories first, natural sort order, `files.exclude` globs, a
    hidden-files toggle, symlink and broken-link markers, and protection
    against symlink cycles.
  - Keyboard navigation, type-to-filter, reveal active file, and the mouse
    (click, double-click, wheel, sidebar resize).
  - New file/folder, rename, move (also across filesystems), delete to the
    Trash after confirmation, and copy path / relative path. Open tabs
    follow renames and moves.
  - Non-recursive watches on expanded directories only, with events
    coalesced over 100 ms.
- **Editing**
  - Rope-based documents.
  - Grapheme-correct cursor movement: emoji, combining marks, wide CJK
    characters and tabs.
  - VS Code-style non-modal editing: word movement, smart Home,
    PgUp/PgDn with a sticky column, select line/all.
  - Multiple cursors: Ctrl+D, Ctrl+Shift+L, add cursor above/below,
    Alt+Click, Escape to collapse.
  - Undo/redo grouped word by word. A multi-cursor edit is one undo step.
  - Auto-indent, splitting `{|}` across lines on Enter, auto-closing
    brackets and quotes (with type-over and pair deletion), and
    matching-bracket highlight.
  - Move, copy and delete lines; insert line above/below; indent/outdent;
    toggle line comment; trim trailing whitespace; change line endings;
    go to line/column.
  - Multi-cursor-aware copy/cut/paste. With nothing selected they act on
    the whole line. Uses the system clipboard, then OSC 52, then an
    internal clipboard.
- **Files**
  - Atomic saves that keep permissions. Symlinks stay symlinks, and
    hard-linked files are written in place.
  - LF, CRLF and mixed line endings are preserved byte for byte. UTF-8,
    with or without BOM, is preserved.
  - Binary and invalid-UTF-8 files are refused rather than corrupted.
  - Save, Save As (with overwrite and create-folder confirmations), Save
    All, Revert, auto-save (`afterDelay`, `onFocusChange`), trim trailing
    whitespace and insert final newline on save.
  - External changes are detected: clean buffers reload, dirty buffers ask
    first, and later saves warn about the conflict. Files deleted on disk
    stay editable.
  - Large-file confirmation and a reduced-feature mode.
- **Tabs and splits**
  - Preview tabs, disambiguated titles (`index.ts — API / src`),
    unsaved-change markers.
  - Close, close others / to the right / all / saved, reopen closed,
    next/previous, most-recently-used switching, move tabs, Alt+1–9.
  - Split right/down, focus and move editors between groups, maximise,
    resize (also by mouse), reset layout. Split views stay in sync.
  - Unsaved changes are never discarded without asking, whether closing a
    tab, a group, a workspace or quitting (Save / Don't Save / Save All /
    Cancel).
- **Keyboard**
  - VS Code Linux/Windows default keymap, with chords
    (e.g. `Ctrl+K Ctrl+M`) and context-sensitive bindings.
  - User `keybindings.toml` can add, override and remove bindings
    (`-command`).
  - Conflict report.
  - Kitty keyboard protocol when the terminal supports it, with documented
    fallbacks for legacy terminals.
- **Settings**
  - Layered `config.toml` < workspace settings < per-repository
    `.vscode/settings.json`, with VS Code keys mapped for a known subset.
  - Each value is validated on its own. Errors give the file and key, and
    the default is used instead.
  - Settings and keybindings reload when saved from inside YACE.
- **Sessions**
  - Restored per workspace: tabs, cursors, scroll positions, splits,
    Explorer expansion, and sidebar state.
- **Command palette**
  - Every command, with its keybinding, from Ctrl+Shift+P or F1.
- **Diagnostics**
  - "Show Performance Metrics", "Show Configuration Errors", "Show
    Keybinding Conflicts", "Show Notifications".
  - Opt-in file logging (`YACE_LOG`).
- **CLI**
  - `yace [PATH | FILE[:LINE[:COL]] | WORKSPACE]`, `--workspace`,
    `--add-root`, `--goto`, `--no-restore`, `--version`, `--help`, with
    clear errors.
- **Terminal handling**
  - The terminal is restored on normal exit, on panic and on
    SIGTERM/SIGHUP. Resizing works, and small terminals don't crash.
  - Colours degrade to 256 colours, 16 colours or monochrome. Glyph sets:
    Unicode, Nerd Font, ASCII.

### Performance

Measured on an i5-1135G7 under Linux 6.11:
- Cold start to the first frame: 5.1 ms (median of 20 runs).
- Resident memory: 5.4 MiB.
- Idle CPU: 0.00%.
- Typing p95: 0.24 ms to handle the key and 0.18 ms to render, while
  typing into a 6,000-line file. Terminal drawing time isn't included.

Reproduce with `scripts/bench.sh` and `scripts/bench_interactive.py`.

### Known limitations

- `Ctrl+P` opens a path prompt; fuzzy Quick Open arrives in M4.
- Not yet available: search, the integrated terminal, themes, syntax
  highlighting, language servers, plugins and Git. Their shortcuts show a
  notice.
- Very large files are read on the main thread.
- There's no crash recovery for unsaved edits yet.

[Unreleased]: https://github.com/OWNER/yace/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/OWNER/yace/releases/tag/v0.1.0
