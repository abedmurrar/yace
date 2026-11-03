# YACE — Master TODO

The complete, living checklist for YACE (Yet Another Code Editor). Every requirement in the product specification is listed here, tagged with its milestone and spec section. **Update this file in the same change as the code it describes.**

Legend:
- `- [ ]` not started
- `- [x]` implemented **and** covered by a test or a documented manual check
- `- [~]` partially done or deliberately deferred (reason given inline)
- `[M1]`…`[M10]` milestone (spec §29) · `§n.n` spec section

---

## Status

| | |
|---|---|
| Current pass | M1–M3 → **v0.1.0**; M4 (Quick Open, palette, find, workspace search) → **v0.2.0** |
| Current step | **M4 complete** — released as v0.2.0; next: M5 (integrated terminal) |
| Toolchain | rustc 1.99.1 stable (rustup) |
| Last updated | 2026-11-03 (M1–M3 complete) |

### Build order for this pass
- [x] Step 0 — rustup, `git init`, `TODO.md`
- [x] Step 1 — Cargo workspace, terminal lifecycle, event loop, empty workbench (render, resize, quit)
- [x] Step 2 — keymap crate, command registry, focus, overlays (Prompt / Confirm / Picker), command palette, config, metrics, CLI
- [x] Step 3 — workspace crate (JSONC parser + CST edits), Explorer, fs workers, watching, workspace commands, session part 1
- [x] Step 4 — buffer crate, unit tests and benchmarks
- [x] Step 5 — editor view, tabs, editing commands, save, external modification
- [x] Step 6 — splits, multi-view sync, session part 2, mouse, docs

---

## Decisions log

