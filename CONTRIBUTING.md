# Contributing to YACE

Thanks for helping. YACE aims to be a fast, native, terminal-first editor
with first-class multi-root workspaces. A few rules keep it that way.

## Ground rules

- **No JavaScript runtimes.** Electron, Node.js, V8, QuickJS and anything
  similar are out, in the editor and in its plugins.
- **Performance is a requirement.**
  - No unbounded queues, caches or background work.
  - Nothing heavy at startup.
  - Back performance claims with numbers from `scripts/bench.sh`,
    `scripts/bench_interactive.py` or `cargo bench`.
- **Multi-root is not optional.** Every feature has to respect workspace
  root boundaries.
- **Never lose user data.** No silent overwrites. Unsaved changes are never
  discarded without a prompt. Unknown fields in user files are never
  rewritten.
- **Never fail silently.** Report errors to the user.

## Workflow

1. Look at [`TODO.md`](TODO.md). It's the master checklist for the whole
   specification. Pick an item, or add one.
2. Develop:
   ```sh
   cargo test --workspace
   cargo clippy --workspace --all-targets -- -D warnings
   cargo fmt --all
   ```
3. In the same change:
   - tick the item in `TODO.md`, or mark it `[~]` with the reason
   - add a line under `## [Unreleased]` in [`CHANGELOG.md`](CHANGELOG.md)
     if users will notice the change
   - update `docs/` when you add keys, settings or commands
4. Use [Conventional Commits](https://www.conventionalcommits.org) for
   commit messages (see [`docs/releasing.md`](docs/releasing.md)).

## Tests

- Library crates (`yace-buffer`, `yace-keymap`, `yace-config`,
  `yace-workspace`) carry unit tests next to the code.
- App behaviour is tested headlessly in `crates/yace/tests/` with the
  harness in `tests/common`. It drives `App` with synthetic keys and mouse
  events and renders to ratatui's `TestBackend`.
- Multi-root tests must use folders that don't share a parent.
  `common::multi_root()` sets these up.
- `tests/pty_smoke.rs` runs the real binary in a pseudo-terminal.

## Dependencies and credit

Adding a dependency? Prefer permissive licences (MIT, Apache-2.0, BSD,
Unlicense, Zlib, CC0). Regenerate `THIRD_PARTY_LICENSES.md` with
`scripts/third_party.py`; `scripts/release.sh` does this too. Add
significant ones to `CREDITS.md`.

## Licence

YACE is dual-licensed under [MIT](LICENSE-MIT) or
[Apache-2.0](LICENSE-APACHE), at your option. Unless you explicitly say
otherwise, any contribution you intentionally submit for inclusion in YACE
is dual-licensed the same way, with no additional terms or conditions.
