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
| Current pass | M1–M3 → **v0.1.0**; M4 → **v0.2.0** (+ v0.2.1 polish); M5 (integrated terminal) → **v0.3.0** (+ v0.3.1); M7 (syntax + language servers) → **v0.4.0** |
| Current step | **M7 complete** — released as v0.4.0; next: M6 (themes and customisation) or M8 (plugins; ask about downloadable grammars first) |
| Toolchain | rustc 1.99.1 stable (rustup) |
| Last updated | 2026-11-03 (M7 complete) |

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
| D19 | Integrated terminal on `alacritty_terminal` 0.26 (Apache-2.0): complete VT emulation, bounded grid, selection, its own PTY + event loop; `portable-pty` only in tests | mature (Alacritty, Zed), small dependency tree; vt100 lacks a PTY and full-screen fidelity; termwiz is large | §11.2, §11.3, §34.2 |
| D21 | Terminal settings are user-scope only | a workspace must never choose which program runs | §4.12, §27.3 |
| D22 | Tree-sitter grammars compiled into the binary, constructed and their queries compiled only when a file of that language opens | no runtime downloads; unused grammars cost disk but no RAM. Superseded in part by D29 (M8) | §12.2 |
| D23 | External language servers allowed, including Node-based ones (user's choice); YACE never bundles or installs them and reports their memory separately | best-in-class TypeScript support needs tsserver; YACE itself stays JS-free | §2.2, §13.4, §34.7 |
| D24 | Own minimal LSP types + typed decoding on the reader thread, instead of `lsp-types` | smaller binary; large replies (completion lists) never parsed on the UI thread; lenient parsing of the shapes servers really send | §13, §21.4 |
| D25 | One server instance per (server, workspace root), `workspaceFolders` = that root | never share server state across unrelated repositories; simple and correct for every server | §13.3 |
| D30 | Git through the installed `git` CLI on two worker threads, not a Git library (user's choice) | behaves exactly like the user's git (config, hooks, LFS, worktrees); no large dependency (+0.3 MB binary); reads use `--no-optional-locks`; status of 9 real repos in 0.2 s total | §15 |
| D28 | Plugins are in-process native libraries behind a versioned C ABI (`yace-plugin-api`), with JSON for structured data; out-of-process host left for later (user's choice) | fastest calls (76 ns) and no per-plugin process memory; honest that they are not sandboxed; JSON keeps the ABI small and append-only | §18.1, §18.8, §18.9 |
| D29 | Hybrid grammars (user's choice): Rust, JS/TS/TSX, JSON, YAML, TOML, HTML, CSS, Markdown, Python, Bash built in; C, C++, Go, Java, Ruby, SCSS, SQL, Dockerfile in the installable `yace-grammars` package, loaded on first use; `all-grammars` feature compiles all in | binary 25.4 → 17.6 MB; the package is 1.0 MB compressed | §12.2, §18.3 |
| D27 | Theme files use VS Code colour ids for UI keys and tree-sitter scope names for syntax; built-ins compiled from their TOML at build time via a generated, test-checked module | one mapping table serves themes, overrides and VS Code import; startup parses no theme (measured 40–90 µs warm / ≤0.3 ms cold → 0.8 µs) | §16.2, §23.1 |
| D26 | Syntax reparse placement predicted from a per-document cost estimate (inline < 0.5 ms, else worker) instead of "try inline with a budget" | a budget that runs out wastes its time on every keystroke (measured p50 2.2 ms → 0.074 ms) | §12.1, §21.4 |
| D13 | State files live under `$XDG_STATE_HOME/yace`, config under `$XDG_CONFIG_HOME/yace`, cache under `$XDG_CACHE_HOME/yace`. Every state write is atomic (temp + rename) | §17.2, §26.2 | §17.2 |

### Open questions (spec §34, user decisions pending)
- [x] Licence (§34.12) — **MIT OR Apache-2.0** (`LICENSE-MIT`, `LICENSE-APACHE`), decided 2026-11-03
- [~] Contribution guidelines, maintainers, security reporting, plugin API governance (§34.12) — `CONTRIBUTING.md` and `SECURITY.md` written; maintainers, code of conduct and plugin API governance still open
- [x] Policy for external Node-based language servers such as the TypeScript one (§2.2, §13.4, §34.7) — allowed as external tools, never bundled or installed, memory reported separately (D23)
- [x] Plugin execution model: in-process C ABI; out-of-process left as an option (D28) (§18.9, §34.3)
- [x] Plugin ABI scope for v1: `docs/plugins.md` → ABI reference (§34.4)
- [x] Search engine: pure Rust (`ignore` + `grep-searcher`) vs an external `rg` (§34.5) — pure Rust (D17)
- [x] Integrated terminal emulator library choice (§11.3, §34.2) — `alacritty_terminal` (D19)
- [ ] Platform scope beyond Linux (§34.8)
- [x] Policy for importing VS Code themes: colour data only, no extension code (`docs/themes.md`) (§16.7, §34.9)

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
- [x] [M1] Job generation tokens so stale results are dropped (cancellation, §21.3) — M4 workers (index, search, find) use sequence numbers and cancel flags; directory reads run one at a time per folder and a request during a read re-reads afterwards, so the newest listing always arrives last
- [x] [M1] Input is handled before background results in the same wakeup (priorities, §21.4)

### App core
- [x] [M1] `App` state is terminal-agnostic; `App::handle(AppEvent)`; `ui::draw(&App, &mut Frame)` is pure
- [x] [M1] Headless test harness: drive `App` with synthetic key events, render to ratatui `TestBackend`
- [x] [M1] Error type plus a notification surface (transient status-bar message, sticky notifications, last-error log) (§33.8)
- [x] [M1] File logging via `tracing`, only when `YACE_LOG` is set; never logs file contents or environment (§27.4)

### Command registry (§7, §19)
- [x] [M1] `CommandId` (VS Code IDs where they exist), title, category, availability predicate, handler
- [x] [M1] Typed arguments (`serde_json::Value`-backed), result and error reporting (§19.1) — `run_command` returns `CommandError` (unknown, invalid arguments, unavailable, failed); `type`, `insertSnippet`, `terminal.sendSequence`, `quickOpen`, `gotoLine`, `files.openFile`/`vscode.open`, `git.checkout` take arguments; bad `args` in keybindings.toml are reported naming the file; plugin `executeCommand` failures go to the plugin's log
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
- [x] [M1] Commands: Focus Explorer (`Ctrl+Shift+E`), Focus Active Editor Group, Focus Next/Previous Part (`F6` / `Shift+F6`; since M7 a real cycle Editor → Panel → Sidebar, skipping hidden parts)

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
- [x] [M6] Live reload of config and keybinding files (§17.5) — see M6

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
- [x] [M8] `--safe-mode`, `--no-plugins`
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
- [x] [M2+] Copy / cut / paste of files and folders in the Explorer (`Ctrl+C` / `Ctrl+X` / `Ctrl+V`): background copy, VS Code "copy" names, symlinks kept as links, no folder into itself, open documents follow a cut
- [x] [M2+] Open File Beside (`Ctrl+Enter` in the Explorer and Quick Open; splits when there is no group to the right)
- [x] [M5] Open Terminal in Directory — Explorer "Open in Integrated Terminal"
- [x] [M2+] Reveal in File Manager (`Shift+Alt+R`): opens the containing folder with `xdg-open` (detached, own process group), never the file

### Filesystem watching (§8.4)
- [x] [M2] notify watcher thread; non-recursive watches on expanded directories (D8)
- [x] [M2] Watches added on expand and removed on collapse; shared per canonical directory — one watch per real directory, events reported under every path it is shown as (symlinked roots/folders)
- [x] [M2] Coalescing window ~100 ms → one refresh per directory
- [x] [M2] Watch failure → notification plus a manual `Refresh Explorer` command; no polling
- [x] [M9] Git decorations in the Explorer (§8.5)

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
- [x] [M7] Toggle block comment (`Shift+Alt+A`)
- [x] [M7] Format document / selection (via LSP)
- [x] [M7] Code folding
- [x] [M7] Word wrap (§9.6)

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
- [~] [M9+] Compare (diff view) for conflicts — conflicts are listed in Source Control; a side-by-side/merge view is not built

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
- [x] [M9] Git branch

### Unsaved-change management
- [x] [M3] Close tab, close group, close workspace, quit → per-file Save / Don't Save / Cancel; "Save All" option
- [x] [M3] Quit (`Ctrl+Q`, `workbench.action.quit`)
- [x] [M3] SIGTERM / SIGHUP: persist the session; never discard silently
- [ ] [M10] Hot-exit backups of dirty buffers for crash recovery (§26.1)

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
- [x] [M4] Result rows: filename, parent dir, ROOT NAME, file-type glyph, match highlighting (§4.5, §6.7) — icons in Nerd Font and Unicode modes (`workbench.fileIcons`)
- [x] [M4] Collisions clearly disambiguated (§6.7)
- [x] [M4] Exclusions: `files.exclude`, `search.exclude`, `.gitignore`; toggle to include ignored or hidden (§6.6)
- [x] [M4] Mode prefixes: `>` commands, `@` symbols in file, `#` workspace symbols, `:N` go to line, `file:L`, `file:L:C`, `?` help, plus plugin prefixes (§6.4)
- [x] [M4] Cancellation of outdated queries (§21.3, §32.4)
- [x] [M4] Lightweight cached index metadata for faster warm start; never treated as authoritative (§23.4)
- [x] [M4] Index freshness via fs events — every indexed folder is watched (within half of `fs.inotify.max_user_watches`; a root over budget keeps the 30 s re-walk on Quick Open, with one notice); new folders are walked in place, new files checked against every ancestor ignore rule, deletions dropped, ignore-file changes and watcher overflow re-walk; still lazy (starts on the first Quick Open) — v0.9.0

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
- [x] [M4] No full-document reprocessing per keystroke — Find follows edits: matches move through the `ChangeSet` and only the edited lines (±1) are searched again, on the UI thread up to 2,000 lines, for queries that can't span lines; whole-file searches read the rope in 64 KiB pieces (no copy) — v0.9.0

### Workspace search (`Ctrl+Shift+F`) (§4.6, §14)
- [x] [M4] Plain, case, whole word, regex
- [x] [M4] Include / exclude globs; scope to all roots, one root, selected roots, a directory — globs, `./Root Name/…` scoping and Find in Folder done; multi-root selection via several `./Root` entries
- [x] [M4] Results grouped by root then file; preview lines; navigate to match
- [x] [M4] Respect ignore rules; skip binaries; streamed; cancellable; bounded memory (§14.2)
- [x] [M4] No full-text index by default (§14.2)
- [x] [M4] Replace in files with preview; per-file and per-match include / exclude (§14.3) — inline old→new on each result row while the replace field is shown; Enter opens a read-only Replace Preview diff of the file (imara-diff unified); preview and Replace All share `content::plan_replacements` — v0.9.0
- [x] [M4] Replace applies to open buffers as edits, never overwriting unsaved buffers silently (§14.3)
- [x] [M4] Search results view in the bottom panel or sidebar

---

## M5 — Integrated terminal (§11)

- [x] [M5] Decide the emulator library (open question) — `alacritty_terminal` (D19)
- [x] [M5] PTY (§11.2) — `alacritty_terminal::tty` (rustix-openpty) rather than `portable-pty` (D19)
- [x] [M5] Terminal emulation: ANSI, cursor, colours, scrolling, alternate screen, input modes, UTF-8, bracketed paste, mouse reporting (§11.3)
- [x] [M5] Bottom panel hosting terminals; `Ctrl+J` panel toggle, `` Ctrl+` `` terminal toggle (with legacy fallback)
- [x] [M5] Multiple sessions with tabs, names, close, restart (§11.4)
- [x] [M5] Independent cwd, env, scrollback per session
- [x] [M5] Commands: Open Terminal in Workspace Root, Open Terminal Here, in Selected File Directory, for Active Repository (§4.8)
- [x] [M5] Terminal tab shows its root identity (§4.8)
- [x] [M5] Profiles: shell, args, env, cwd policy, name, startup command; default = `$SHELL` (§11.5)
- [x] [M5] Focus-based input routing; Ctrl+C sends an interrupt in the terminal; Esc passes through (§11.6)
- [x] [M5] Configurable list of commands that "skip the shell" (editor shortcuts while the terminal is focused)
- [x] [M5] Async output reading; coalesced redraws; bounded scrollback (configurable) (§11.7)
- [x] [M5] Thousands of lines/s of output must not degrade editor typing (benchmark) (§11.7) — `seq 1 2000000` while typing: editor input p95 0.47 ms, render p95 0.13 ms (`scripts/bench_terminal.py`)
- [x] [M5] Resize propagates to the PTY (§3.3)
- [x] [M5] Lifecycle: exit, failed spawn, crash, closing with a running process (confirm), app quit policy, broken pipe; no orphaned processes (§11.8)
- [x] [M5] Session restore: layout and metadata only; never pretends a process survived (§4.9)
- [x] [M5] Split terminal views — groups of panes side by side (`Ctrl+Shift+5` / `Alt+\`), per-pane PTY size, `Alt+arrows` focus and `Ctrl+Shift+Left/Right` or drag to resize (keys reach the shell when not split), one tab per group, unsplit, restored with the session — v0.10.0
- [x] [M5] Terminal scrollback search — `Ctrl+F` bar (case, whole word, regex, count to 1000+), alacritty's regex DFAs on a worker in 1,000-line chunks so the PTY never stalls; the match is selected for copying — v0.10.0
- [x] [M5] PTY integration tests: shell startup, input, ANSI, resize, alt screen, Ctrl+C, exit, high volume, scrollback limit, concurrency (§28.3)
- [~] [M5] Manual: Claude Code runs correctly inside the integrated terminal (Workflow D) — awaiting the user's manual check

---

## M6 — Themes and customisation (§16, §17)

- [x] [M6] TOML theme schema covering every §16.2 token: UI keys are VS Code colour ids (64 keys incl. the 16 terminal colours, aliases for other VS Code ids), `[syntax]` by scope name with bold/italic/underline, `[semantic]` stored for later; `#rgb`/`#rrggbb`/`#rrggbbaa` (alpha blended), ANSI names, `ansi:N` (`crates/yace/src/theme/schema.rs`, `docs/themes.md` kept in sync by a test)
- [x] [M6] Theme inheritance (`extends`, any depth, cycles reported; no `extends` → YACE Dark/Light by `type`; background/foreground changes carry to derived keys) (§16.4)
- [x] [M6] User colour overrides in `config.toml`: `workbench.colorCustomizations` and `editor.tokenColorCustomizations` (VS Code shapes incl. `"[Theme]"` sections and `textMateRules`), merged across layers (§16.4)
- [x] [M6] Built-in themes: YACE Dark, YACE Light, One Dark, Dracula, Catppuccin Mocha, Tokyo Night — MIT palettes credited in CREDITS.md and THIRD_PARTY_LICENSES.md; compiled from their TOML files (`builtin_gen.rs`, regenerated by a test) so startup parses nothing (§16.1)
- [x] [M6] Theme picker (`Ctrl+K Ctrl+T`) with search and live preview; Esc (or closing any other way) reverts (§16.3)
- [~] [M6] Theme persistence: saved to user `config.toml` with comments kept (`toml_edit`); a workspace that sets its own theme keeps it (YACE says so). Writing the choice into a workspace file: not yet
- [x] [M6] Invalid theme → error with file/line/key, previous theme stays; at startup falls back to YACE Dark (§32.6)
- [x] [M6] Monochrome / minimal-colour fallback: selection, cursor, list rows, tabs, status bar, matches, brackets and diagnostics use reverse/bold/underline when there is no colour (§16.5)
- [x] [M6] Docs: fonts belong to the terminal emulator (`docs/themes.md`, `docs/configuration.md`) (§16.6)
- [x] [M6] Keybinding editor UI (`Ctrl+K Ctrl+S`): every command incl. unbound, search by text/`@conflicts`/`@user`/`@default`/`@unbound`, search by recorded keys (`Alt+K`), define (chords), remove, reset (`Alt+R`), conflicts marked, Restore Default Keybindings (→ `.bak`), Export Keybindings; edits keep comments (§5.4)
- [x] [M6] Live reload of config, keybindings, themes and `.vscode/settings.json` through the file watcher; content-hash check skips our own writes; a file that stops parsing keeps its last good values (§17.4, §17.5)
- [x] [M6] Settings commands: Open User Settings, Open Workspace Settings (workspace file or folder `.vscode/settings.json`), Open Default Settings (read-only), Open Keyboard Shortcuts (editor + TOML file), Create Theme from Current, Edit User Theme, Inspect Editor Tokens (§17.6)
- [x] [M6] Layout persistence (sidebar, panel size/maximised, group split ratios — per workspace) and "Reset Layout"; side bar left/right (`workbench.sideBarLocation`, toggle command) (§3.2, §17.3)
- [x] [M6] Responsive layout: side bar overlays below 50 columns, panel only when focused/maximised below 12 rows, tabs and status bar hidden below 3 rows; property test renders every size 1×1…50×15 in 11 UI states (§3.3)
- [x] [M6+] VS Code colour theme import, data only, no JS: installed extensions (VS Code, server, OSS, Cursor, Windsurf, bundled) or a `.json` file; `include` chains, TextMate rules resolved by selector specificity, semantic colours (§16.7)
- [~] [M6] Theme picker: show installed-but-not-imported VS Code themes inline — dropped (2026-11-09): colour themes will ship as plugins

---

## M7 — Syntax and development intelligence (§12, §13)

### Syntax (§12)
- [x] [M7] Language detection: `files.associations` (VS Code key, also imported from `.vscode/settings.json`) → file name (`Dockerfile.dev` too) → extension → `#!` line (incl. `/usr/bin/env`) → plain text (`yace-syntax/src/language.rs`)
- [x] [M7] Tree-sitter 0.27 integration (`crates/yace-syntax`); grammars compiled in but constructed, and their queries compiled, only when a file of that language opens (§12.2)
- [x] [M7] Incremental reparse fed from rope chunks (no copy). Each document keeps a running estimate of its reparse cost: predicted < 0.5 ms runs on the UI thread (1 ms cap), otherwise the syntax worker (lazy thread) parses and edits made meanwhile are replayed onto its tree; frames are drawn from the edited old tree, so input never waits (§12.1)
- [x] [M7] Edits reach syntax through a buffer-level edit log (`Document::track_edits`/`take_edits`, O(1) rope snapshots): typing, undo/redo, reload, save transforms, replace-all all included
- [x] [M7] Viewport-only highlighting (`QueryCursor` byte range), later-pattern-wins precedence (Neovim-style), spans cached per tree generation and viewport (§22.3, §22.4)
- [x] [M7] Languages with grammars: Rust, JS, JSX, TS, TSX, JSON, JSONC, YAML, TOML, HTML, CSS, SCSS, Markdown (+inline), SQL, Bash, Python, Go, Dockerfile, C, C++, Java, Ruby. Rules only (no grammar yet): Makefile, ignore files, `.env`/ini/properties/git config, Lua, plain text (§12.2)
- [ ] [M7] Grammar for git config / Makefile / Lua (write our own if no maintained crate fits)
- [x] [M7] Embedded languages (injections, depth ≤ 2, parsed lazily per visible region): Markdown code fences and inline, `<script>`/`<style>` in HTML, JS template/regex/JSDoc injections (§12.3)
- [x] [M7] Comment tokens, brackets, auto-close pairs and indent triggers from the language registry (replaces the M3 extension table); Toggle Block Comment (`Shift+Alt+A`); CSS/HTML `Ctrl+/` uses block comments; Enter between `<div></div>` splits into three lines; `:` indents in Python/YAML
- [x] [M7] Status bar language from detection
- [x] [M7] Syntax colours in the theme (`Theme::syntax`, YACE Dark ≈ VS Code Dark+), downgraded for 256/16 colours; large-file mode disables highlighting
- [x] [M7] Display map (`crates/yace/src/display.rs`): buffer lines → screen rows with folds and soft wrap, computed on demand for the viewport only; plain fast path when neither is active. Used by rendering, the terminal cursor, scroll-to-cursor, PgUp/PgDn, mouse wheel and hit-testing
- [x] [M7] Code folding: ranges from the syntax tree (outermost node starting on the line, or a block opened at its end; closing line stays visible), indentation otherwise (`yace-syntax/src/fold.rs`); Fold `Ctrl+Shift+[`, Unfold `Ctrl+Shift+]`, Toggle `Ctrl+K Ctrl+L`, Fold All `Ctrl+K Ctrl+0`, Unfold All `Ctrl+K Ctrl+J`; gutter `▾`/`▸` (click toggles; `editor.showFoldingControls`), `…` placeholder; Up/Down skip folds, Left/Right step over them; jumps into a fold (go to line, search) open it; edits inside a fold reveal it; folds are per view, mapped through edits and saved in the session
- [x] [M7] Word wrap: `editor.wordWrap` off/on/wordWrapColumn/bounded + `wordWrapColumn` (VS Code keys, imported), `Alt+Z` toggles for the window; word-boundary breaks, continuation rows keep the indentation, wide chars never straddle; Up/Down move by rows
- [ ] [M7] Word wrap: `Home`/`End` go to the wrapped row's start/end first (VS Code); currently line-based
- [ ] [M7] Fold commands: Fold Recursively, Fold Level N, Fold All Block Comments

### LSP (§13)
- [x] [M7] `yace-lsp` crate: JSON-RPC over stdio; per server a writer, a reader (decodes replies into typed `Reply`s off the UI thread) and a stderr ring (500 lines); own lenient protocol types (D24); positions in UTF-8/16/32 (negotiated `positionEncodings`); `file://` URIs
- [x] [M7] Lazy start when the first file of a language opens; built-in server table + `[[lsp.servers]]` overrides, `lsp.disabled`, `lsp.enabled`; `PATH` lookup; one install hint per language when none is found (§13.2)
- [x] [M7] Shutdown when idle (`lsp.idleShutdownSecs`, default 300; 0 = never); `shutdown`/`exit` then group kill by a reaper thread; all servers told to exit on quit
- [x] [M7] Multi-root: one instance per (server, root), `workspaceFolders` = that root (D25) (§13.3)
- [x] [M7] Never share server state across unrelated repositories
- [x] [M7] Document sync: `didOpen` (queued until initialized), incremental `didChange` from the shared edit log (full sync when that's all the server supports), `didSave` (with text if asked), `didClose`; edits are flushed before every request; answers for older versions dropped
- [x] [M7] Server→client requests: `workspace/configuration` (from `settings`), `workspace/workspaceFolders`, `workspace/applyEdit`, `window/workDoneProgress/create`, `client/(un)registerCapability`, `window/showMessageRequest`, `window/showDocument`; `$/progress`, `window/logMessage`, `window/showMessage`
- [x] [M7] Completion: trigger characters, quick suggestions after a typing pause (`editor.quickSuggestions[Delay]`, so legacy terminals get completion too), `Ctrl+Space` / Trigger Suggest; client-side fuzzy re-filter (narrowing, score-only ranking; 5,003 items: worst keystroke 3.1 ms release); snippets (placeholders, choices, variables, `$0`); `textEdit`/`insertReplace`; `additionalTextEdits` immediately or via `completionItem/resolve` (auto-imports); `command` for parameter hints / re-suggest
- [x] [M7] Hover (`Ctrl+K Ctrl+I`), parameter hints (`(`, `,`, `Ctrl+Shift+Space`) with the active parameter (string or offset labels)
- [x] [M7] Definition (`F12`, `Ctrl+Click`), declaration, type definition, implementation (`Ctrl+F12`), references (`Shift+F12`) — single result jumps, several open a picker with line previews; jumps go into other roots and are recorded in navigation history
- [x] [M7] Rename (`F2`): prompt seeded with the word; open documents get one undoable unsaved edit each, closed files are rewritten safely with conflict check; create/rename/delete file operations reported as skipped (no confirmation dialog, as VS Code)
- [x] [M7] Document symbols (`Ctrl+Shift+O` / Quick Open `@`, hierarchy indented, fetched once per version) and workspace symbols (`Ctrl+T` / `#`, every running server, re-queried as you type)
- [x] [M7] Code actions (`Ctrl+.`): diagnostics in context, preferred first, disabled shown with reason, `codeAction/resolve`, `workspace/executeCommand`
- [x] [M7] Formatting (`Ctrl+Shift+I`, `Ctrl+K Ctrl+F`) and `editor.formatOnSave` for Ctrl+S (waits ≤ 2 s, then saves anyway)
- [x] [M7] UI reflects the server's capabilities: every feature explains "not supported by X" / "no server for Y" / "starting" (§13.1)
- [x] [M7] Crash isolation: exit during startup → one message with stderr + install hint, no restart loop; crash after startup → restart (≤ 3 in 5 min), then stopped; "Restart / Stop Language Server" commands; requests time out (`lsp.requestTimeoutMs`) (§13.5)
- [x] [M7] Node-based servers treated as external tools; documented; "Developer: Show Running Language Servers" lists pid and RSS of each server's process group separately from YACE (§13.4) — measured: typescript-language-server + 2 tsserver + typings installer = 538 MiB on a 2-file project, YACE 32 MiB
- [x] [M7] Diagnostics: underline in the severity colour, line number coloured, status bar counts (⊗/⚠) and the message under the cursor, Problems picker (`Ctrl+Shift+M`), `F8`/`Shift+F8` across files; positions mapped through edits until republished
- [x] [M7] `Ctrl+Space`, `F12`, `F2`, `Ctrl+Shift+O`, `Ctrl+T` wired (+ legacy fallbacks `Alt+O`, `Alt+M`, `Alt+I`)
- [x] [M7] Format on save (setting)
- [x] [M7] Notification history picker (`Ctrl+K Ctrl+N`, Enter copies) and "Open Notifications in Editor"; one-time hints stay until Esc
- [x] [M7] "Show Language Server Output" (stderr + log in an untitled editor); status bar shows the active server and its progress
- [x] [M7] Tests: fake JSON-RPC server (`tests/fixtures/fake_lsp.py`) covering sync, diagnostics, completion, hover, signature help, cross-root definition, references, rename, symbols, code actions, format on save, crash/restart, startup failure, missing-server hint, stale answers, 5k-item lists; real-server smoke test with typescript-language-server (`YACE_TEST_TSLS`)
- [x] [M7] TypeScript 7: built-in `typescript` server (`tsc --lsp --stdio`) resolved from the project's TypeScript ≥ 7 (native binary started directly) or a global one; typescript-language-server only for TypeScript ≤ 6 (`registry::resolve`, `DefKind`) — v0.4.1
- [x] [M7] Pull diagnostics (`textDocument/diagnostic`, coalesced per file, inter-file re-pull, `workspace/diagnostic/refresh`) — v0.4.1
- [x] [M7] Replies decoded from raw JSON (`RawValue` envelope); completion items keep compact JSON text for resolve: 13,425 items 103 → 20.7 MiB; trim after big lists close — v0.4.2
- [x] [M7] Missing-server hint per folder and per missing server (mixed TypeScript 7/5 workspaces) — v0.4.2
- [ ] [M7] First keystroke over a 13k-item completion list (~8 ms est.): score in parallel chunks or cap the scanned set
- [~] [M7] Semantic tokens — deferred: tree-sitter covers colouring
- [ ] [M7] Several language servers per document (core mechanism: merged diagnostics, code actions, formatting); tool-specific servers such as ESLint or Biome ship as plugins, not built in
- [ ] [Plugins] External ESLint and Biome plugins (outside the core, optional) built on several servers per document; test both on a real repo with `biome.json` and one with an ESLint config; without the plugin nothing changes — later, after the core mechanism
- [ ] [Plugins] Colour themes as plugins (later; replaces showing VS Code themes in the picker)
- [ ] [M7] Mouse-hover popups; completion item documentation side panel
- [ ] [M7] Code lens, inlay hints, call hierarchy, document highlights (not in the M7 scope)

---

## M8 — Native plugin system (§18)

- [x] [M8] Decide in-process vs out-of-process → in-process C ABI (D28)
- [x] [M8] Asked the user about downloadable grammars → hybrid (D29)
- [x] [M8] `yace-plugin-api`: C ABI v1, versioned append-only tables with sizes, optional callbacks, status codes, buffer ownership, UTF-8 strings, borrowed lifetimes, threading rules, jobs with cancellation; C header kept in sync by a layout test (§18.8)
- [x] [M8] `yace-plugin-host`: discovery (worker thread), manifest validation before any code loads, OS/arch/YACE/ABI checks, SHA-256 approval record, `dlopen` (RTLD_NOW), table validation, timed calls, job pool (§18.5)
- [x] [M8] Manifest format (TOML, every §18.7 field; every problem reported) — `docs/plugins.md`
- [x] [M8] Lazy activation: command, language, view, workspace contents, event, Quick Open prefix, startup; contributions implicitly activate (§18.6)
- [x] [M8] Contributions: commands, keybindings, themes, status bar items, sidebar views (tree), Explorer decorations, Quick Open providers, settings, languages, grammars (§18.3)
- [~] [M8] Contributions: bottom-panel views, Git integrations (with M9), LSP servers from plugins (manifest key parsed, not wired yet)
- [x] [M8] Event API: workspace opened/closed, roots, documents opened/closed/saved/changed, active document, selection, theme, terminals, configuration; selection/document changes coalesced (50 ms) (§18.4)
- [x] [M8] Callbacks on the UI thread, every call timed, one-time warning above 100 ms, background jobs with cancellation (§18.10)
- [x] [M8] Controlled UI model: plugins provide data, the editor renders with the theme; control characters stripped (§18.12)
- [x] [M8] Plugin settings `[plugin."<id>"]` validated against the manifest, defaults, errors reported (§18.11)
- [x] [M8] Enable/disable globally (`plugins.disabled`) and per workspace (session state, workspace file untouched); `--safe-mode` / `--no-plugins` (§18.11)
- [x] [M8] Plugin manager: install from folder or `.tar.gz` (safe extraction), uninstall, enable, disable, approve, details, log, settings, reload; update = install a newer package (§18.13)
- [x] [M8] Commands: Extensions: Install from Package/Directory, Uninstall, Enable, Disable, Reload, List Installed, Show Performance, Show Plugin Views, and per plugin: Show Details (dependencies), Open Plugin Settings, Show Plugin Log (§18.14)
- [x] [M8] Trust UI: identity, publisher, source, version, capabilities, external programs, code hashes, "not sandboxed"; explicit confirmation; changed or hand-copied code not loaded until approved (§18.9, §27.2)
- [x] [M8] Docs say plainly that in-process plugins are not sandboxed (`docs/plugins.md`) (§33.11)
- [x] [M8] SDK: `yace-plugin` (Plugin trait, Host, `export_plugin!` catching panics, `MockHost`), examples `plugins/hello` and `plugins/tasks` (Task Runner), `scripts/build_plugin.sh`, `yace --package-plugin`, API reference, release checklist (§18.15, §18.16)
- [x] [M8] CLI: `--install-plugin [--yes]`, `--list-plugins`, `--package-plugin [-o]`, `--safe-mode`
- [x] [M8] Crash diagnosis: per-process marker of active plugins; an abnormal exit is reported at the next start
- [x] [M8] Hybrid grammars: `all-grammars` feature, runtime grammar/language registries in `yace-syntax`, `scripts/build_grammars.py` (package + licences), lazy load with hash check, re-highlighting of open files, one-time hint when missing; release workflow publishes the package
- [x] [M8] Tests: valid load, invalid manifest, unsupported ABI (manifest and init), init failure, short table, wrong ABI answer, changed code, hand-copied code, incompatible version, activation (command, workspace contents, event), deactivation (disable), registration, settings validation, slow and failing callbacks, safe mode, malformed ABI input (null/invalid UTF-8/oversized strings, null outputs, unknown handles, undeclared ids, missing capabilities, bad JSON), crash markers, data-only theme package, packaging, grammar package (load, tamper), C header layout, PTY install + run (§28.6)
- [ ] [M8] Load and hash-check grammar libraries on a worker (C++: 8.8 ms hash + query compile on first open)
- [ ] [M8] Status item click runs its command; plugin view context actions
- [~] [M8+] Install from Git or release artifact; registry (optional) (§18.13, §34.10)
- [~] [M8+] Out-of-process plugin host for stronger isolation (§18.9)

---

## M9 — Git and advanced workflow (§15)

- [x] [M9] `yace-git` crate: runs the user's `git` (timeouts, no prompt/pager/editor, `--no-optional-locks` reads, command log), porcelain v2 `-z` status parser, `imara-diff` line hunks, operations (D30) (§4.7)
- [x] [M9] Per-root repository detection; roots in one repository share it; non-repo roots and a missing git program handled (one hint)
- [x] [M9] Current branch (ahead/behind, dirty, detached, conflicts) of the active file's repository in the status bar, always visible
- [x] [M9] Working tree status (modified, added, deleted, renamed, untracked, ignored, conflicted), per repository, independently scheduled (one in flight, coalesced, 300 ms debounce), after the first frame (§15.3)
- [x] [M9] Explorer decorations scoped to each root's repository: colours, letters, folder propagation, ignored dimmed; map built off the UI thread (§8.5)
- [x] [M9] Source Control view (`Ctrl+Shift+G`): per repository conflicts / staged / changes / untracked with counts; keys and mouse (§15.1)
- [x] [M9] Staged and unstaged diffs as read-only changes documents, coloured by line; Enter jumps to the file line (§15.1)
- [x] [M9] Refresh triggers: `.git` watched non-recursively, file events and saves, focus regained, 10 s while the view is visible, after operations; git due times are event-loop deadlines
- [x] [M9] Read-only inspection automatic; mutations only from commands; opening a workspace runs only `rev-parse`/`status`/`show` (tested) (§15.4)
- [x] [M9] Never runs checkout / commit / reset on open (§15.4)
- [x] [M9] Gutter change bars vs the index (unsaved edits included, recomputed on a worker 150 ms after typing), Next/Previous Change (`Alt+F5`), Show Change Under Cursor
- [x] [M9] Stage / unstage file, group or repository; commit (message prompt, smart commit when nothing is staged), Commit All; switch branch (local or remote → tracking), create branch; failures show git's message (§15.2)
- [x] [M9] Commands ask which repository when ambiguous (never "the workspace")
- [x] [M9] `gitStatusChanged` plugin event (the Task Runner rescans on branch switches)
- [x] [M9] Test: branches in two roots stay independent; switching one leaves the other (§28.5)
- [~] [M9+] Stage/unstage hunks, discard changes, history, compare revisions, stash/apply, side-by-side diff, conflict navigation (§15.2)
- [ ] [M9] Status bar branch clickable (opens Checkout to…)

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
- [x] Documentation website (mdBook, site/; deploys with .github/workflows/pages.yml once the repo is public)
- [x] Website screenshots and recordings generated from the real release binary in a PTY (`site/capture/`: SVG stills, animated SVG recordings and asciicast v2 sources, in `site/src/media/`)

### Performance (§23)
- [ ] [M10] Benchmark suite: cold and warm start, workspace load, directory expansion, Quick Open, typing latency, scrolling, large file, multi-cursor, syntax, search, git, terminal throughput, plugin activation, memory (§23.7)
- [ ] [M10] Separate memory categories: core / child process / plugin / caches (§23.2)
- [ ] [M10] CI regression tracking where measurements are stable (§23.8)
- [ ] [M10] Minimal / reduced-feature mode (§23.9)
- [ ] [M10] Terminal compatibility matrix: GNOME Terminal, Kitty, Alacritty, WezTerm, Foot, Konsole, tmux (§22.5)

### Measured results vs §23.1 targets

Leftovers (v0.8.0, 2026-11-03, i5-1135G7, release). `cargo run --release --example bench_leftovers`: file-type icon lookup 45 ns (2.7 µs for a 60-row Explorer frame, no allocation); copy naming 16 µs; copying 10,000 files in 100 folders 0.36 s on a worker thread (the UI keeps running). Typing in a 1,700-line file with Nerd Font icons: input p95 0.71–0.84 ms (one outlier run 1.25 ms) vs 0.71–0.88 ms with icons off, i.e. within noise; idle CPU 0%. Binary 17.95 → 18.05 MB.

Git (M9, 2026-11-03, i5-1135G7, release). `cargo run --release --example bench_m9`: parsing 50,000 status records (2.5 MB) 6.1 ms and building their decoration map 7 ms — both on a git worker; gutter diff of a 10,000-line file with 20 hunks 1.4 ms (worker, after a 150 ms typing pause). The user's 9-folder workspace: first frame 8.21 ms with Git vs 8.23 ms without (30 runs each, interleaved); all 9 statuses arrive 0.2 s after start (`git status` per repo 2–57 ms); idle CPU 0% over 5 s; RSS 9.3 MiB (heap 1.28 MiB), two more threads. Typing in a 1,700-line file inside a repository: input p95 0.73–0.74 ms with Git, 0.64–0.77 ms without. Binary 17.63 → 17.95 MB.

Plugins (M8, 2026-11-03, i5-1135G7, release). `cargo run --release --example bench_m8`: load + init of a plugin 0.07 ms; editor → plugin call 76 ns; event delivery (JSON) 0.36 µs; a command making three host calls with JSON both ways 0.58 µs. First frame, 40 interleaved runs: v0.4.2 median 4.69 ms, M8 4.48 ms; with 20 installed plugins 4.77 ms vs 4.75 ms with none (manifests are read on a worker). Heap at idle identical to v0.4.2 (1,152 kB `RssAnon`); one more thread (plugin message forwarder). Binary 25.35 MB (v0.4.2) → 17.63 MB with hybrid grammars; the grammar package is 9.6 MB of libraries, 1.0 MB compressed. C++ grammar on first `.cpp` open: SHA-256 check 8.8 ms, `dlopen` 0.07 ms, query compilation 29.5 ms (the built-in grammar paid the same compilation).

Themes and customisation (M6, 2026-11-03, i5-1135G7). `cargo run --release --example bench_m6`: loading a built-in theme (compiled) 0.8–0.9 µs (parsed from TOML it was 38 µs for a root theme, 88–91 µs for one on top of a root; up to 0.3 ms cold at startup); keybinding-editor filtering over the whole default keymap (222 bindings) median 44 µs, worst 0.11 ms per keystroke. First frame (`--startup-time`, 3×30 runs, same machine, back to back): v0.4.2 median 5.03–5.44 ms, M6 5.43–5.64 ms (within run-to-run noise; min 3.19–3.40 vs 3.27–3.33). Idle RSS: heap (`RssAnon`) identical at 1,152 kB; +0.35 MiB of clean, shared binary pages (`RssFile`). Binary 25.35 → 25.92 MB (`toml_edit`, generated themes).

Syntax (M7, 2026-11-03, i5-1135G7). `cargo bench -p yace-syntax` on a 10k-line TypeScript file: full parse 80 ms (worker), incremental reparse after one keystroke 0.92 ms, highlight a 60-line viewport 0.58 ms (cached until the tree changes). `scripts/bench_interactive.py 300 big.ts` (typing at the top of the 10k-line file, the worst case for incremental parsing): input p50 0.074 ms / p95 0.55–1.0 ms (v0.3.1 without highlighting: 0.055 / 0.4–1.1 ms), render p95 0.13 ms. First frame (`--startup-time`, 15 runs): folder 4.8 ms (v0.3.1 4.5), small `.rs` 6.2 ms (5.7), 10k-line `.ts` 6.6 ms (5.1); RSS +0.4–0.6 MiB. Binary 7.2 MB → 24.4 MB: grammar tables are read-only data paged in only when used (largest: C++ 3.3 MB, TypeScript+TSX 2.7, SQL 2.3, Ruby 2.0, Bash 1.3, Rust 1.1). Revisit at M8 (downloadable grammars).

Terminal (`scripts/bench_terminal.py`, 2026-11-03): editor RSS 6.4 MiB idle, 7.9 MiB with 1 terminal, 26.7 MiB with a full 10k-line scrollback during `seq 1 2000000`, 30.9 MiB with 4 terminals (shells are separate processes).

Memory after Quick Open + a workspace search on a real 9-folder workspace (`scripts/bench_memory.py`, 2026-11-03): v0.2.1 defaults 20.0 MiB (peak 21.5 MiB) with search-as-you-type; with `.claude/worktrees` included 23.9 MiB; without heap trimming (`YACE_MALLOC_TRIM=0`, ≈ v0.2.0) 33.2 MiB. A 108 MiB report from an interactive v0.2.0 session was not reproduced.

Search benchmarks (`cargo bench -p yace-search`, 2026-11-03, i5-1135G7): Quick Open query over 200k paths 9–19 ms (parallel, top-200); walk of this repo 1.5 ms; content search of this repo 3.5 ms. After M4, startup 5.4 ms median and 6.4 MiB at first frame (indexing is lazy).

Leftovers batch 2, search (v0.9.0, 2026-11-09, i5-1135G7, on `~/Dev/node`: 44,100 files, 3,300–4,000 folders). `cargo run --release --example bench_batch2 -- ~/Dev/node`:
- **Narrowing a search** ("process.nextTic" → "process.nextTick"): a whole walk 183–259 ms → only the 368 files that matched 25–28 ms. Those files are 22 MB, one of them 10.5 MB.
- **Live update** of one changed file: 2.7–3.3 ms on the updater thread.
- **Find after a keystroke** (100k lines, 10k matches): copy and search the whole file 1.7–2.2 ms → edited lines only 0.23–0.31 ms. A whole-file search without the copy (64 KiB pieces) costs 2.5 ms, against 2.2 ms with the copy.
- **Index walk** with folders: 41–78 ms.
- **Watching 4,001 folders:** 41–72 ms on the watch thread. The kernel count matches; a created file's event arrives in < 2 ms.

Real binary against v0.8.2 on node (`scripts/bench.sh` and a PTY idle script):
- **First frame:** median 3.90–4.06 ms, against 3.93–4.00 ms.
- **Idle RSS without Quick Open:** 8.4 MiB, against 8.3.
- **Idle RSS after the first Quick Open:** 15.2–15.6 MiB, against 14.2–14.4. It now watches 3,328 folders; most of the +1 MiB is `notify`'s path tables.
  - An eager index at startup (tried first) cost 17 MiB at idle, so it stays lazy.
  - Keeping one YACE copy of the folder list instead of three saved about 0.8 MiB.
- **Idle CPU:** 0.
- **Typing** (`bench_interactive.py`, 300 keys): input p95 0.32 ms against 0.24–0.30; render p95 0.39–0.46 ms against 0.43–0.44. Within run-to-run noise.
- **Binary:** 18.05 → 18.30 MB.

Leftovers batch 2, terminal (v0.10.0, 2026-11-09, i5-1135G7):
- **Terminal find** (`cargo test --release -p yace-terminal -- --ignored --nocapture search_cost`): a whole-scrollback search takes 14–21 ms over 10k lines and 140–160 ms over 100k. It runs on the worker in 1,000-line chunks, about 1.5 ms of lock each, so the shell's output keeps flowing. Highlighting the visible matches costs 0.11–0.14 ms per frame, only while the bar is open.
- **Flood** (`scripts/bench_terminal.py`, `YACE_BENCH_SPLIT=1` for two panes): input p95 0.56–0.64 ms with one pane, 0.30–0.71 ms with two. Render p95 0.16–0.17 ms with one pane, 0.09–0.18 ms with two.
- **RSS against v0.8.2:** 1 terminal 10.6–10.9 MiB against 9.8–10.2; flood 29.5–29.8 against 28.7–29.0; 4 terminals 34.2–34.4 against 33.1–33.3. A split costs nothing measurable.
  - The +0.5–1 MiB is clean binary pages (`RssFile` 8.4–8.6 against 7.9–8.3 MiB at idle). The heap (`RssAnon`) is identical at 768 kB, plus one thread (the watcher).

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
