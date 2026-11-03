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

## [0.8.0] - 2026-11-03

### Added

- Commands take arguments, from key bindings (`args = { … }` in
  `keybindings.toml`) and plugins: `type { text }`,
  `editor.action.insertSnippet { snippet }`,
  `workbench.action.terminal.sendSequence { text }`,
  `workbench.action.quickOpen "<prefix>"`, `workbench.action.gotoLine
  { line, column }`, `workbench.action.files.openFile` / `vscode.open
  { path, line, column }`, `git.checkout { branch }`.
- Explorer: **Copy, Cut and Paste** files and folders (`Ctrl+C`, `Ctrl+X`,
  `Ctrl+V`), across workspace folders. Copies run in the background; an
  existing name gets " copy" / " copy 2" like VS Code; symlinks are copied
  as links; a cut item is dimmed until pasted and open editors follow it.
- **Open to the Side** (`Ctrl+Enter`) in the Explorer and Quick Open.
- **Reveal in File Manager** (`Shift+Alt+R`) opens the containing folder.
- **File-type icons** (`workbench.fileIcons`, on by default): with
  `workbench.glyphs = "nerdFont"`, coloured devicons for ~120 file types
  and special folders in the Explorer, Quick Open, tabs and Source
  Control; in Unicode mode a coloured `●` in the Explorer and Quick Open.
  Names never move when icons are turned on or off.
- A command that can't run says why ("no editor is open", the expected
  arguments); a key binding with wrong `args` names `keybindings.toml`;
  a plugin's failed `executeCommand` is written to its log.

### Fixed

- Nerd Font mode drew no expand/collapse chevrons on folders (they were
  empty), so folder names sat one column left of file names.
- A file created while the Explorer was reading its folder (a checkout, a
  build) could stay missing until the next change; the folder is now read
  again.
- A folder reachable by two paths (a root that is a symlink into another
  root) was watched twice, and collapsing it under one path stopped
  updates under the other.

## [0.7.0] - 2026-11-03

### Added

- **Git** (M9), per repository: every workspace folder uses its own
  repository's state (spec §4.7). Uses your installed `git`; see
  `docs/git.md`.
  - Status bar: the active file's branch, `*` for changes, `↑`/`↓`
    ahead/behind, conflicts.
  - Explorer: changed files coloured and marked (M, A, U, D, R, !),
    folders with changes marked, ignored files dimmed.
  - Gutter: change bars against the index, updated as you type;
    `Alt+F5` / `Shift+Alt+F5` jump between changes; "Show Change Under
    Cursor".
  - Source Control view (`Ctrl+Shift+G`): conflicts, staged changes,
    changes and untracked files of every repository; diffs, stage/unstage
    (`s`/`u`), commit (`c`, with "commit all" when nothing is staged),
    switch or create branches (`b`).
  - Changes appear when repositories change outside YACE (the `.git`
    folder is watched), when the window regains focus and after saves.
  - Opening a workspace only reads repositories (without taking the index
    lock); "Git: Show Output" lists every git command run.
  - Settings `git.enabled`, `git.path`, `git.decorationsEnabled`,
    `git.autoRefresh`, `git.diffDecorations` (VS Code keys imported).
- Plugins receive `gitStatusChanged`; the Task Runner rescans its tasks
  when the branch changes.
- `.diff` / `.patch` files are coloured by line.

### Fixed

- Debounced work (gutter diffs, plugin event coalescing) waited for the
  next key press instead of running on time.

## [0.6.0] - 2026-11-03

### Added

- **Native plugins** (M8). Shared libraries written in Rust (with the new
  `yace-plugin` SDK) or any language with a C ABI (`yace_plugin.h`).
  Plugins can contribute commands, keybindings, settings, sidebar views,
  status bar items, Explorer badges, Quick Open providers (e.g. `!`),
  themes, languages and grammars, open terminals, read and edit
  documents, react to editor events and run background jobs. See
  `docs/plugins.md`.
