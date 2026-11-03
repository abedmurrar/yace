# Versioning and releases

## Versioning

YACE follows [Semantic Versioning](https://semver.org). Every crate in the
workspace shares one version, set in `[workspace.package]` in the root
`Cargo.toml`.

**Before 1.0** (`0.MINOR.PATCH`):
- `MINOR` goes up for new features and for anything that breaks users:
  the CLI, config or keybinding file formats, command ids, session-state
  compatibility.
- `PATCH` goes up for fixes and internal changes only.

**From 1.0 on**, the usual SemVer rules apply. These count as public API:
- CLI flags
- command ids
- `config.toml` and `keybindings.toml` keys
- the `.code-workspace` handling
- the plugin ABI (once it exists)

The **plugin ABI** gets its own version number once M8 lands (spec
§25.10). Plugins check that version, not the editor's.

Milestones map onto minor versions: 0.1.0 = M1–M3. M4 and later each bump
`MINOR` when they ship.

### Plugin ABI version

The native plugin ABI (`crates/yace-plugin-api`, `ABI_VERSION`) is
versioned **separately** from YACE (spec §25.10):

- Appending a function to `YaceHost` or a callback to `YacePlugin`, adding
  an event, capability, manifest key or status code: compatible, same ABI.
- Changing or removing anything in those tables, or the meaning of a field:
  a new ABI major version. YACE refuses plugins built for an ABI it doesn't
  support, with a message naming both versions.
- Record ABI changes in the changelog under **Plugin ABI**, and keep
  `include/yace_plugin.h` in sync (the layout test fails otherwise).

A release also publishes the grammar package
(`yace-grammars-<version>-<arch>.tar.gz`, built by
`scripts/build_grammars.py`).

## Changelog

`CHANGELOG.md` uses the [Keep a Changelog](https://keepachangelog.com)
format.
- Every user-visible change adds a line under `## [Unreleased]` in the same
  commit, under Added, Changed, Deprecated, Removed, Fixed, Security or
  Performance.
- Performance claims include numbers and how they were measured (spec
  §33.9).
- At release time, `scripts/release.sh` turns `Unreleased` into the version
  heading.

## Commit messages

[Conventional Commits](https://www.conventionalcommits.org):
`feat(explorer): …`, `fix(buffer): …`, `perf: …`, `docs: …`, `test: …`,
`chore(release): v0.2.0`. Breaking changes use `!` (`feat(cli)!: …`) and a
`BREAKING CHANGE:` footer.

## Tags

- Releases are **annotated** tags named `vMAJOR.MINOR.PATCH`, created on
  `main`. The tag message is that version's changelog section.
- Pre-releases use `v0.2.0-beta.1` or `v0.2.0-rc.1`. They are published as
  GitHub pre-releases and never promoted automatically (spec §25.9).
- Tags are never moved or deleted after they're pushed. A broken release is
  fixed by a new patch release.

## Cutting a release

```sh
scripts/release.sh 0.2.0        # or 0.2.0-rc.1
git push origin main --follow-tags
```

`scripts/release.sh`:
1. refuses to run with a dirty tree, off `main`, with an existing tag, or
   with an empty `Unreleased` section
2. runs `cargo fmt --check`, `cargo clippy -D warnings` and
   `cargo test --workspace`
3. sets the workspace version and refreshes `Cargo.lock`
4. moves the `Unreleased` notes under `## [X.Y.Z] - DATE` and updates the
   comparison links
5. commits `chore(release): vX.Y.Z` and creates the annotated tag

Pushing the tag runs `.github/workflows/release.yml`:
1. checks that the tag matches the `Cargo.toml` version
2. runs the full checks
3. builds release binaries for `x86_64` and `aarch64` Linux
4. packages each as a `.tar.gz` with the licences, README, CHANGELOG and
   docs
5. writes SHA-256 checksums
6. publishes a GitHub Release whose notes are the changelog section

Debian packages, the signed APT repository and the Snap are added to this
pipeline in M10 (spec §25.11).

## Before the first public push

- [ ] Create the GitHub repository and replace `OWNER` in the
  `CHANGELOG.md` links.
- [ ] Set `repository` in `Cargo.toml`.
- [ ] Enable private vulnerability reporting (see `SECURITY.md`).
- [ ] Protect `main`: require the CI workflow to pass, disallow force-pushes.
- [ ] Push `main` and the existing tags: `git push -u origin main --follow-tags`.
