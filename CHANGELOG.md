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

### Fixed

- In terminals without the enhanced keyboard protocol (e.g. GNOME
  Terminal), `` Ctrl+` `` arrives as `Ctrl+Space` and showed "Completion
  is not available yet". There, `Ctrl+Space` now toggles the integrated
  terminal, so `` Ctrl+` `` works as expected. `Ctrl+J` works too.
  (`` Alt+` `` is taken by GNOME's window switcher on Ubuntu.)

## [0.3.0] - 2026-11-03

### Added

- **Integrated terminal** in a bottom panel, running real shells on a PTY
  (`alacritty_terminal`). Colours, full-screen apps (vim, htop, less,
  Claude Code), resizing, bracketed paste, mouse reporting, a bounded
  scrollback and the alternate screen all work.
  - Open with `` Ctrl+` `` (or `` Alt+` `` on terminals that can't send it),
    toggle the panel with `Ctrl+J`, add one with `` Ctrl+Shift+` ``.
  - Terminals are **root-aware**. A new one starts in the workspace folder
    of the active file, and "Open Terminal in Workspace Folder…" lets you
    pick. "Open Terminal in Active File's Folder" and Explorer → "Open in
    Integrated Terminal" are there too. Each tab shows its folder, e.g.
    `1: zsh — GLOBAL API`.
  - Multiple sessions with tabs: `Ctrl+PgUp`/`Ctrl+PgDn`, click, `×`, `+`.
    Rename, clear, kill, and a maximize panel.
  - With the terminal focused, every key goes to the shell. Ctrl+C
    interrupts, or copies when text is selected. Only a short list of
    commands keeps working: Quick Open, the palette, terminal toggles and
    switching, Explorer, search, focus editor, `F6`. Configure it with
    `terminal.commandsToSkipShell`.
  - Mouse: drag to select (Shift inside mouse-aware apps), the wheel
    scrolls back, middle-click pastes. `Shift+PgUp/PgDn` scrolls too.
  - Lifecycle: an exited shell shows its exit code and `Enter` restarts it.
    Killing a terminal while a command runs asks first, and so do quitting
    or switching workspace with running commands. Shells are stopped on
    exit, so no orphan processes are left.
  - Profiles (`[[terminal.profiles]]`: shell, args, env, `startupCommand`,
    e.g. one that runs `claude`) and "Create New Terminal (With Profile)…".
  - Terminal tabs are restored with the workspace as **new** sessions,
    marked "(new session)". YACE never pretends a process survived a quit.
- `[terminal]` settings (user scope only, so a workspace can't choose the
  program that runs): `shell`, `args`, `env`, `scrollback`, `panelHeight`,
  `cwd`, `commandsToSkipShell`, `confirmKill`, `profiles`,
  `defaultProfile`.

### Performance

- Terminal output is parsed on its own thread, and redraws are coalesced to
  one per frame. Typing in the editor while a terminal prints
  `seq 1 2000000`: input handling p95 0.47 ms, render p95 0.13 ms.
- Editor memory: 7.9 MiB with one terminal, 26.7 MiB with a full
  10,000-line scrollback. Shells run as separate processes. Reproduce with
  `scripts/bench_terminal.py`.

### Added

- `CREDITS.md` acknowledges the projects YACE is built on, and
  `THIRD_PARTY_LICENSES.md` (generated) lists every bundled crate with its
  licence texts. Both are shipped in release archives and referenced from
  the About dialog.

## [0.2.1] - 2026-11-03

### Changed

- Claude Code worktrees (`**/.claude/worktrees`) are excluded from Quick
  Open and search by default. They are full copies of the repository and
  flooded results with duplicates. Add `"!**/.claude/worktrees"` to
  `search.exclude` to include them.
- The find bar is narrower and has a clickable `×`. Its toggles (`Aa`,
  `ab`, `.*`, `≡`) are clickable too, and it shows "Esc closes" while the
  query is empty.

### Performance

- Freed memory is returned to the OS after indexing, searches and closing
  large files: glibc arenas are capped at 2, with `malloc_trim` after
  background work. Measured on a 9-folder workspace after Quick Open and a
  search-as-you-type: 20 MiB, down from 33 MiB. Peak was 21.5 MiB.
  Reproduce with `scripts/bench_memory.py`.

## [0.2.0] - 2026-11-03

### Added

- **Quick Open (`Ctrl+P`, `Ctrl+E`)**
  - Fuzzy file search across every workspace folder, with VS Code-style
    ranking. Matches in the file name rank first, as do exact names and
    prefixes, word starts and contiguous matches. Recently opened and
    picked files, plus files in the folder you're working in, get a boost.
  - A word can also match a folder's name, e.g. `api index`.
  - Each result shows the file name with the match highlighted, its
    directory and its folder name, so identical names in different
    repositories are told apart.
  - The window opens instantly. Results stream in while folders are still
    being indexed, and the title shows progress.
  - Respects `.gitignore`, `files.exclude` and the new `search.exclude`.
    `Alt+I` includes ignored files.
  - Modes: `name:42:8` opens at a line and column, `:42` goes to a line,
    `>` runs a command, `?` lists the modes.
  - The file index is built lazily, on first use. It's refreshed in the
    background and cached by name for an instant first open next time
    (`search.cacheIndex`).
- **Command palette (`Ctrl+Shift+P` / `F1`)**
  - Now part of Quick Open: fuzzy matching on title and command id,
    recently used commands first (remembered per workspace), with
    keybindings shown.
- **Find and replace in the editor (`Ctrl+F`, `Ctrl+H`)**
  - A find bar with a live match count ("3 of 41") and highlighted matches.
  - Next/previous (`Enter` / `Shift+Enter`, `F3` / `Shift+F3`). Toggles for
    match case, whole word, regex and in-selection (`Alt+C`, `Alt+W`,
    `Alt+R`, `Alt+L`). Search history.
  - Replace one (`Ctrl+Shift+1`) or all (`Ctrl+Alt+Enter`, as one undo
    step), with `$1` capture groups.
  - `Alt+Enter` turns every match into a cursor.
  - Matching runs on a background thread.
- **Workspace search (`Ctrl+Shift+F`, `Ctrl+Shift+H`)**
  - A Search view in the sidebar, with query, replace, include and exclude
    fields, plus case, whole-word and regex toggles.
  - Results stream in and can be cancelled. They're grouped by folder →
    file → line, with highlighted previews. `Enter` opens a match,
    `Space` previews it.
  - `./Folder Name/…` in "files to include", or Explorer → "Find in
    Folder…" (`Shift+Alt+F`), limits the search to one folder.
  - Respects ignore files and skips binary files. Unsaved editor contents
    are searched as you see them.
  - Replace All asks first. Matches or files can be excluded with
    `Delete`. Open files are edited in their editors (undoable, not saved);
    other files are written to disk, and any file changed since the search
    is skipped and reported.
- **Navigation history**
  - Go Back / Go Forward across files and folders: `Alt+Left` /
    `Alt+Right`, `Ctrl+Alt+-` / `Ctrl+Shift+-`.
- **Settings**
  - A new `[search]` section: `exclude`, `useIgnoreFiles`,
    `quickOpenIncludeIgnored`, `cacheIndex`, `maxResults`,
    `maxIndexedFiles`.
  - `search.exclude` and `search.useIgnoreFiles` are read from VS Code
    workspace and folder settings.

### Changed

- Files larger than 1 MiB open in the background with a "Loading…" tab
  instead of freezing the editor. A requested `--goto` position is
  applied once loading finishes.
- `Ctrl+P` no longer opens a path prompt; "File: Open File…" still does.

### Performance

Measured on an i5-1135G7:
- Quick Open query over 200,000 paths: 9–19 ms, running on a background
  thread.
- Content search of the YACE repository: 3.5 ms.
- Startup is unchanged at 5.4 ms median, since indexing is lazy. Memory at
  the first frame is 6.4 MiB.

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

[Unreleased]: https://github.com/OWNER/yace/compare/v0.3.0...HEAD
[0.3.0]: https://github.com/OWNER/yace/compare/v0.2.1...v0.3.0
[0.2.1]: https://github.com/OWNER/yace/compare/v0.2.0...v0.2.1
[0.2.0]: https://github.com/OWNER/yace/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/OWNER/yace/releases/tag/v0.1.0