- Plugins load lazily: installing reads only the manifest; code loads on
  the first command, language, view, folder content or event it declares.
- "Extensions: …" commands: Install from Package/Directory (with a
  confirmation showing identity, capabilities, code hashes and that native
  plugins are not sandboxed), List Installed, Enable/Disable (also per
  workspace), Uninstall, Reload, Show Performance, Show Plugin Views; per
  plugin: details, log, settings, approve.
- `yace --install-plugin`, `--list-plugins`, `--package-plugin` and
  `--safe-mode` (`--no-plugins`).
- Plugin code is loaded only if its SHA-256 matches what you approved;
  slow plugins are reported; crashing plugins are stopped; after an
  abnormal exit YACE names the plugins that were active.
- Example plugins: `hello`, and **Task Runner**, which lists every
  folder's npm/pnpm/yarn/bun scripts, Cargo commands and Makefile targets
  in a sidebar view and in Quick Open (`!`) and runs them in a terminal.
- `scripts/build_plugin.sh` and `scripts/build_grammars.py`.

### Changed

- **Hybrid grammars**: C, C++, Go, Java, Ruby, SCSS, SQL and Dockerfile
  highlighting moved to the installable `yace-grammars` package (built by
  `scripts/build_grammars.py`, published with releases), loaded the first
  time a file of that language opens. Until it is installed those files
  open as plain text (comments, brackets and indentation still work) and
  YACE explains once how to get it. Build with `--features
  yace-syntax/all-grammars` to compile every grammar in.
- Dependencies are built without debug info in dev builds (`target/` was
  growing past 40 GB).

### Performance

- Release binary 25.4 MB → 17.6 MB.
- Calling a plugin costs 76 ns; 20 installed plugins add nothing to
  startup; heap memory at idle is unchanged.

### Plugin ABI

- Version 1 (first release).

## [0.5.0] - 2026-11-03

### Added

- **Themes** (M6). Six built-in themes: YACE Dark (default), YACE Light,
  One Dark, Dracula, Catppuccin Mocha and Tokyo Night. `Ctrl+K Ctrl+T`
  opens a picker that previews each theme as you move; Enter keeps it
  (saved in `config.toml` with your comments kept), Esc goes back.
- Your own themes in `~/.config/yace/themes/*.toml`: VS Code colour ids for
  the interface, scope names for syntax (with bold/italic/underline),
  `extends` to build on another theme. "Create Theme from Current" and
  "Edit User Theme…" get you started; saving applies immediately. See
  `docs/themes.md`.
- Colour overrides without a theme: `workbench.colorCustomizations` and
  `editor.tokenColorCustomizations` (VS Code's keys and shapes, including
  per-theme `"[Name]"` sections), also read from VS Code workspace
  settings, as is `workbench.colorTheme`.
- "Import VS Code Color Theme…": converts a theme from an installed VS
  Code (or Cursor, Windsurf, VS Code Server) extension, or any theme
  `.json`, into a YACE theme. Colour data only; no extension code runs.
- "Developer: Inspect Editor Tokens" shows the scope, colour and style
  under the cursor.
- **Keyboard Shortcuts editor** (`Ctrl+K Ctrl+S`): every command with its
  keys, when-clause and source; search by text, `@conflicts`, `@user`,
  `@unbound`, or by pressing the keys (`Alt+K`); Enter rebinds (chords
  too), Delete removes, `Alt+R` resets; conflicts are marked. "Restore
  Default Keybindings" (keeps a `.bak`) and "Export Keybindings".
- **Live reload**: `config.toml`, `keybindings.toml`, theme files and
  `.vscode/settings.json` apply as soon as they are saved, in YACE or any
  other editor.
- "Open Workspace Settings", "Open Default Settings (Read-only)", "Reset
  Layout (Sidebar, Panel, Group Sizes)", "Reset Editor Group Sizes".
- The side bar can sit on the right: `workbench.sideBarLocation` and
  "View: Primary Side Bar Position: Left/Right".

### Changed

