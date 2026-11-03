# YACE architecture

## Crates

| Crate | Responsibility | Depends on |
|---|---|---|
| `yace-buffer` | Rope documents (Ropey), multi-range selections, change sets (apply / invert / map), undo history, VS Code-style movement and edit operations, Unicode helpers, UTF-8 loading and atomic saving | — |
| `yace-keymap` | Key and chord parsing (`ctrl+k ctrl+t`), `when` clauses, layered keymaps (default < plugin < user, `-command` removal, conflict detection), chord resolver, VS Code default table | — |
| `yace-config` | Settings schema, per-key validated layering, scopes (application / window / resource), VS Code settings mapping, JSONC parsing, XDG paths, atomic writes | `yace-keymap` |
| `yace-search` | Fuzzy scorer, parallel ignore-aware walker, Quick Open file index (top-K, warm cache), ripgrep-based content search and safe replace, in-document find | `yace-buffer`, `yace-config` |
| `yace-workspace` | Multi-root model (roots, status, duplicates, nesting, path → root), `.code-workspace` parsing and non-destructive CST editing, session state, recent list | `yace-config` |
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
- **No Tokio.** Threads plus one channel are enough for M1–M3. The choice
  will be revisited with benchmarks when language servers arrive (M7).

### Workers (M4)

| Worker | Owns | Fed by | Emits |
|---|---|---|---|
| index actor (`workers::spawn_index_actor`) | the `FileIndex`; spawns walker threads | `Effect::Index(Sync/Query/FsChange)` | `IndexProgress` (throttled), `QuickOpenResults` |
| search runner | one cancel flag per search | `Effect::Search` / `SearchCancel` | `SearchFile` per file, `SearchDone` |
| find worker | — | `Effect::Find` (rope snapshot, O(1) clone) | `FindDone` |
| file loader | — | `Effect::LoadFile` (files > 1 MiB) | `FileLoaded` |

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
