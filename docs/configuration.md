# Configuration

## Files and locations (XDG)

| What | Path | Override |
|---|---|---|
| User settings | `~/.config/yace/config.toml` | `YACE_CONFIG_DIR` |
| User keybindings | `~/.config/yace/keybindings.toml` | `YACE_CONFIG_DIR` |
| Session state, recent list, untitled workspaces, log | `~/.local/state/yace/` | `YACE_STATE_DIR` |
| Cache | `~/.cache/yace/` | `YACE_CACHE_DIR` |

"Preferences: Open User Settings (TOML)" (`Ctrl+,`) creates the file from a
commented template if it doesn't exist. Saving it inside YACE reloads the
settings immediately.

## Precedence

1. Built-in defaults
2. User settings (`config.toml`)
3. Workspace settings: the `settings` object of the `.code-workspace` file
4. Repository settings: `<root>/.vscode/settings.json`, for files inside
   that root

Steps 3 and 4 use **VS Code's setting names**. YACE maps the keys it
understands, listed below, and ignores the rest without rewriting them. To
set YACE-only options in a workspace file, put them in a `"yace"` object
that mirrors the TOML layout, for example
`"yace": { "files": { "largeFileThresholdMb": 100 } }`.

Each value is validated on its own. An invalid value is reported (file +
key) and the next lower layer's value is used. The full list of problems is
in "Developer: Show Configuration Errors".

**Scopes.** Some settings only make sense in certain places:
- *application* (user only): `keyboard.*`, `workbench.restoreWorkspace`,
  `workbench.colorDepth`, `workbench.glyphs`
- *window* (user or workspace): `files.autoSave`, `files.autoSaveDelay`,
  `files.largeFileThresholdMb`, `explorer.*`, the other `workbench.*` keys
- *resource* (also per repository): every `editor.*` key, `files.exclude`,
  `files.eol`, `files.insertFinalNewline`, `files.trimTrailingWhitespace`,
  `search.exclude`, `search.useIgnoreFiles`

## Settings

### `[editor]`

| Key | Default | Values | VS Code key |
|---|---|---|---|
| `tabSize` | `4` | integer ≥ 1 | `editor.tabSize` |
| `insertSpaces` | `true` | bool | `editor.insertSpaces` |
| `autoClosingBrackets` | `true` | bool | `editor.autoClosingBrackets` (`"never"` → false) |
| `lineNumbers` | `"on"` | `"on"`, `"off"`, `"relative"` | `editor.lineNumbers` |
| `renderLineHighlight` | `true` | bool | `editor.renderLineHighlight` (`"none"` → false) |
| `cursorSurroundingLines` | `3` | integer | `editor.cursorSurroundingLines` |
| `scrollBeyondLastLine` | `true` | bool | `editor.scrollBeyondLastLine` |
| `matchBrackets` | `true` | bool | `editor.matchBrackets` (`"never"` → false) |

### `[files]`

| Key | Default | Values | VS Code key |
|---|---|---|---|
| `exclude` | `.git`, `.svn`, `.hg`, `CVS`, `.DS_Store`, `Thumbs.db` (as `**/…`) | list of globs. **Additive**: each layer adds; `"!glob"` removes one | `files.exclude` (`{glob: true/false}`) |
| `autoSave` | `"off"` | `"off"`, `"afterDelay"`, `"onFocusChange"` | `files.autoSave` (`onWindowChange` → `onFocusChange`) |
| `autoSaveDelay` | `1000` | milliseconds | `files.autoSaveDelay` |
| `insertFinalNewline` | `false` | bool | `files.insertFinalNewline` |
| `trimTrailingWhitespace` | `false` | bool | `files.trimTrailingWhitespace` |
| `eol` | `"auto"` | `"auto"`, `"lf"`, `"crlf"` (for new files; existing files keep theirs) | `files.eol` |
| `largeFileThresholdMb` | `50` | MiB; larger files open in large-file mode after confirmation | — |

### `[explorer]`

| Key | Default | Meaning |
|---|---|---|
| `showHidden` | `true` | show dotfiles (excluded globs stay hidden) |
| `confirmDelete` | `true` | ask before moving to Trash |