- Without colour (`NO_COLOR`, `colorDepth = "mono"`), selection, cursor,
  list selection, the status bar and matches are shown with reverse video,
  bold and underline instead of disappearing.
- A settings or keybindings file that no longer parses keeps its last
  working values (with the file and line reported) instead of falling back
  to the defaults.
- Small terminals: below 12 rows the panel shows only while focused or
  maximised, and below 3 rows the tab and status bars are hidden. Every
  size down to 1×1 renders.
- VS Code folder settings (`.vscode/settings.json`) that hold window-level
  keys such as the theme are no longer reported as errors; they are
  ignored for the folder, as before.

### Performance

- Built-in themes are compiled from their TOML files, so startup parses no
  theme (0.8 µs to load one; parsing took 40–90 µs, up to 0.3 ms cold).
  Switching themes re-colours the next frame without re-highlighting.
  First frame and heap memory are unchanged from 0.4.2.

## [0.4.2] - 2026-11-03

### Fixed

- Memory: a 13,425-item completion list (TypeScript 7 in a real project)
  made YACE grow from 10 to 103 MiB. Replies are now decoded straight from
  the server's JSON text and each item keeps its original JSON as compact
  text: the same list costs 20.7 MiB, and the memory is returned to the OS
  when the list closes.
- The "no language server" hint names the folder and the server that is
  missing there (in a workspace mixing TypeScript 7 and 5 projects it used
  to suggest installing TypeScript when typescript-language-server was
  what the TypeScript 5 folders needed).

## [0.4.1] - 2026-11-03

### Added

- TypeScript 7 support: projects whose `node_modules/typescript` is 7 or
  newer use TypeScript's own native language server (`tsc --lsp --stdio`)
  automatically, started directly without a Node.js process (138 MiB vs.
  538 MiB for typescript-language-server + tsserver on the same project).
  Projects on older TypeScript keep using typescript-language-server.
- Pull diagnostics (LSP 3.17 `textDocument/diagnostic`), which TypeScript 7
  and other newer servers use instead of pushing them: requested after
  opening and editing a file (one request in flight per file), re-pulled
  across open files when the server says diagnostics span files, and on
  `workspace/diagnostic/refresh`.

### Fixed

- With TypeScript 7 installed, typescript-language-server failed to start
  ("provides no tsserver.js"); YACE now picks the right server per project.

## [0.4.0] - 2026-11-03

### Added

- Syntax highlighting with tree-sitter for Rust, JavaScript/JSX,
  TypeScript/TSX, JSON/JSONC, YAML, TOML, HTML, CSS, SCSS, Markdown, SQL,
  Bash, Python, Go, Dockerfile, C, C++, Java and Ruby, including embedded
  languages (Markdown code fences, `<script>`/`<style>`). Grammars are
  built in but only loaded when a file of that language opens.
- Language detection from `files.associations` (also read from
  `.vscode/settings.json`), file name, extension and `#!` line; the status
  bar shows the detected language.
- Language-aware editing: comment tokens, auto-closing pairs and
  indentation come from the language (`:` indents in Python and YAML,
  Enter between HTML tags splits the line). New **Toggle Block Comment**
  (`Shift+Alt+A`); in CSS and HTML `Ctrl+/` uses block comments.
- Code folding from the syntax tree (indentation for other files): Fold
  (`Ctrl+Shift+[`), Unfold (`Ctrl+Shift+]`), Toggle Fold (`Ctrl+K Ctrl+L`),
  Fold All (`Ctrl+K Ctrl+0`), Unfold All (`Ctrl+K Ctrl+J`), and `▾`/`▸`
  markers in the gutter. Folds survive edits elsewhere and are restored with
  the session.
- Word wrap: `editor.wordWrap` (`off`, `on`, `wordWrapColumn`, `bounded`)
  and `editor.wordWrapColumn`, also read from VS Code settings; `Alt+Z`
  toggles it.