| # | Decision | Rationale | Spec |
|---|---|---|---|
| D1 | 5 crates: `yace-buffer`, `yace-workspace`, `yace-keymap`, `yace-config`, `yace` (bin) | §20 says split only when the boundary justifies it; further crates (terminal, syntax, lsp, plugin-api, plugin-host, git) are added in their milestones | §20 |
| D2 | **No Tokio.** Threads plus one `crossbeam-channel` of `AppEvent`; the main loop blocks on `recv`/`recv_timeout` | smallest runtime; zero idle CPU; revisit with benchmarks when LSP lands (M7) | §21.2, §34.6 |
| D3 | Ropey rope as the text buffer | spec's preferred candidate; benchmarks in `crates/yace-buffer/benches` | §9.2, §34.1 |
| D4 | Command IDs reuse **VS Code command IDs** where an equivalent exists | lets VS Code `keybindings.json` be imported later; familiar to users | §5.4, §7 |
| D5 | `.code-workspace` edits go through `jsonc-parser` CST, so only the `folders` array is touched | non-destructive: comments, unknown fields and formatting are preserved | §4.2, §33.12 |
| D6 | Line endings are kept byte-exact in the rope. New lines use the dominant ending. Mixed files are preserved byte-for-byte and shown as "Mixed" | never corrupt files; no silent normalisation | §9.4 |
| D7 | Encodings: UTF-8 and UTF-8 with BOM (BOM preserved). Invalid UTF-8, or a NUL byte in the first 8 KB, means the file is refused with a message; never opened lossily | §9.4 says "must not silently corrupt" | §9.4 |
| D8 | Fs watching: non-recursive watches on expanded directories and on the parents of open files only, coalesced over ~100 ms. No polling fallback; a manual Refresh command instead | bounded inotify use on huge trees | §8.4, §21.5 |
| D9 | Delete in the Explorer goes to the freedesktop trash by default, after confirmation; permanent delete is offered only if trashing fails | protects user work | §8.2 |
| D10 | Clipboard order: system clipboard (arboard, lazy) → OSC 52 → internal register | works over SSH; no hard X11/Wayland dependency | §9.1 |
| D11 | Basic command palette (substring filter) pulled forward from M4 into M2 | without it the workspace commands can't be reached; fuzzy ranking stays in M4 | §7 |
| D12 | Theme is a semantic-token struct with a hardcoded YACE Dark now; loading from TOML comes in M6 | every widget already reads tokens, so M6 is additive | §3.4, §16 |
| D14 | `Ctrl+P` opens a path prompt with Tab completion until fuzzy Quick Open lands (M4); other M4–M7 shortcuts show a "not available yet" notice | a shortcut should never silently do nothing | §5.1, §33.8 |
| D15 | Go to Line (`Ctrl+G`) pulled forward from M4 | trivial with the Prompt overlay | §6.4 |
| D16 | Reloading a file from disk keeps cursors at their offsets (clamped); the reload is undoable | cursors must not jump; a reload must not lose work | §9.10 |
| D17 | Search engine is pure Rust: `ignore` + `grep-searcher`/`grep-regex` (ripgrep's crates) in the new `yace-search` crate; no external `rg` | portable, no runtime dependency, same ignore semantics as ripgrep | §14.2, §34.5 |
| D18 | Own VS Code-style fuzzy scorer (DP alignment, filename-first, word/segment/camel bonuses, deterministic tie-break) instead of nucleo-matcher (MPL-2.0) | no extra licence; fully unit-tested; 200k paths in 9–19 ms using parallel chunks | §6.3 |
| D20 | glibc tuning: `M_ARENA_MAX=2` at startup and `malloc_trim(0)` after indexing, searches and closing large files (`crates/yace/src/mem.rs`, the only `unsafe`); `YACE_MALLOC_TRIM=0` disables it | per-thread arenas kept freed memory after parallel walks; ~30% lower steady-state RSS measured | §23.6 |
| D13 | State files live under `$XDG_STATE_HOME/yace`, config under `$XDG_CONFIG_HOME/yace`, cache under `$XDG_CACHE_HOME/yace`. Every state write is atomic (temp + rename) | §17.2, §26.2 | §17.2 |

### Open questions (spec §34, user decisions pending)
- [x] Licence (§34.12) — **MIT OR Apache-2.0** (`LICENSE-MIT`, `LICENSE-APACHE`), decided 2026-11-03
- [~] Contribution guidelines, maintainers, security reporting, plugin API governance (§34.12) — `CONTRIBUTING.md` and `SECURITY.md` written; maintainers, code of conduct and plugin API governance still open
- [ ] Policy for external Node-based language servers such as the TypeScript one (§2.2, §13.4, §34.7)
- [ ] Plugin execution model: in-process vs out-of-process vs hybrid (§18.9, §34.3)
- [ ] Plugin ABI scope for v1 (§34.4)
- [x] Search engine: pure Rust (`ignore` + `grep-searcher`) vs an external `rg` (§34.5) — pure Rust (D17)
- [ ] Integrated terminal emulator library choice, e.g. `alacritty_terminal` / `vt100` / `wezterm-term` (§11.3, §34.2)
- [ ] Platform scope beyond Linux (§34.8)
- [ ] Policy for importing VS Code themes (§16.7, §34.9)

---

## M1 — Native application foundation (§29)

### Project setup
- [x] [M1] Install rustup stable toolchain with clippy and rustfmt
- [x] [M1] `git init`, `.gitignore`
- [x] [M1] Cargo workspace (`edition = "2024"`), `rust-toolchain.toml`, shared `[workspace.dependencies]`
- [x] [M1] Release profile: `lto = "thin"`, `codegen-units = 1`, `panic = "unwind"` (so the panic hook can restore the terminal)
- [x] [M1] `README.md` with build, run and architecture overview
- [x] [M1] `docs/architecture.md`
- [x] [M1] CI-ready checks: `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test --workspace`

### Terminal lifecycle (§26.3, §22)
- [x] [M1] Enter: raw mode, alternate screen, mouse capture, bracketed paste, hide/show cursor handling
- [x] [M1] Push Kitty keyboard enhancement flags when `supports_keyboard_enhancement()` is true (§5.5)
- [x] [M1] Restore on normal exit via a Drop guard
- [x] [M1] Restore on panic via a panic hook that runs before the default hook prints
- [x] [M1] Restore on SIGTERM / SIGHUP (signal-hook → graceful-quit event)
- [x] [M1] Refuse to start, with a clear message, when stdout is not a TTY (§24.3)
- [x] [M1] Detect colour depth (`COLORTERM`, `TERM`) → truecolor / 256 / 16 / mono (§16.5)

### Event loop and concurrency (§21)
- [x] [M1] Single `AppEvent` channel; input thread using `crossterm::event::read`
- [x] [M1] Main loop blocks on `recv` / `recv_timeout`, so idle CPU is zero (§21.1, §22.1)
- [x] [M1] Render only when dirty (§22.1)
- [x] [M1] Resize events → relayout plus a clamp of all stored sizes (§3.3)
- [x] [M1] Bounded fs worker pool (fixed threads, bounded job queue) (§21.5)
- [~] [M1] Job generation tokens so stale results are dropped (cancellation, §21.3) — M4 workers (index, search, find) use sequence numbers and cancel flags; Explorer directory reads only de-duplicate in-flight requests
- [x] [M1] Input is handled before background results in the same wakeup (priorities, §21.4)

### App core
- [x] [M1] `App` state is terminal-agnostic; `App::handle(AppEvent)`; `ui::draw(&App, &mut Frame)` is pure
- [x] [M1] Headless test harness: drive `App` with synthetic key events, render to ratatui `TestBackend`
- [x] [M1] Error type plus a notification surface (transient status-bar message, sticky notifications, last-error log) (§33.8)
- [x] [M1] File logging via `tracing`, only when `YACE_LOG` is set; never logs file contents or environment (§27.4)

### Command registry (§7, §19)
- [x] [M1] `CommandId` (VS Code IDs where they exist), title, category, availability predicate, handler
- [~] [M1] Typed arguments (`serde_json::Value`-backed), result and error reporting (§19.1) — partial: args are plumbed to handlers; no built-in command uses them yet
- [x] [M1] Every UI action goes through a command, including mouse and explorer actions (§19)
- [x] [M1] Registry can be enumerated by the palette and by keybinding docs

### Keybinding engine — `yace-keymap` (§5)
- [x] [M1] Key spec parser: `ctrl+shift+p`, `ctrl+k ctrl+t`, named keys, `` ` `` / backtick, backslash
- [x] [M1] Focus contexts: `editorFocus`, `explorerFocus`, `overlayFocus`, `inputFocus`, `textInputFocus`
- [x] [M1] Context flags: `editorHasSelection`, `editorHasMultipleSelections`, `sidebarVisible`, …
- [x] [M1] `when` clauses with `&&`, `||`, `!`
- [x] [M1] Chord state machine with timeout, Esc cancel and an "invalid second key" message (§5.3)
- [x] [M1] Layers: default < plugin (reserved) < user; within a layer, later wins (§5.4)
- [x] [M1] Unbinding via `-command`; adding extra bindings; conflict detection report (§5.4)
- [x] [M1] VS Code Linux default keymap table (§5.1)
- [x] [M1] Legacy-terminal fallback bindings, enabled automatically without enhanced keyboard (§5.5)
- [x] [M1] `keybindings.toml` user overrides, loaded and validated
- [x] [M1] Chord-pending indicator in the status bar
- [x] [M1] `docs/keybindings.md`: full table, each combo's terminal requirements, fallbacks (§5.5, §33.10)
- [x] [M1] Unit tests: parse, resolve by context, chords, timeout, precedence, unbind, conflicts (§28.4)

### Focus manager (§5.6)
- [x] [M1] `Focus` enum: Explorer / EditorGroup(id) / Overlay / (Panel reserved for M5)
- [x] [M1] Visible focus indicator (border colour or title) from theme tokens
- [x] [M1] Focus changes never alter the active file
- [x] [M1] Commands: Focus Explorer (`Ctrl+Shift+E`), Focus Active Editor Group, Focus Next/Previous Part (`F6` / `Shift+F6`)

### Overlay layer (§3.1)
- [x] [M1] Overlay stack drawn above the workbench without replacing it
- [x] [M1] `Prompt` widget: single-line input, cursor, editing keys, path completion (Tab)
- [x] [M1] `Confirm` widget: message plus buttons (e.g. Save / Don't Save / Cancel), keyboard and mouse
- [x] [M1] `Picker` widget: input plus filtered list, arrows / PgUp / PgDn, Enter, Esc, highlighted active row (base for Quick Open in M4)
- [x] [M1] Esc closes the top overlay and restores the previous focus

### Command palette — basic (pulled forward from M4, D11)
- [x] [M1] `Ctrl+Shift+P` / `F1` open the palette; lists available commands with category, title and keybinding
- [x] [M1] Substring filtering (fuzzy ranking is M4)
- [x] [M4] Fuzzy ranking, recently used commands first

### Configuration — `yace-config` (§17)
- [x] [M1] XDG paths: config, state, cache (`directories`) (§17.2)
- [x] [M1] `config.toml` schema with defaults (editor, files, explorer, workbench, terminal reserved)
- [x] [M1] Layering: defaults < user < workspace < root (resource scope) < session (§17.1)
- [x] [M1] Per-setting allowed scopes (e.g. layout is not allowed at root scope) (§17.1)
- [x] [M1] Map VS Code workspace settings: `editor.tabSize`, `editor.insertSpaces`, `files.exclude`, `files.autoSave`, `files.insertFinalNewline`, `files.trimTrailingWhitespace`, `files.eol`
- [x] [M1] Validation errors name file and key; fall back to the default per key, never a crash (§17.4)
- [x] [M1] `docs/configuration.md`: every key, default, values, example (§17.6)
- [x] [M1] Unit tests: precedence, scope restrictions, invalid-value fallback (§28.1)
- [~] [M6] Live reload of config and keybinding files (§17.5)

### Theme scaffolding (§3.4, §16)
- [x] [M1] `Theme` semantic-token struct covering every §16.2 token
- [x] [M1] Built-in YACE Dark, hardcoded
- [x] [M1] Colour downgrade truecolor → 256 → 16 (§16.5)
- [x] [M1] Glyph sets: Unicode (default), Nerd Font, ASCII (§8.6)
- [x] [M1] Shared design primitives: borders, focus border, panel titles, scrollbars, tree indent, tab styles (§3.4)

### Metrics and diagnostics (§23, §26.5)
- [x] [M1] Startup timestamp → first frame drawn
- [x] [M1] Per-event handle time and render time (rolling p50 / p95 / max)
- [x] [M1] RSS from `/proc/self/status`
- [x] [M1] `Developer: Show Performance Metrics` command
- [x] [M1] `yace --startup-time`: print time to first frame and exit
- [x] [M1] `scripts/bench.sh`: 20× startup, RSS, results written into this file
- [~] [M10] Full diagnostics report: version, build, OS, terminal capabilities, plugins, LSPs, roots (§26.5)

### CLI (§24)
- [x] [M1] `yace` reopens the most recent workspace (setting `window.restoreWorkspace`)
- [x] [M1] `yace .`, `yace <dir>`, `yace <file>`, `yace <file.code-workspace>`
- [x] [M1] `--workspace <file>`, `--add-root <path>` (repeatable), `--goto path:line[:col]`
- [x] [M1] `--no-restore`, `--version`, `--help`
- [x] [M1] Clear errors: missing path, invalid workspace file, invalid line/col, unsupported flag, no TTY (§24.3)
- [~] [M8] `--safe-mode`, `--no-plugins`
- [~] [M10] Shell completions (§24.4)

---

## M2 — Multi-root workspaces and Explorer (§4, §8, §29)

### Workspace model — `yace-workspace` (§4.1)
- [x] [M2] `Workspace { id, name, file, roots, settings }`; `Root { id, display_name, configured_path, resolved_path, canonical, status }`
- [x] [M2] Stable `RootId`, independent of ordering
- [x] [M2] Root status: Available / Missing(reason) / Duplicate(of) / Nested(in)
- [x] [M2] Path → root resolution (deepest matching root wins; nested roots handled) (§4.11)

### `.code-workspace` compatibility (§4.2)
- [x] [M2] JSONC parse: comments (`//`, `/* */`), trailing commas
- [x] [M2] Absolute paths; relative paths resolved against the **workspace file's directory**; `~` expansion
- [x] [M2] `name` per folder; paths with spaces; non-ASCII paths
- [x] [M2] Workspace file stored outside the folders it references
- [x] [M2] `settings` read (known subset mapped, unknown keys preserved)
- [x] [M2] Unknown top-level fields (`extensions`, `launch`, `tasks`, …) preserved untouched
- [x] [M2] Non-destructive edits via CST (add / remove / rename / reorder folders) (D5)
- [x] [M2] Atomic write of the workspace file
- [x] [M2] YACE-specific settings stored in a namespaced `"yace"` section or in the state file, never by clobbering
- [x] [M2] Tests: every case above, plus CST round-trips that keep comments and unknown fields byte-identical

### Workspace lifecycle commands (§4.3)
- [x] [M2] New Empty Workspace
- [x] [M2] Open Folder (creates an untitled single-root workspace)
- [x] [M2] Open Workspace from File…
- [x] [M2] Add Folder to Workspace… (any unrelated path)
- [x] [M2] Remove Folder from Workspace (the wording says files on disk are not touched) (§4.3)
- [x] [M2] Rename Root display name
- [x] [M2] Move Root Up / Down (reorder)
- [x] [M2] Save Workspace / Save Workspace As…
- [x] [M2] Open Recent Workspace (picker)
- [x] [M2] Close Workspace (with the unsaved-changes flow)
- [x] [M2] Switch between saved workspaces
- [x] [M2] Test: removing a root never deletes its directory (§28.5)

### Missing, duplicate and linked roots (§4.10, §4.11)
- [x] [M2] A missing or unreadable root renders greyed "(missing: reason)" without blocking the others
- [x] [M2] Locate Root… (prompt for a new path) and Remove Root actions on missing roots
- [x] [M2] Never silently substitute another directory
- [x] [M2] Duplicate detection by canonical path; no duplicate watching or indexing
- [x] [M2] Nested root detection
- [x] [M2] Symlink cycle protection during traversal and reveal
- [x] [M2] Case-sensitive and Unicode paths handled

### Workspace security (§4.12, §27.3)
- [x] [M2] Opening a workspace never executes anything (tasks, launch configs and scripts are ignored)
- [~] [M4+] Trust policy for workspace-defined tasks, when tasks exist

### Explorer (§4.4, §8)
- [x] [M2] Each root is a distinct top-level node, rendered in uppercase like VS Code, with its own expansion state (§4.4)
- [x] [M2] Lazy per-directory loading on the worker pool; "loading…" placeholder (§8.3)
- [x] [M2] Sorting: directories first, case-insensitive natural order
- [x] [M2] `files.exclude` globs (defaults `.git`, `node_modules`, `.DS_Store`, …) via globset
- [x] [M2] Toggle hidden / excluded files
- [x] [M2] Symlink marker; broken-symlink marker
- [x] [M2] Keyboard: Up/Down, Left (collapse or go to parent), Right (expand or go to first child), Enter (open), Space (preview), Home/End, PgUp/PgDn (§8.1)
- [x] [M2] Type-to-filter visible entries (§8.1)
- [x] [M2] Mouse: click selects, click on a directory toggles, double-click opens, wheel scrolls
- [x] [M2] Reveal Active File in Explorer (expands ancestors lazily)
- [x] [M2] Scrollbar for long trees
- [x] [M2] Sidebar: toggle (`Ctrl+B`), resize via commands and mouse drag on the border (§3.2)
- [x] [M2] File ops: New File, New Folder (§8.2)
- [x] [M2] File ops: Rename (file or directory)
- [x] [M2] File ops: Delete → trash with confirmation; permanent delete only after a trash failure, with a second confirmation (D9)
- [x] [M2] File ops: Move (rename to path; across roots; cross-filesystem via copy + delete)
- [x] [M2] File ops: Copy Path, Copy Relative Path (relative to the owning root)
- [x] [M2] Collision, permission and cross-device errors reported clearly (§8.2)
- [x] [M2] Renaming or moving an open file updates its open tabs
- [~] [M2+] Copy / paste of files and directories inside the Explorer (`Ctrl+C` / `Ctrl+V` in explorer context)
- [~] [M2+] Open File Beside (opens in the next group; needs M3 splits)
- [~] [M5] Open Terminal in Directory
- [~] [M2+] Reveal in system file manager (`xdg-open`) — needs a trust/confirmation decision

### Filesystem watching (§8.4)
- [x] [M2] notify watcher thread; non-recursive watches on expanded directories (D8)
- [~] [M2] Watches added on expand and removed on collapse; shared per canonical directory — partial: keyed by path, not canonical path
- [x] [M2] Coalescing window ~100 ms → one refresh per directory
- [x] [M2] Watch failure → notification plus a manual `Refresh Explorer` command; no polling
- [~] [M9] Git decorations in the Explorer (§8.5)

### Session persistence, part 1 (§4.9)
- [x] [M2] State file per workspace: `$XDG_STATE_HOME/yace/workspaces/<hash>.json` (atomic write)
- [x] [M2] Persist Explorer expansion per root, selection, scroll
- [x] [M2] Persist sidebar visibility and width
- [x] [M2] A corrupt state file is ignored with a warning and startup continues (§26.2)
- [x] [M2] Recent workspaces list (`recent.json`)

---

## M3 — Full basic editing (§9, §10, §29)

### Text buffer — `yace-buffer` (§9.2)
- [x] [M3] `Document`: rope, path, line-ending info, BOM flag, disk snapshot (mtime, len, hash), version, saved version → dirty
- [x] [M3] Insert, delete and replace ranges without whole-file copies
- [x] [M3] Line indexing (char ↔ line / column)
- [x] [M3] Criterion benchmarks: edits at start, middle and end of a 10 MB file; 1k-cursor typing; large undo (§9.2, §23.7)

### Unicode correctness (§9.3)
- [x] [M3] Distinguish bytes, chars, graphemes and display cells
- [x] [M3] Grapheme-aware cursor movement and deletion (Backspace removes one grapheme; ZWJ emoji stay whole)
- [x] [M3] Cell width via unicode-width; tabs expanded to tab stops
- [x] [M3] Horizontal scrolling measured in cells
- [x] [M3] Tests: CJK, emoji, ZWJ sequences, combining marks, RTL text (no corruption)

### Line endings and encoding (§9.4)
- [x] [M3] Detect LF / CRLF / Mixed; preserve byte-for-byte (D6)
- [x] [M3] New lines use the dominant ending (or `files.eol` when set)
- [x] [M3] Change EOL command (LF ↔ CRLF converts the whole document as one undo step)
- [x] [M3] UTF-8 with BOM preserved (D7)
- [x] [M3] Invalid UTF-8 / binary detection → refuse with a message (D7)
- [~] [M10+] Additional encodings (§9.4)

### Large files (§9.8)
- [x] [M3] Viewport-only rendering
- [x] [M3] Confirmation above the threshold (default 50 MB); large-file mode disables expensive decorations — files > 1 MiB load on a worker (M4); restoring a session still loads them synchronously
- [x] [M3] Saving still works in large-file mode

### Selections and multiple cursors (§9.5)
- [x] [M3] `Selection` of ranges (anchor, head) with a primary; normalised and merged
- [x] [M3] Selections mapped through every change (including edits made from other views)
- [x] [M3] `Ctrl+D` add next match; `Ctrl+Shift+L` select all matches
- [x] [M3] Add cursor above / below (`Ctrl+Shift+Up/Down`)
- [x] [M3] `Alt+Click` adds a cursor
- [x] [M3] Esc collapses to the primary cursor
- [x] [M3] Simultaneous insert and delete across cursors
- [x] [M3] Multi-selection-aware copy, cut and paste (one line per cursor when counts match)
- [x] [M3] Multi-cursor edit = one undo transaction
- [~] [M4+] Column / box selection (`Shift+Alt+drag`, `Ctrl+Shift+Alt+arrows`)

### Undo / redo (§9.1)
- [x] [M3] Transaction history with selection before and after
- [x] [M3] Coalescing of consecutive typing (word boundary or ~1 s gap)
- [x] [M3] `Ctrl+Z`, `Ctrl+Y`, `Ctrl+Shift+Z`
- [x] [M3] Dirty state correct after undoing back to the saved version

### Basic editing (§9.1)
- [x] [M3] Insert and delete text; Backspace / Delete; `Ctrl+Backspace` / `Ctrl+Delete` (word)
- [x] [M3] Arrow movement; `Ctrl+Left/Right` word movement; Home (smart) / End; `Ctrl+Home/End`; PgUp / PgDn
- [x] [M3] Shift variants of every movement (selection extension)
- [x] [M3] `Ctrl+A` select all; `Ctrl+L` select line
- [x] [M3] Sticky "desired column" for vertical movement
- [x] [M3] Clipboard: arboard → OSC 52 → internal register (D10); copy or cut of the whole line when nothing is selected
- [x] [M3] Bracketed paste inserted as one transaction
- [x] [M3] Save (`Ctrl+S`), Save As (`Ctrl+Shift+S`), Save All, Revert File
- [x] [M3] Unsaved-change indicators (tab dot, status bar)
- [x] [M3] Auto-save: `off` / `afterDelay` / `onFocusChange` (settings `files.autoSave`, `files.autoSaveDelay`)
- [x] [M3] Tab and indentation settings honoured (`tabSize`, `insertSpaces`)

### Intelligent editing (§9.6)
- [x] [M3] Auto-indent: a new line copies the previous indentation (+ one level after an opening bracket)
- [x] [M3] Tab / Shift+Tab indent and outdent selection
- [x] [M3] Auto-close brackets and quotes; type-over the closing char; delete the pair on Backspace
- [x] [M3] Matching-bracket highlight
- [x] [M3] Move line up / down (`Alt+Up/Down`)
- [x] [M3] Copy line up / down (`Shift+Alt+Up/Down`)
- [x] [M3] Delete line (`Ctrl+Shift+K`)
- [x] [M3] Insert line below / above (`Ctrl+Enter` / `Ctrl+Shift+Enter`)
- [x] [M3] Toggle line comment (`Ctrl+/`) via an extension → comment-token table (replaced in M7)
- [x] [M3] Trim trailing whitespace on save / insert final newline on save (settings)
- [~] [M3+] Toggle block comment (`Shift+Alt+A`)
- [~] [M7] Format document / selection (via LSP or formatter)
- [~] [M7] Code folding
- [~] [M7] Word wrap (§9.6)

### Safe saving (§9.9)
- [x] [M3] Write a temp file in the target's real directory, fsync, rename, fsync the directory
- [x] [M3] Preserve file permissions (mode)
- [x] [M3] Symlinks: write through to the target; the link stays a link
- [x] [M3] Pre-save check against the disk snapshot → Overwrite / Cancel prompt
- [x] [M3] Errors (ENOSPC, EACCES, EROFS) keep the buffer dirty and report clearly
- [x] [M3] New-file save (Save As of an untitled buffer) creates parent directories only after confirmation
- [x] [M3] Tests: atomicity, permissions, symlink, failure keeps the dirty state

### External modification (§9.10)
- [x] [M3] Watch the parent directories of open files
- [x] [M3] Clean buffer → silent reload, cursor clamped
- [x] [M3] Dirty buffer → sticky notification: Reload (discard mine) / Keep mine; keeping marks it conflicted
- [x] [M3] Deleted on disk → tab marked "(deleted)"; buffer stays editable and can be saved again
- [~] [M4+] Compare (diff view) for conflicts

### Tabs (§10.1)
- [x] [M3] Open in a tab; switch; close (`Ctrl+W`, prompts if dirty)
- [x] [M3] Close Others, Close to the Right, Close All, Close Saved
- [x] [M3] Reopen Closed Editor (`Ctrl+Shift+T`)
- [x] [M3] Next / Previous (`Ctrl+PgDn` / `Ctrl+PgUp`); `Alt+1..9`; `Ctrl+Tab` MRU cycling
- [x] [M3] Move tab left / right (`Ctrl+Shift+PgUp/PgDn`)
- [x] [M3] Dirty indicator; root-qualified title when names collide (§6.7)
- [x] [M3] Preview tabs (italic; replaced by the next preview; pinned on edit, double-click or Enter); setting `workbench.editor.enablePreview`
- [x] [M3] Tab bar overflow: scroll to keep the active tab visible
- [x] [M3] Mouse: click to activate, middle-click to close

### Split editors (§10.2)
- [x] [M3] Layout tree of groups (horizontal / vertical splits)
- [x] [M3] Split Right (`Ctrl+\`), Split Down
- [x] [M3] Focus group 1/2/3 (`Ctrl+1/2/3`), focus next / previous group
- [x] [M3] Move editor to the next / previous group
- [x] [M3] Same document in several groups; edits synchronised; independent cursor and scroll per view
- [x] [M3] Close group
- [x] [M3] Resize groups via commands and mouse drag
- [x] [M3] Maximise / restore group; reset layout (§3.2)

### Editor rendering (§22.3, §22.4)
- [x] [M3] Viewport-only line rendering
- [x] [M3] Gutter line numbers (absolute; relative is a setting)
- [x] [M3] Current-line highlight
- [x] [M3] Selection highlight across ranges
- [x] [M3] Primary cursor = terminal cursor; secondary cursors drawn reversed
- [x] [M3] Horizontal scroll and cursor-follow scrolling with margins
- [x] [M3] Vertical scrollbar
- [x] [M3] Mouse: click to place the cursor, drag to select, double-click word, triple-click line, wheel scroll

### Status bar (§3.1)
- [x] [M3] Root name, relative path, `Ln x, Col y` (grapheme columns), selection count
- [x] [M3] Language (by extension), EOL (LF / CRLF / Mixed), encoding (UTF-8 / UTF-8 BOM), indent (Spaces: 4 / Tab)
- [x] [M3] Chord pending, notifications
- [~] [M9] Git branch

### Unsaved-change management
- [x] [M3] Close tab, close group, close workspace, quit → per-file Save / Don't Save / Cancel; "Save All" option
- [x] [M3] Quit (`Ctrl+Q`, `workbench.action.quit`)
- [x] [M3] SIGTERM / SIGHUP: persist the session; never discard silently
- [~] [M10] Hot-exit backups of dirty buffers for crash recovery (§26.1)

### Session persistence, part 2 (§4.9, Workflow G)
- [x] [M3] Open tabs per group, tab order, active group and tab, preview state
- [x] [M3] Split tree and sizes (clamped to the current terminal size)
- [x] [M3] Per-view cursor and scroll positions
- [x] [M3] Recently opened files list (used by Quick Open ranking in M4)
- [x] [M3] Missing files skipped on restore with a notice

### Integration and end-to-end tests (§28)
- [x] [M3] Multi-root fixture: 3 `TempDir`s with no common parent, plus a `.code-workspace` stored elsewhere (comments, trailing comma, relative path, missing root) (§28.5)
- [x] [M3] Every root appears and expands; missing root doesn't block the others
- [x] [M3] Open files across roots; duplicate `index.ts` disambiguated
- [x] [M3] Edit → save → bytes on disk are correct
- [x] [M3] External modification detected
- [x] [M3] Removing a root leaves its files intact
- [x] [M3] Session restores tabs, cursors and splits
- [x] [M3] Workspace settings (tabSize) apply to files in the right root
- [x] [M3] PTY end-to-end smoke test of the real binary (`portable-pty`): open, navigate, type, save, quit; terminal restored
- [x] [M3] Keybinding dispatch tests per focus context (§28.4)

---

## M4 — Quick Open and searching (§6, §7, §9.7, §14)

### Quick Open (`Ctrl+P`) (§6)
- [x] [M4] Floating overlay near the top; editor stays visible (§6.1)
- [x] [M4] Opens immediately; never waits for a full scan (§6.2, §32.4)
- [x] [M4] Background file discovery per root (`ignore` crate), streaming results progressively (§6.5)
- [x] [M4] Results from finished roots usable while other roots continue
- [x] [M4] Fuzzy matching (e.g. `jobserv` → `jobs.service.ts`) (§6.3)
- [x] [M4] Ranking: exact name, prefix, sequence quality, path segments, recently opened, recently selected, active root, depth (§6.3)
- [x] [M4] Deterministic ranking
- [x] [M4] Bounded result set; no unlimited allocation (§6.5)
- [~] [M4] Result rows: filename, parent dir, ROOT NAME, file-type glyph, match highlighting (§4.5, §6.7) — filename (fuzzy-highlighted), parent dir and ROOT NAME shown; file-type glyphs arrive with M6 icon themes
- [x] [M4] Collisions clearly disambiguated (§6.7)
- [x] [M4] Exclusions: `files.exclude`, `search.exclude`, `.gitignore`; toggle to include ignored or hidden (§6.6)
- [~] [M4] Mode prefixes: `>` commands, `@` symbols in file, `#` workspace symbols, `:N` go to line, `file:L`, `file:L:C`, `?` help (§6.4) — `>`, `:N`, `file:L[:C]`, `?` done; `@`/`#` show a notice until language support (M7)
- [x] [M4] Cancellation of outdated queries (§21.3, §32.4)
- [x] [M4] Lightweight cached index metadata for faster warm start; never treated as authoritative (§23.4)
- [~] [M4] Index freshness via fs events / bounded refresh — re-walk when Quick Open opens and the index is >30 s old; fs events patch it in between (only for watched dirs)

### Command palette (full) (§7)
- [x] [M4] Shares the Picker component with Quick Open
- [x] [M4] Fuzzy search by title and by ID; recently used first
- [x] [M4] Keybinding shown per command

### Go to Line (`Ctrl+G`)
- [x] [M4] `:line[:col]` prompt; out-of-range clamped

### Navigation history (§10.3)
- [x] [M4] Back / Forward across files and roots (`Ctrl+Alt+-`, `Ctrl+Shift+-`)

### Find in file (`Ctrl+F`, `Ctrl+H`) (§9.7)
- [x] [M4] Incremental search; next / previous; match count; highlight
- [x] [M4] Case sensitive, whole word, regex toggles
- [x] [M4] Replace one, replace all (one undo step)
- [x] [M4] Search within selection
- [x] [M4] Search history
- [~] [M4] No full-document reprocessing per keystroke (incremental / viewport-first) — matching runs on a worker against an O(1) rope snapshot, so typing never blocks; it is not yet incremental

### Workspace search (`Ctrl+Shift+F`) (§4.6, §14)
- [x] [M4] Plain, case, whole word, regex
- [x] [M4] Include / exclude globs; scope to all roots, one root, selected roots, a directory — globs, `./Root Name/…` scoping and Find in Folder done; multi-root selection via several `./Root` entries
- [x] [M4] Results grouped by root then file; preview lines; navigate to match
- [x] [M4] Respect ignore rules; skip binaries; streamed; cancellable; bounded memory (§14.2)
- [x] [M4] No full-text index by default (§14.2)
- [~] [M4] Replace in files with preview; per-file and per-match include / exclude (§14.3) — matches listed with per-match/per-file exclusion and a confirmation; an inline before/after diff preview is not yet shown
- [x] [M4] Replace applies to open buffers as edits, never overwriting unsaved buffers silently (§14.3)
- [x] [M4] Search results view in the bottom panel or sidebar

---

## M5 — Integrated terminal (§11)

- [ ] [M5] Decide the emulator library (open question)
- [ ] [M5] PTY via `portable-pty` (§11.2)
- [ ] [M5] Terminal emulation: ANSI, cursor, colours, scrolling, alternate screen, input modes, UTF-8, bracketed paste, mouse reporting (§11.3)
- [ ] [M5] Bottom panel hosting terminals; `Ctrl+J` panel toggle, `` Ctrl+` `` terminal toggle (with legacy fallback)
- [ ] [M5] Multiple sessions with tabs, names, close, restart (§11.4)
- [ ] [M5] Independent cwd, env, scrollback per session
- [ ] [M5] Commands: Open Terminal in Workspace Root, Open Terminal Here, in Selected File Directory, for Active Repository (§4.8)
- [ ] [M5] Terminal tab shows its root identity (§4.8)
- [ ] [M5] Profiles: shell, args, env, cwd policy, name, startup command; default = `$SHELL` (§11.5)
- [ ] [M5] Focus-based input routing; Ctrl+C sends an interrupt in the terminal; Esc passes through (§11.6)
- [ ] [M5] Configurable list of commands that "skip the shell" (editor shortcuts while the terminal is focused)
- [ ] [M5] Async output reading; coalesced redraws; bounded scrollback (configurable) (§11.7)
- [ ] [M5] Thousands of lines/s of output must not degrade editor typing (benchmark) (§11.7)
- [ ] [M5] Resize propagates to the PTY (§3.3)
- [ ] [M5] Lifecycle: exit, failed spawn, crash, closing with a running process (confirm), app quit policy, broken pipe; no orphaned processes (§11.8)
- [ ] [M5] Session restore: layout and metadata only; never pretends a process survived (§4.9)
- [ ] [M5] Split terminal views (desirable)
- [ ] [M5] Terminal scrollback search / copy mode
- [ ] [M5] PTY integration tests: shell startup, input, ANSI, resize, alt screen, Ctrl+C, exit, high volume, scrollback limit, concurrency (§28.3)
- [ ] [M5] Manual: Claude Code runs correctly inside the integrated terminal (Workflow D)

---

## M6 — Themes and customisation (§16, §17)

- [ ] [M6] TOML theme schema covering every §16.2 token, including syntax and semantic categories
- [ ] [M6] Theme inheritance (`extends = "yace-dark"`) (§16.4)
- [ ] [M6] User colour overrides in `config.toml` (§16.4)
- [ ] [M6] Built-in themes: YACE Dark, YACE Light, One Dark-style, Dracula-style, Catppuccin-style, Tokyo Night-style (licence-checked) (§16.1)
- [ ] [M6] Theme picker (`Ctrl+K Ctrl+T`) with search and live preview; Esc reverts (§16.3)
- [ ] [M6] Theme persistence per chosen scope (user or workspace)
- [ ] [M6] Invalid theme → clear error, fall back to the previous theme (§32.6)
- [ ] [M6] Monochrome / minimal-colour fallback (§16.5)
- [ ] [M6] Docs: fonts belong to the terminal emulator (§16.6)
- [ ] [M6] Keybinding editor UI (`Ctrl+K Ctrl+S`): search, conflicts, record, restore defaults, export (§5.4)
- [ ] [M6] Live reload of config, keybindings and themes (§17.5)
- [ ] [M6] Settings commands: Open User Settings, Open Workspace Settings, Open Keybindings File (§17.6)
- [ ] [M6] Layout persistence and reset (§3.2)
- [ ] [M6] Responsive layout: collapse sidebar and panel on small terminals; never panic on 1×1 (§3.3)
- [~] [M6+] VS Code colour theme import, data only, no JS (§16.7)

---

## M7 — Syntax and development intelligence (§12, §13)

### Syntax (§12)
- [ ] [M7] Language detection (extension, filename, shebang)
- [ ] [M7] Tree-sitter integration; grammars loaded lazily per language (§12.2)
- [ ] [M7] Incremental reparse on edits, off the input path (§12.1)
- [ ] [M7] Viewport-first highlighting for large files
- [ ] [M7] Highlight cache invalidated by edit ranges (§22.4)
- [ ] [M7] Languages: Rust, JS, TS, TSX, JSX, JSON, JSONC, YAML, TOML, HTML, CSS, SCSS, Markdown, SQL, Bash, Python, Go, Dockerfile, git config, plain text (§12.2)
- [ ] [M7] Embedded languages (injections) (§12.3)
- [ ] [M7] Comment tokens, brackets and indentation rules from language config (replaces the M3 table)
- [ ] [M7] Code folding from syntax tree
- [ ] [M7] Word wrap

### LSP (§13)
- [ ] [M7] `yace-lsp` crate: JSON-RPC over stdio, own reader/writer threads
- [ ] [M7] Lazy start based on active file, language, config, availability (§13.2)
- [ ] [M7] Shutdown when idle (configurable reuse policy)
- [ ] [M7] Multi-root: `workspaceFolders` for servers that support it, else one instance per root (§13.3)
- [ ] [M7] Never share server state across unrelated repositories
- [ ] [M7] Features: completion, hover, diagnostics, definition, declaration, references, implementation, rename, document symbols, workspace symbols, signature help, code actions, formatting, semantic tokens (§13.1)
- [ ] [M7] UI reflects the server's capabilities (§13.1)
- [ ] [M7] Crash / hang / init failure isolation; restart and disable commands (§13.5)
- [ ] [M7] Node-based servers treated as external tools; documented; excluded from core memory benchmarks (§13.4)
- [ ] [M7] Diagnostics panel; gutter markers; status counts
- [ ] [M7] `Ctrl+Space`, `F12`, `F2`, `Ctrl+Shift+O`, `Ctrl+T` wired
- [ ] [M7] Format on save (setting)

---

## M8 — Native plugin system (§18)

- [ ] [M8] Decide in-process vs out-of-process (open question)
- [ ] [M8] `yace-plugin-api`: C ABI, versioned function tables, struct sizes, optional capabilities, error codes, allocation ownership, UTF-8 strings, lifetimes, threading rules, cancellation (§18.8)
- [ ] [M8] `yace-plugin-host`: discovery, manifest validation (before loading code), ABI checking, `dlopen`, activation, cleanup (§18.5)
- [ ] [M8] Manifest format (TOML): id, name, version, description, author, OS/arch, min YACE version, ABI version, entry library, commands, keybindings, config schema, activation events, capabilities, dependencies (§18.7)
- [ ] [M8] Lazy activation triggers: command, file type, view, event (§18.6)
- [ ] [M8] Contributions: commands, keybindings, themes, status bar items, panels, explorer decorations, Quick Open providers, settings (§18.3)
- [ ] [M8] Event API with coalescing / rate limiting (§18.4)
- [ ] [M8] Non-blocking callbacks; time budget measured; background job API (§18.10)
- [ ] [M8] Controlled UI model; no raw ANSI from plugins (§18.12)
- [ ] [M8] Plugin settings with validation and defaults (§18.11)
- [ ] [M8] Enable / disable globally and per workspace; safe mode (`--safe-mode`, `--no-plugins`) (§18.11)
- [ ] [M8] Plugin manager: install from local package or directory, remove, enable, disable, inspect, diagnose (§18.13)
- [ ] [M8] Commands: Extensions: Install / Uninstall / Enable / Disable / Update / Reload / List Installed / Show Dependencies / Show Performance / Open Plugin Settings (§18.14)
- [ ] [M8] Trust UI: identity, publisher, source, version, capabilities, hashes; explicit confirmation (§18.9, §27.2)
- [ ] [M8] Docs state honestly that in-process plugins are not sandboxed (§33.11)
- [ ] [M8] SDK: Rust bindings, example plugin, packaging tool, test harness, API reference, release checklist (§18.15)
- [ ] [M8] Tests: valid load, invalid manifest, unsupported ABI, missing library, activation, deactivation, registration, settings, failure, performance, safe mode, malformed ABI inputs (§28.6)
- [~] [M8+] Install from Git or release artifact; registry (optional) (§18.13, §34.10)

---

## M9 — Git and advanced workflow (§15)

- [ ] [M9] `yace-git` crate; per-root repository detection (§4.7)
- [ ] [M9] Current branch per root in the status bar for the active root
- [ ] [M9] Working tree status (modified, added, deleted, untracked, ignored, conflicted), lazily and independently scheduled per root (§15.3)
- [ ] [M9] Explorer decorations scoped to the right repository (§8.5)
- [ ] [M9] Git status view; staged and unstaged diffs (§15.1)
- [ ] [M9] Refresh triggers: fs events in `.git`, focus regain, bounded interval only when visible
- [ ] [M9] Read-only inspection automatic; mutations only on explicit user action (§15.4)
- [ ] [M9] Never run checkout / commit / reset on open (§15.4)
- [ ] [M9] Diff navigation in editor gutter
- [~] [M9+] Stage / unstage file and hunk, commit, branch switch / create, history, compare, stash (§15.2)
- [ ] [M9] Test: branches in two roots stay independent (§28.5)

---

## M10 — Public release preparation (§25, §26)

### Reliability (§26)
- [ ] [M10] Hot-exit backups of dirty buffers; recovery on next start; never overwrite newer disk content (§26.1)
- [ ] [M10] Fault isolation audit for every optional subsystem (§26.4)
- [ ] [M10] Diagnostics report command (§26.5)

### Packaging (§25)
- [ ] [M10] Release binaries for x86_64 and aarch64 Linux (§25.8)
- [~] [M10] GitHub Releases: archives, checksums, licence, docs, release notes (§25.1) — automated in `release.yml`; untested until the repository exists
- [ ] [M10] Debian package (`cargo-deb`): correct deps, docs, upgrade, removal keeps user config (§25.2)
- [ ] [M10] Signed APT repository (scoped `signed-by` keys) or PPA (§25.3, §25.4)
- [ ] [M10] Snap with classic confinement; submit for review; never claim approval until granted (§25.5, §25.6)
- [ ] [M10] Release channels: dev, nightly, beta, stable (§25.9)
- [x] [M10] SemVer, with plugin ABI versioned separately (§25.10) — policy in `docs/releasing.md`; the plugin ABI version arrives with M8
- [~] [M10] GitHub Actions release pipeline (14 stages, §25.11) — `.github/workflows/ci.yml` and `release.yml` prepared (verify → checks → x86_64/aarch64 builds → checksums → GitHub Release); not yet run (no GitHub repository); Debian/APT/Snap stages pending
- [ ] [M10] Installation tests: install, launch, workspace, edit, terminal, plugins, config persistence, upgrade, removal, snap behaviour (§25.12)
- [ ] [M10] No self-updater (§27.6)
- [x] [M10] Credits and third-party licence notices: `CREDITS.md`, generated `THIRD_PARTY_LICENSES.md` (`scripts/third_party.py`, regenerated by `release.sh` and per target in `release.yml`), shipped in release archives; About dialog credits
- [ ] [M10] Include `THIRD_PARTY_LICENSES.md` / copyright file in the Debian package and Snap
- [x] [M10] CHANGELOG process — `CHANGELOG.md` (Keep a Changelog), `scripts/release.sh`, annotated `vX.Y.Z` tags

### Performance (§23)
- [ ] [M10] Benchmark suite: cold and warm start, workspace load, directory expansion, Quick Open, typing latency, scrolling, large file, multi-cursor, syntax, search, git, terminal throughput, plugin activation, memory (§23.7)
- [ ] [M10] Separate memory categories: core / child process / plugin / caches (§23.2)
- [ ] [M10] CI regression tracking where measurements are stable (§23.8)
- [ ] [M10] Minimal / reduced-feature mode (§23.9)
- [ ] [M10] Terminal compatibility matrix: GNOME Terminal, Kitty, Alacritty, WezTerm, Foot, Konsole, tmux (§22.5)

### Measured results vs §23.1 targets

Memory after Quick Open + a workspace search on a real 9-folder workspace (`scripts/bench_memory.py`, 2026-11-03): v0.2.1 defaults 20.0 MiB (peak 21.5 MiB) with search-as-you-type; with `.claude/worktrees` included 23.9 MiB; without heap trimming (`YACE_MALLOC_TRIM=0`, ≈ v0.2.0) 33.2 MiB. A 108 MiB report from an interactive v0.2.0 session was not reproduced.

Search benchmarks (`cargo bench -p yace-search`, 2026-11-03, i5-1135G7): Quick Open query over 200k paths 9–19 ms (parallel, top-200); walk of this repo 1.5 ms; content search of this repo 3.5 ms. After M4, startup 5.4 ms median and 6.4 MiB at first frame (indexing is lazy).

Buffer benchmarks (`cargo bench -p yace-buffer`, 2026-11-03, i5-1135G7): type 1 char in a 10 MB file 0.76 µs (start) / 1.01 µs (middle) / 1.15 µs (end); type with 1,000 cursors 2.0 ms; undo+redo of a 100k-change transaction 55.6 ms.
| Metric | Target | Measured | Date / hardware |
|---|---|---|---|
| Cold start to visible UI | ≤ 50 ms | **5.1 ms median, 5.7 ms p95** (20 runs, `scripts/bench.sh`, enhanced-keyboard probe disabled) | 2026-11-03 · i5-1135G7 · Linux 6.11 |
| Idle RSS (minimal) | ≤ 20 MB | **5.4 MiB** at first frame (editor process only) | 2026-11-03 · same |
| Typing → render p95 | ≤ 5 ms | **≈0.4 ms**: input handling p95 0.24 ms + render p95 0.18 ms (300 keys into a 6,000-line file, `scripts/bench_interactive.py`); excludes terminal emulator paint time | 2026-11-03 · i5-1135G7 · Linux 6.11 |
| Idle CPU | ~0 | **0.00%** of one core over 5 s idle (`scripts/bench_interactive.py`) | 2026-11-03 · same |

---

## Known gaps after the M1–M3 pass

- [x] Large files are read synchronously; move loading to a worker (§9.8) — done in M4 (except session restore)
- [x] Automated tests for mouse handling, auto-save, Save All within the close flow, Save As during quit, Reopen Closed Editor (`tests/flows.rs`)
- [ ] Manual check in GNOME Terminal, Kitty, Alacritty, WezTerm, foot, Konsole, tmux (§22.5) — only a PTY test so far
- [x] Measure typing→render latency and idle CPU end-to-end (`scripts/bench_interactive.py`)
- [ ] Hot-exit backups (M10) — a crash currently loses unsaved edits
- [x] Initial commit and `v0.1.0` tag; licence decided (MIT OR Apache-2.0)
- [ ] Create the GitHub repository, push, and run the workflows once (`docs/releasing.md` → Before the first public push)

## Cross-cutting requirements (must hold at every milestone)

- [x] No JavaScript runtime anywhere in YACE's implementation or plugins (§2.2, §33.2) — holds as of M1–M3
- [x] Works fully offline; no account; no telemetry by default (§2.7, §27.1, §27.5) — holds as of M1–M3
- [~] No unbounded queues, caches, scrollback, results or watchers (§21.5, §33.7) — the fs job queue, watches and history are bounded; the main event channel is unbounded
- [x] Nothing heavy at startup: no full scans, no LSPs, no grammars, no plugins, no git status, no recursive reads (§23.3, §33.3) — holds as of M1–M3
- [x] Every root-aware subsystem respects root boundaries (§2.4, §33.4) — holds as of M1–M3
- [x] Errors are always surfaced, never silently swallowed (§33.8) — holds as of M1–M3
- [x] No performance claims without reproducible benchmarks (§33.9) — holds as of M1–M3
- [x] No claim that every shortcut works in every terminal (§33.10) — holds as of M1–M3
- [x] VS Code-style non-modal editing is the default (§33.5) — holds as of M1–M3
