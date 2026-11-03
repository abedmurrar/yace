# YACE architecture

## Crates

| Crate | Responsibility | Depends on |
|---|---|---|
| `yace-buffer` | Rope documents (Ropey), multi-range selections, change sets (apply / invert / map), undo history, VS Code-style movement and edit operations, Unicode helpers, UTF-8 loading and atomic saving | — |
| `yace-keymap` | Key and chord parsing (`ctrl+k ctrl+t`), `when` clauses, layered keymaps (default < plugin < user, `-command` removal, conflict detection), chord resolver, VS Code default table | — |
| `yace-config` | Settings schema, per-key validated layering, scopes (application / window / resource), VS Code settings mapping, JSONC parsing, XDG paths, atomic writes | `yace-keymap` |
| `yace-search` | Fuzzy scorer, parallel ignore-aware walker, Quick Open file index (top-K, warm cache), ripgrep-based content search and safe replace, in-document find | `yace-buffer`, `yace-config` |
| `yace-workspace` | Multi-root model (roots, status, duplicates, nesting, path → root), `.code-workspace` parsing and non-destructive CST editing, session state, recent list | `yace-config` |
| `yace-terminal` | Shell sessions on PTYs (`alacritty_terminal`), key/mouse/paste encoding, screen snapshots, job detection | — |
| `yace-syntax` | Language registry and detection, tree-sitter grammars (compiled in, constructed lazily), incremental parsing from rope chunks with a time budget, viewport-only highlighting with injections, folding ranges | — |
| `yace-lsp` | Language server client: JSON-RPC framing, one process per server with writer/reader/stderr threads, typed decoding of responses on the reader thread, positions in UTF-8/16/32, `file://` URIs, server registry and `PATH` lookup | — |
| `yace` | The application: state machine (`App`), command registry, Explorer, editor groups and layout, overlays, rendering, event loop, terminal lifecycle, CLI | all of the above |

The library crates have no terminal dependencies, so every editing,
keybinding, configuration and workspace behaviour is unit-tested in
isolation.

## Event loop

```
 input thread ──┐
 fs workers ────┼──► crossbeam channel ──► App::handle(event) ──► Effects ──► runtime
 notify watcher ┤        (blocking recv;                │                (read dir, watch,
 signal thread ─┘        timeout only for a              └──► needs_render ──► ui::draw (ratatui diff)
                         pending deadline)
```

- **Zero idle CPU.** The loop blocks on `recv()`. It uses `recv_timeout`
  only while a deadline is pending: an unfinished chord, the fs-event
  coalescing window, an auto-save delay or a notification expiry.
- **Render only when something changed.** Ratatui's buffer diff sends only
  the cells that changed.
- **Bursts cost one frame.** After an event, everything already queued is
  drained before drawing, with input first.
- **Bounded background work.** A fixed pool of 2–4 threads lists
  directories through a bounded queue, and duplicate requests are dropped.
  Watches are non-recursive and cover only expanded directories and the
  parents of open files. Events are coalesced over ~100 ms.
- **No Tokio.** Threads plus one channel. M7 kept it that way: each
  language server gets three small blocking threads (writer, reader,
  stderr); the reader decodes JSON into typed replies, so the UI thread
  never parses a large response.

### Workers (M4)

| Worker | Owns | Fed by | Emits |
|---|---|---|---|
| index actor (`workers::spawn_index_actor`) | the `FileIndex`; spawns walker threads | `Effect::Index(Sync/Query/FsChange)` | `IndexProgress` (throttled), `QuickOpenResults` |
| search runner | one cancel flag per search | `Effect::Search` / `SearchCancel` | `SearchFile` per file, `SearchDone` |
| find worker | — | `Effect::Find` (rope snapshot, O(1) clone) | `FindDone` |
| file loader | — | `Effect::LoadFile` (files > 1 MiB) | `FileLoaded` |
| syntax worker (started on first use) | — | `Effect::Parse` (rope snapshot + old tree); jobs for the same document collapse to the newest | `Parsed { cost }` |
| language servers (`yace-lsp`) | per server: writer, reader (decodes into `Reply`), stderr ring | `Client::request` / `notify` | `AppEvent::Lsp` (`Response`, `Diagnostics`, `Progress`, server requests, `Exited`) |

#### Syntax and language servers (M7)

- Every `Document` can keep an **edit log** of `(rope before, ChangeSet)`
  pairs (rope clones share nodes, so this is O(1)). `App::sync_documents`
  drains it once per event and hands the same edits to the syntax tree
  (tree-sitter `InputEdit`s) and to the language server (incremental
  `didChange`). Typing, undo, reload, save transforms and replace-all are
  all covered without special cases.
- **Reparse placement is predicted**, not tried: each document keeps a
  running estimate of what an incremental reparse costs. Under 0.5 ms it
  runs inline (1 ms cap); otherwise it goes to the syntax worker, and the
  frame is drawn from the *edited* old tree. Edits made while a job runs are
  replayed onto its result.
- **Highlighting is viewport-only**: the query runs on the visible byte
  range of each contiguous run of lines (folds skip hidden text), and spans
  are cached until the tree changes.