- **Language servers.** YACE starts the servers you have installed
  (`rust-analyzer`, `typescript-language-server`, `pyright`, `gopls`,
  `clangd`, …, or any configured under `[[lsp.servers]]`) when you open a
  file of their language, one per workspace folder:
  - completion while you type and on `.` etc., with snippets and
    auto-imports; `Ctrl+Space` in terminals that can send it
  - diagnostics: underlines, coloured line numbers, counts and the message
    under the cursor in the status bar, Problems (`Ctrl+Shift+M`),
    `F8`/`Shift+F8`
  - hover (`Ctrl+K Ctrl+I`) and parameter hints
  - go to definition (`F12`, `Ctrl+Click`, also into other folders),
    implementations (`Ctrl+F12`), references (`Shift+F12`)
  - rename (`F2`): open files get undoable edits, others are rewritten
    safely
  - symbols in the file (`Ctrl+Shift+O`, Quick Open `@`) and workspace
    (`Ctrl+T`, `#`)
  - code actions (`Ctrl+.`), formatting (`Ctrl+Shift+I`, `Ctrl+K Ctrl+F`)
    and `editor.formatOnSave`
  - "Restart / Stop Language Server", "Show Running Language Servers"
    (with their memory, reported separately from YACE's) and "Show
    Language Server Output"
  YACE never installs servers; when one is missing it says how to install
  it, once, in a message that stays until you press Esc. Node-based
  servers are external tools.

- Notification history (`Ctrl+K Ctrl+N`, "View: Show Notifications"):
  every message of the session, searchable; `Enter` copies it to the
  clipboard. "View: Open Notifications in Editor" shows them as text.

### Changed

- `F6` / `Shift+F6` cycle Editor → Panel → Sidebar (skipping hidden parts),
  so `F6` from the terminal goes to the editor's side of the cycle instead
  of always jumping to the Explorer.
- `Ctrl+/` in plain-text files no longer inserts `//` (plain text has no
  comment syntax, as in VS Code).
- `Alt+Z` now toggles word wrap, as in VS Code, instead of being a Redo
  fallback for legacy terminals; use `Ctrl+Y` to redo there.

### Performance

- Typing never waits for the parser: cheap reparses (predicted under
  0.5 ms) run inline, larger ones on a background thread while the screen
  is drawn from the shifted previous tree. Typing at the top of a 10k-line
  TypeScript file: input p50 0.074 ms, p95 ≤ 1 ms. Highlighting only covers
  the visible lines and is cached. Startup and idle memory are essentially
  unchanged (+0.3 ms, +0.5 MiB); the binary grows from 7.2 MB to 24.4 MB
  because of the built-in grammars.
- Language servers never block the editor: their replies are decoded on
  background threads, completion lists are filtered locally as you type
  (5,000 items: at most 3.1 ms per keystroke), and nothing starts until a
  file needs it.

## [0.3.1] - 2026-11-03

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

[Unreleased]: https://github.com/OWNER/yace/compare/v0.8.0...HEAD
[0.8.0]: https://github.com/OWNER/yace/compare/v0.7.0...v0.8.0
[0.7.0]: https://github.com/OWNER/yace/compare/v0.6.0...v0.7.0
[0.6.0]: https://github.com/OWNER/yace/compare/v0.5.0...v0.6.0
[0.5.0]: https://github.com/OWNER/yace/compare/v0.4.2...v0.5.0
[0.4.2]: https://github.com/OWNER/yace/compare/v0.4.1...v0.4.2
[0.4.1]: https://github.com/OWNER/yace/compare/v0.4.0...v0.4.1
[0.4.0]: https://github.com/OWNER/yace/compare/v0.3.1...v0.4.0
[0.3.1]: https://github.com/OWNER/yace/compare/v0.3.0...v0.3.1
[0.3.0]: https://github.com/OWNER/yace/compare/v0.2.1...v0.3.0
[0.2.1]: https://github.com/OWNER/yace/compare/v0.2.0...v0.2.1
[0.2.0]: https://github.com/OWNER/yace/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/OWNER/yace/releases/tag/v0.1.0
