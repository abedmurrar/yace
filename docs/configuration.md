# Configuration

## Files and locations (XDG)

| What | Path | Override |
|---|---|---|
| User settings | `~/.config/yace/config.toml` | `YACE_CONFIG_DIR` |
| User keybindings | `~/.config/yace/keybindings.toml` | `YACE_CONFIG_DIR` |
| User themes | `~/.config/yace/themes/*.toml` ([themes.md](themes.md)) | `YACE_CONFIG_DIR` |
| Plugins and their approval record | `~/.config/yace/plugins/` ([plugins.md](plugins.md)) | `YACE_CONFIG_DIR` |
| Session state, recent list, untitled workspaces, log | `~/.local/state/yace/` | `YACE_STATE_DIR` |
| Cache | `~/.cache/yace/` | `YACE_CACHE_DIR` |

"Preferences: Open User Settings (TOML)" (`Ctrl+,`) creates the file from a
commented template if it doesn't exist. Other settings commands:
"Preferences: Open Workspace Settings" (the `.code-workspace` file, or the
folder's `.vscode/settings.json`), "Preferences: Open Default Settings
(Read-only)" (every key with its default), "Preferences: Open Keyboard
Shortcuts" (`Ctrl+K Ctrl+S`, see [keybindings.md](keybindings.md)) and
"Preferences: Color Theme" (`Ctrl+K Ctrl+T`).

**Live reload.** Changes to `config.toml`, `keybindings.toml`, theme files
and `.vscode/settings.json` apply as soon as the file is saved, in YACE or
in any other program. If a file no longer parses, YACE reports the file and
line and **keeps the last values that worked** until it is fixed.
`workbench.colorDepth` and `workbench.glyphs` changes apply on reload too.

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
  `files.largeFileThresholdMb`, `explorer.*`, the other `workbench.*` keys,
  `editor.tokenColorCustomizations`
- *resource* (also per repository): the other `editor.*` keys, `files.exclude`,
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
| `wordWrap` | `"off"` | `"off"`, `"on"` (viewport width), `"wordWrapColumn"`, `"bounded"` (smaller of the two). `Alt+Z` toggles it for the window | `editor.wordWrap` |
| `wordWrapColumn` | `80` | integer | `editor.wordWrapColumn` |
| `folding` | `true` | bool | `editor.folding` |
| `formatOnSave` | `false` | bool; formats with the language server before Ctrl+S saves (waits at most 2 s) | `editor.formatOnSave` |
| `quickSuggestions` | `true` | bool; ask for completions while typing an identifier, not only on `.` etc. | `editor.quickSuggestions` |
| `quickSuggestionsDelay` | `50` | ms of typing pause before asking | `editor.quickSuggestionsDelay` |
| `suggestOnTriggerCharacters` | `true` | bool | `editor.suggestOnTriggerCharacters` |
| `showFoldingControls` | `"mouseover"` | `"always"`, `"mouseover"` (folded lines always; foldable ones while the mouse is over the gutter), `"never"` | `editor.showFoldingControls` |
| `tokenColorCustomizations` | `{}` | syntax colour overrides: `comments`, `strings`, `keywords`, `numbers`, `types`, `functions`, `variables`, any scope name, `textMateRules`, and `"[Theme Name]"` sections ([themes.md](themes.md)) | `editor.tokenColorCustomizations` |

### `[files]`

| Key | Default | Values | VS Code key |
|---|---|---|---|
| `exclude` | `.git`, `.svn`, `.hg`, `CVS`, `.DS_Store`, `Thumbs.db` (as `**/…`) | list of globs. **Additive**: each layer adds; `"!glob"` removes one | `files.exclude` (`{glob: true/false}`) |
| `autoSave` | `"off"` | `"off"`, `"afterDelay"`, `"onFocusChange"` | `files.autoSave` (`onWindowChange` → `onFocusChange`) |
| `autoSaveDelay` | `1000` | milliseconds | `files.autoSaveDelay` |
| `insertFinalNewline` | `false` | bool | `files.insertFinalNewline` |
| `trimTrailingWhitespace` | `false` | bool | `files.trimTrailingWhitespace` |
| `eol` | `"auto"` | `"auto"`, `"lf"`, `"crlf"` (for new files; existing files keep theirs) | `files.eol` |
| `largeFileThresholdMb` | `50` | MiB; larger files open in large-file mode after confirmation (no highlighting, wrap or folding) | — |
| `associations` | `{}` | table of glob (or file name) → language id, e.g. `"*.mdc" = "markdown"`. Ids: `rust`, `typescript`, `typescriptreact`, `javascript`, `javascriptreact`, `json`, `jsonc`, `yaml`, `toml`, `html`, `css`, `scss`, `markdown`, `python`, `go`, `shellscript`, `sql`, `dockerfile`, `c`, `cpp`, `java`, `ruby`, `makefile`, `ignore`, `properties`, `lua`, `plaintext` | `files.associations` |

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
| `colorTheme` | `"YACE Dark"` | theme name or file id ([themes.md](themes.md)); VS Code theme names map to the closest built-in | `workbench.colorTheme` |
| `colorCustomizations` | `{}` | UI colour overrides by VS Code colour id, with `"[Theme Name]"` sections | `workbench.colorCustomizations` |
| `sideBarLocation` | `"left"` | `"left"` or `"right"`; "View: Primary Side Bar Position: Left/Right" toggles it | `workbench.sideBar.location` |

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

### `[lsp]` (user settings only)

A workspace can never choose which programs run, so language servers are
configured in your user `config.toml` only.

| Key | Default | Values |
|---|---|---|
| `enabled` | `true` | bool |
| `disabled` | `[]` | server names never started, e.g. `["pyright"]` |
| `idleShutdownSecs` | `300` | stop a server this long after its last file closed (`0` = never) |
| `requestTimeoutMs` | `10000` | drop requests without an answer after this long |
| `servers` | `[]` | extra servers, or overrides of built-in ones with the same `name` (below) |

```toml
[[lsp.servers]]
name = "typescript-language-server"     # replaces the built-in entry
command = "typescript-language-server"  # on PATH, or an absolute path
args = ["--stdio"]
languages = ["typescript", "typescriptreact", "javascript", "javascriptreact"]
env = { NODE_OPTIONS = "--max-old-space-size=4096" }
initializationOptions = { preferences = { importModuleSpecifierPreference = "relative" } }
settings = { typescript = { format = { semicolons = "remove" } } }   # answers workspace/configuration
```

Built-in servers (used when found on `PATH`; YACE never installs them):

| Server | Languages | Install |
|---|---|---|
| `rust-analyzer` | Rust | `rustup component add rust-analyzer` |
| `typescript` — TypeScript 7's own server (`tsc --lsp --stdio`, native binary) | TS, TSX, JS, JSX | used automatically when the project's `node_modules/typescript` is ≥ 7 (or, without one, a global TypeScript ≥ 7): `npm install -g typescript` |
| `typescript-language-server` (Node.js) | TS, TSX, JS, JSX (projects on TypeScript ≤ 6) | `npm install -g typescript@5 typescript-language-server` |
| `pyright` (Node.js), else `pylsp` | Python | `npm install -g pyright` / `pip install python-lsp-server` |
| `gopls` | Go | `go install golang.org/x/tools/gopls@latest` |
| `clangd` | C, C++ | your package manager |
| `bash-language-server` (Node.js) | Shell | `npm install -g bash-language-server` |
| `yaml-language-server` (Node.js) | YAML | `npm install -g yaml-language-server` |
| `vscode-json/css/html-language-server` (Node.js) | JSON/JSONC, CSS/SCSS, HTML | `npm install -g vscode-langservers-extracted` |
| `taplo` | TOML | `cargo install taplo-cli --locked` |
| `marksman` | Markdown | see github.com/artempyanykh/marksman |
| `docker-langserver` (Node.js) | Dockerfile | `npm install -g dockerfile-language-server-nodejs` |

Node.js servers are external tools like any other: their memory is not
YACE's and is shown separately by "Developer: Show Running Language
Servers". Restart, stop and output commands are under "Developer:".

### `[git]`

| Key | Default | Meaning | VS Code key |
|---|---|---|---|
| `enabled` | `true` | Git status, decorations, gutter and Source Control ([git.md](git.md)) | `git.enabled` |
| `path` | `"git"` | the git program (user settings only) | — |
| `decorationsEnabled` | `true` | colour and mark changed files in the Explorer | `git.decorations.enabled` |
| `autoRefresh` | `true` | refresh on file and repository changes, focus, and while Source Control is visible | `git.autorefresh` |
| `diffDecorations` | `"all"` | `"all"`/`"gutter"`: change bars in the gutter; `"none"`: off | `scm.diffDecorations` |

### `[plugins]` (user settings only) and `[plugin."<id>"]`

| Key | Default | Meaning |
|---|---|---|
| `plugins.disabled` | `[]` | plugin ids that are not loaded ("Extensions: Disable" edits it) |
| `plugin."<id>".<key>` | the plugin's default | a plugin's own settings, as declared in its manifest and validated against it |

Which native code runs is decided in user settings only; a workspace
cannot enable a plugin. Enabling or disabling a plugin for one workspace is
kept in that workspace's session state ([plugins.md](plugins.md)).

```toml
[plugins]
disabled = ["dev.example.noisy"]

[plugin."dev.yace.hello"]
greeting = "Howdy"
```

### `[keyboard]`

| Key | Default | Meaning |
|---|---|---|
| `chordTimeoutMs` | `3000` | how long `Ctrl+K` waits for the second key |
| `enhancedProtocol` | `true` | use the Kitty keyboard protocol when the terminal supports it |

## Layout

The side bar width, panel height and maximised state, and editor group
sizes are saved per workspace. "View: Reset Layout (Sidebar, Panel, Group
Sizes)" restores the defaults; "View: Reset Editor Group Sizes" only evens
out the groups. On small terminals YACE adapts instead of failing: below 50
columns the side bar covers the editor only while it has focus; below 12
rows of editor space the panel shows only while focused or maximised; below
3 rows the tab bar and status bar are hidden.

## Fonts

YACE runs inside your terminal, so the terminal emulator controls the font
family, size and rendering. YACE controls colours, text styles and which
glyphs it uses (`workbench.glyphs`). See [themes.md](themes.md#fonts-and-glyphs).

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
colorTheme = "Tokyo Night"
colorCustomizations = { "statusBar.background" = "#3d59a1" }
```