- **The display map** (`display.rs`) maps buffer lines to screen rows
  (folds + soft wrap) on demand for the rows asked for; rendering, cursor
  placement, scrolling, vertical movement and mouse hit-testing share it.
- **Language servers** start when the first file of their language opens,
  one per (server, workspace root). Requests carry the document version;
  answers for an older version are dropped. Pending edits are flushed
  before every request, so the server always answers about what is on
  screen. A server that exits during startup is reported once with its
  stderr and an install hint; a crash after startup restarts it (at most 3
  times in 5 minutes).

| terminal sessions (`yace-terminal`) | one `alacritty_terminal` event-loop thread per shell: PTY I/O + VT parsing into a bounded grid | `Session::write` / `resize` / `kill` | `Notice::Output` (coalesced: one pending notice per session until the UI draws it), `Title`, `Exit`, `Bell` |

Every request carries a sequence number, and the app ignores results whose
number is out of date. The index actor coalesces bursts of queries and
only answers the latest one. Indexing is **lazy**: it starts on the first
Quick Open, never at startup. Roots are re-walked when Quick Open opens and
their index is more than 30 s old. In between, filesystem events patch the
index. Cached name lists (`search.cacheIndex`) make the first Quick Open
instant and are always refreshed in the background.

`IndexEngine` contains all the index logic and is used both by the actor
and, synchronously, by the test harness.

## `App` is terminal-agnostic

`App` consumes `AppEvent`s (terminal events, directory results, filesystem
changes, termination) and emits `Effect`s (read a directory,
watch/unwatch, raw bytes for OSC 52). Integration tests drive it with
synthetic events, run effects synchronously and render to ratatui's
`TestBackend` (`crates/yace/tests/common`). A separate PTY test runs the
real binary.

## Commands and keys

Every action is a command with a stable id, using VS Code ids where an
equivalent exists. This covers key presses, palette entries, mouse handlers
and tests. Keys go through:

```
overlay (if open) ─► resolver.feed(key, context) ─► Command | Pending chord | Invalid chord | Unbound
                                                               Unbound + printable ─► type into editor / filter explorer
```

The context is computed per key and includes: focus (`editorFocus`,
`explorerFocus`), whether the editor has a selection or multiple
selections, whether the explorer filter is active, `keyboardEnhanced` and
`notificationVisible`.

## Themes and configuration (M6)

- `theme/` holds the `Theme` (semantic colour tokens + syntax styles) and
  its loader. Theme files are TOML whose UI keys are VS Code colour ids;
  one table (`COLOR_KEYS`) maps ids to fields and drives parsing,
  overrides, VS Code import and the docs (a test keeps `docs/themes.md` in
  sync). Inheritance (`extends`) is resolved by building the parent first.
- The six built-in themes are **compiled**: `builtin_gen.rs` is generated
  from `crates/yace/themes/*.toml` by a test that fails when it is stale,
  so startup never parses a theme (0.8 µs instead of 40–90 µs warm, up to
  0.3 ms cold). User themes are parsed when used.
- The theme is rebuilt only when what it depends on changes (name,
  overrides, colour depth, or a theme file on disk). Spans store syntax
  scopes, not colours, so switching themes re-colours the next frame with
  no re-highlighting.
- State styles (`selection()`, `find_match()`, `list_selected()`, `tab()`,
  `status()`, `cursor()`) switch to text attributes in monochrome, so no
  state depends on colour alone.
- **Live reload** (`app/live_reload.rs`): the config folder, the themes
  folder and each root's `.vscode` folder are watched by the same watcher
  as the Explorer. A change is applied only if the file's content hash
  differs from what was last applied (so YACE's own writes don't reload
  twice). A file that fails to parse keeps its last good layer.
- **Writing settings** (`yace_config::edit_setting` / `edit_toml`) uses
  `toml_edit`, so comments, order and formatting survive; used by the theme
  picker, the side bar position toggle and the keybinding editor.
- **Keybinding editor** (`keybindings_editor.rs`): a terminal-agnostic
  model (rows, filter, record/define modes) that emits `KbEdit`s; the app
  turns them into `keybindings.toml` edits (`-command` entries to replace
  defaults) and reloads the keymap.

## Documents, views, groups

- `DocumentStore` owns documents, keyed by canonical path, so one file is
  one document.
- A `View` has its own selection and scroll position. Each edit returns the
  applied `ChangeSet`, and the other views of that document map their
  selections through it, which keeps split views in sync.
- `Layout` is a split tree of `Group`s (each has tabs, an active index, MRU
  order and a preview tab). Group rectangles and split handles come from
  one function that both drawing and mouse hit-testing use.

## Safety properties

- Every save goes through `yace_buffer::io::write_safely`: temp file in
  the target's real directory, fsync, rename, then fsync the directory.
  Permissions are copied. Hard-linked files are written in place.
- Each save compares a disk snapshot (mtime, size, hash) first. If the file
  changed on disk, the user must choose to overwrite.
- Unsaved changes are never discarded without a prompt. Closing tabs,
  groups or workspaces and quitting all run the same close flow
  (`actions::CloseFlow`).
- State files (sessions, recent list, workspace files) are written
  atomically. A corrupt one is reported and ignored.
- The terminal is restored by a Drop guard, a panic hook and the
  SIGTERM/SIGHUP handler.