### `[workbench]`

| Key | Default | Meaning | VS Code key |
|---|---|---|---|
| `enablePreview` | `true` | single-click / Space opens a reusable preview tab | `workbench.editor.enablePreview` |
| `restoreWorkspace` | `true` | `yace` with no arguments reopens the most recent workspace | — |
| `glyphs` | `"unicode"` | `"unicode"`, `"nerdFont"`, `"ascii"`: icon and border characters | — |
| `colorDepth` | `"auto"` | `"auto"`, `"truecolor"`, `"256"`, `"16"`, `"mono"` (`NO_COLOR` forces mono) | — |
| `sidebarWidth` | `32` | initial side bar width in cells | — |

### `[search]`

| Key | Default | Meaning | VS Code key |
|---|---|---|---|
| `exclude` | `**/node_modules`, `**/bower_components`, `**/*.code-search`, `**/.claude/worktrees` | globs left out of Quick Open and search, on top of `files.exclude`. **Additive**; `"!glob"` removes one (e.g. `"!**/.claude/worktrees"` to search Claude Code worktrees) | `search.exclude` |
| `useIgnoreFiles` | `true` | honour `.gitignore`, `.ignore` and global git excludes | `search.useIgnoreFiles` |
| `quickOpenIncludeIgnored` | `false` | Quick Open also lists ignored files (toggle per session with `Alt+I`) | — |
| `cacheIndex` | `true` | cache file **names** in `~/.cache/yace/index/` so Quick Open is instant on the next launch; always refreshed in the background, never trusted blindly | — |
| `maxResults` | `20000` | stop a workspace search after this many matches | `search.maxResults` |
| `maxIndexedFiles` | `500000` | per-folder cap for the Quick Open index | — |

### `[terminal]`

User settings only: a workspace file can never choose which program runs
(spec §4.12).

| Key | Default | Meaning |
|---|---|---|
| `shell` | `""` | shell executable; empty = `$SHELL` (else `/bin/sh`) |
| `args` | `[]` | arguments for the shell |
| `env` | `{}` | extra environment variables, e.g. `[terminal.env]` `EDITOR = "yace"` |
| `scrollback` | `10000` | lines kept per terminal (memory is bounded by this; a full 10,000-line buffer at 100 columns is about 20 MiB) |
| `panelHeight` | `12` | bottom panel height in rows (drag its top border to resize) |
| `cwd` | `"activeRoot"` | where new terminals start: `"activeRoot"` (workspace folder of the active file), `"activeFileDir"`, `"firstRoot"` |
| `commandsToSkipShell` | `[]` | extra command ids that keep working while a terminal has focus |
| `confirmKill` | `"whenRunning"` | ask before killing a terminal: `"whenRunning"`, `"always"`, `"never"` |
| `defaultProfile` | `""` | profile used by "Create New Terminal" |
| `profiles` | `[]` | `[[terminal.profiles]]` tables: `name`, `shell`, `args`, `env`, `startupCommand` |

Example: a profile that starts Claude Code in the active folder.

```toml
[[terminal.profiles]]
name = "claude"
startupCommand = "claude"
```

Terminal tabs (name, folder, profile) are restored with the workspace.
Restored tabs start a **new** shell when shown, marked "(new session)":
processes never survive quitting YACE.

### `[keyboard]`

| Key | Default | Meaning |
|---|---|---|
| `chordTimeoutMs` | `3000` | how long `Ctrl+K` waits for the second key |
| `enhancedProtocol` | `true` | use the Kitty keyboard protocol when the terminal supports it |

## Fonts

YACE runs inside your terminal, so the terminal emulator controls the font
family, size and rendering. YACE controls colours, text styles and which
glyphs it uses (`workbench.glyphs`).

## Example

```toml
[editor]
tabSize = 2
lineNumbers = "relative"

[files]
autoSave = "afterDelay"
trimTrailingWhitespace = true
exclude = ["**/node_modules", "**/dist", "**/.next", "**/coverage"]

[workbench]
glyphs = "nerdFont"
```
