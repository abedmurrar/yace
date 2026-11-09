# Getting started

**Start YACE with a folder or a workspace file, then use `Ctrl+P` to open
files and `Ctrl+Shift+P` (or `F1`) for every other command.** This page
assumes `yace` is on your `PATH`; see [Installation](installation.md) to
build it.

## Open something

```sh
yace                          # reopen the most recent workspace
yace .                        # open the current folder
yace ~/work.code-workspace    # open a VS Code or Cursor multi-root workspace
yace api --add-root ../web    # combine unrelated folders into one workspace
yace --goto src/main.rs:42:8  # open a file at line 42, column 8
yace --no-restore .           # open without restoring the previous tabs and layout
```

Opening a file opens its folder too (the nearest folder with a `.git`
entry, or the file's own folder). With no arguments, YACE reopens the
workspace you used last; set `workbench.restoreWorkspace = false` to start
with an empty window instead.

Inside YACE, "Open Folder…" (`Ctrl+K Ctrl+O`), "Open Workspace from
File…" and "Open Recent…" (`Ctrl+R`) switch workspaces, and "Add Folder to
Workspace…" adds another folder to the current one. See
[Workspaces and sessions](features/workspaces.md).

## The keys that matter

| Keys | What it does |
|---|---|
| `Ctrl+P` | Quick Open: fuzzy file search across every workspace folder |
| `Ctrl+Shift+P`, `F1` | Command Palette: every command, with its shortcut |
| `` Ctrl+` `` | show or hide the integrated terminal |
| `Ctrl+Shift+F` / `Ctrl+Shift+H` | search / replace across the workspace |
| `Ctrl+F` / `Ctrl+H` | find / replace in the file (`Ctrl+F` in a terminal searches its output) |
| `Ctrl+Shift+E` | focus the Explorer; `Ctrl+B` hides or shows the side bar |
| `Ctrl+Shift+G` | Source Control |
| `Ctrl+S` / `Ctrl+W` | save / close the editor |
| `Ctrl+\` | split the editor |
| `Ctrl+D` | add the next match to the selection (multiple cursors) |
| `F12` / `F2` | go to definition / rename (with a language server) |
| `Ctrl+K Ctrl+T` | choose a colour theme |
| `Ctrl+,` | open your settings |
| `Ctrl+Q` | quit |

The [Keybindings](docs/keybindings.md) reference lists every default.

## If a shortcut doesn't work

Terminals don't all send the same keys. Many (GNOME Terminal, Konsole,
tmux) can't send `Ctrl+Shift+<letter>`, so `Ctrl+Shift+P` arrives as
`Ctrl+P`. Terminals with the enhanced keyboard protocol (Kitty, WezTerm,
foot, Alacritty 0.13 or later, Ghostty) send everything, and YACE turns the
protocol on by itself. In other terminals, YACE adds fallbacks:

| Instead of | Press |
|---|---|
| `Ctrl+Shift+P` | `F1` |
| `Ctrl+Shift+F` / `Ctrl+Shift+H` | `Alt+Shift+F` / `Alt+Shift+H` |
| `Ctrl+Shift+E` | `Alt+E` |
| `Ctrl+\` | `Ctrl+4` |
| `Ctrl+/` | `Ctrl+7` |

The full table is in [What your terminal can send](docs/keybindings.md#what-your-terminal-can-send).
When in doubt, open the Command Palette with `F1` and type what you want.

## What to read next

- [Features](features/index.md), with screenshots of each area.
- [Configuration](docs/configuration.md): every setting.
- [Using plugins](plugins/using.md).
