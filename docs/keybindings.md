# Keybindings

YACE's defaults follow **VS Code on Linux/Windows**. Every binding maps to
a command id, so you can find and run any of them from the Command Palette
(`Ctrl+Shift+P` or `F1`), and override or remove them in
`~/.config/yace/keybindings.toml`.

## What your terminal can send

Terminals don't all report the same key combinations. Some need the
**enhanced keyboard protocol** (Kitty protocol, `CSI u`). YACE turns it on
automatically when the terminal supports it. Run "Developer: Show
Performance Metrics" to see which mode is active.

| Combination | Legacy terminal (GNOME Terminal, Konsole, tmux) | Enhanced terminal (Kitty, WezTerm, foot, Alacritty ≥ 0.13, Ghostty) |
|---|---|---|
| `Ctrl+<letter>`, `Alt+<letter>`, `F1`–`F12`, arrows with modifiers, `Ctrl+PgUp/PgDn` | ✅ | ✅ |
| `Ctrl+Shift+<letter>` (e.g. `Ctrl+Shift+P`) | ❌ arrives as `Ctrl+<letter>` | ✅ |
| `Ctrl+Tab`, `Ctrl+Enter`, `Ctrl+Shift+Enter` | ❌ | ✅ |
| `` Ctrl+` `` | arrives as `Ctrl+Space`; YACE treats that as `` Ctrl+` `` | ✅ |
| `Ctrl+\` | arrives as `Ctrl+4` | ✅ |
| `Ctrl+/` | arrives as `Ctrl+7` | ✅ |
| `Ctrl+Backspace` | arrives as `Ctrl+H` | ✅ |

Your terminal emulator may also capture some shortcuts before YACE sees them,
for example `Ctrl+Shift+T` (new tab) or `Ctrl+Shift+C/V` (copy/paste).
Rebind those in the terminal, or use the fallbacks below.

### Fallbacks (active only without the enhanced protocol)

| Fallback | Command | Replaces |
|---|---|---|
| `F1` | Show All Commands | `Ctrl+Shift+P` |
| `Ctrl+4` | Split Editor | `Ctrl+\` |
| `Ctrl+7` | Toggle Line Comment | `Ctrl+/` |
| `Ctrl+H` | Delete Word Left | `Ctrl+Backspace` |
| `Alt+E` | Focus Explorer | `Ctrl+Shift+E` |
| `Alt+S` | Save As | `Ctrl+Shift+S` |
| `Alt+K` | Delete Line | `Ctrl+Shift+K` |
| `Alt+L` | Select All Occurrences | `Ctrl+Shift+L` |
| `Alt+T` | Reopen Closed Editor | `Ctrl+Shift+T` |
| `Alt+Shift+F` | Search across the workspace | `Ctrl+Shift+F` |
| `Alt+Shift+H` | Replace across the workspace | `Ctrl+Shift+H` |
| `Ctrl+Space` (what `` Ctrl+` `` arrives as), `Ctrl+J`, `` Alt+` `` | Toggle terminal / panel | `` Ctrl+` `` — just press `` Ctrl+` ``; `` Alt+` `` is taken by GNOME's window switcher on Ubuntu |
| `Ctrl+PgDn` / `Ctrl+PgUp` | Next / previous editor | `Ctrl+Tab` |

## Defaults

### Workbench

| Keys | Command | Id |
|---|---|---|
| `Ctrl+Shift+P`, `F1` | Show All Commands | `workbench.action.showCommands` |
| `Ctrl+P`, `Ctrl+E` | Quick Open (fuzzy, all roots; `>` commands, `:line`, `file:line:col`, `?` help) | `workbench.action.quickOpen` |
| `Ctrl+Shift+F` | Search across the workspace (sidebar Search view) | `workbench.action.findInFiles` |
| `Ctrl+Shift+H` | Replace across the workspace | `workbench.action.replaceInFiles` |
| `Alt+Left` / `Alt+Right`, `Ctrl+Alt+-` / `Ctrl+Shift+-` | Go Back / Go Forward | `workbench.action.navigateBack` … |
| `Ctrl+Shift+E` | Focus Explorer | `workbench.view.explorer` |
| `Ctrl+B` | Toggle Side Bar | `workbench.action.toggleSidebarVisibility` |
| `Ctrl+K Ctrl+R` | Reveal Active File in Explorer | `workbench.files.action.showActiveFileInExplorer` |
| `F6` / `Shift+F6` | Focus Next / Previous Part (Explorer ↔ editor) | `workbench.action.focusNextPart` … |
| `Ctrl+N` | New Untitled File | `workbench.action.files.newUntitledFile` |
| `Ctrl+S` / `Ctrl+Shift+S` / `Ctrl+K S` | Save / Save As / Save All | `workbench.action.files.save` … |
| `Ctrl+W`, `Ctrl+F4` | Close Editor | `workbench.action.closeActiveEditor` |
| `Ctrl+K Ctrl+W` / `Ctrl+K U` / `Ctrl+K W` | Close All / Saved / Group's Editors | |
| `Ctrl+Shift+T` | Reopen Closed Editor | `workbench.action.reopenClosedEditor` |
| `Ctrl+PgDn` / `Ctrl+PgUp` | Next / Previous Editor | `workbench.action.nextEditor` … |
| `Ctrl+Tab` / `Ctrl+Shift+Tab` | Recently Used Editor | `workbench.action.openNextRecentlyUsedEditorInGroup` … |
| `Ctrl+Shift+PgUp/PgDn` | Move Editor Left / Right | `workbench.action.moveEditorLeftInGroup` … |
| `Alt+1`…`Alt+8`, `Alt+9` | Open Editor N / Last | `workbench.action.openEditorAtIndexN` |
| `Ctrl+K Enter` | Keep (pin) Preview Editor | `workbench.action.keepEditor` |
| `Ctrl+\` | Split Editor Right | `workbench.action.splitEditor` |
| `Ctrl+K Ctrl+\` | Split Editor Down | `workbench.action.splitEditorOrthogonal` |
| `Ctrl+1` / `Ctrl+2` / `Ctrl+3` | Focus Editor Group N (`Ctrl+2` splits if needed) | `workbench.action.focusFirstEditorGroup` … |
| `Ctrl+K Ctrl+Left/Right/Up/Down` | Focus Neighbouring Group | `workbench.action.focusLeftGroup` … |
| `Ctrl+Alt+Right` / `Ctrl+Alt+Left` | Move Editor to Next / Previous Group | `workbench.action.moveEditorToNextGroup` … |
| `Ctrl+K Ctrl+M` | Toggle Maximize Editor Group | `workbench.action.toggleMaximizeEditorGroup` |
| `Ctrl+K Ctrl+O` | Open Folder | `workbench.action.files.openFolder` |
| `Ctrl+R` | Open Recent | `workbench.action.openRecent` |
| `Ctrl+K F` | Close Workspace | `workbench.action.closeFolder` |
| `Ctrl+,` | Open User Settings | `workbench.action.openSettingsJson` |
| `Ctrl+K Ctrl+S` | Open Keyboard Shortcuts (editor) | `workbench.action.openGlobalKeybindings` |
| `Ctrl+K Ctrl+T` | Color Theme (picker with live preview) | `workbench.action.selectTheme` |
| `Ctrl+G` | Go to Line/Column | `workbench.action.gotoLine` |
| `Ctrl+Q` | Quit | `workbench.action.quit` |
| `Escape` | Dismiss Notification | `yace.dismissNotification` |


### Integrated terminal

| Keys | Action |
|---|---|
| `` Ctrl+` `` | show and focus the terminal; again to hide it (works in GNOME Terminal too) |
| `` Ctrl+Shift+` `` | new terminal in the active file's workspace folder |
| `Ctrl+J` | show / hide the bottom panel |
| `Ctrl+PgDn` / `Ctrl+PgUp` (terminal focused) | next / previous terminal |
| `Ctrl+C` | interrupt (or copy, when text is selected) |
| `Ctrl+Shift+C` / `Ctrl+Shift+V` | copy selection / paste |
| `Shift+PgUp` / `Shift+PgDn`, `Ctrl+Shift+Up` / `Ctrl+Shift+Down`, mouse wheel | scroll back |
| mouse drag (hold `Shift` inside mouse-aware apps) | select text |
| `F6` | switch focus between the terminal/explorer and the editor |

While a terminal has focus, **every key goes to the shell** except these
commands, which keep working: Quick Open (`Ctrl+P`), Command Palette
(`Ctrl+Shift+P`, `F1`), toggle terminal/panel, new terminal, next/previous
terminal, terminal copy/paste/scroll, maximize panel, Explorer
(`Ctrl+Shift+E`), Find in Files (`Ctrl+Shift+F`), focus editor group
(`Ctrl+1`–`3`) and `F6`. Add more with `terminal.commandsToSkipShell`.
`Ctrl+K`, `Ctrl+R`, `Ctrl+D`, `Ctrl+B` and so on reach the shell.

Other terminal commands are in the palette: "Open Terminal in Workspace
Folder…", "Open Terminal in Active File's Folder", "Create New Terminal
(With Profile)…", "Kill the Active Terminal Instance", "Rename…", "Clear",
"Toggle Maximized Panel". In the Explorer, "Open in Integrated Terminal"
starts one in the selected folder.

### Quick Open (`Ctrl+P`)

| Keys / prefix | Action |
|---|---|
| type | fuzzy file search across every workspace folder (e.g. `jobserv` → `jobs.service.ts`; a word can also match a folder's name: `api index`) |
| `name:42`, `name:42:8` | open at line / column |
| `>` | commands (this is also the Command Palette, `Ctrl+Shift+P`) |
| `:42` | go to line in the active editor |
| `?` | list the modes |
| `Alt+I` | include git-ignored files |
| `Up`/`Down`, `PgUp`/`PgDn`, `Enter`, `Escape` | navigate, open, close |
| `Ctrl+Enter` | open the file to the side |

### Find / replace in the editor (`Ctrl+F`, `Ctrl+H`)

| Keys | Action |
|---|---|
| `Enter` / `Shift+Enter`, `F3` / `Shift+F3` | next / previous match |
| `Alt+C` / `Alt+W` / `Alt+R` / `Alt+L` | match case / whole word / regex / in selection |
| `Tab` | switch between the find and replace fields |
| `Up` / `Down` (find field) | search history |
| `Ctrl+Shift+1` | replace current match |
| `Ctrl+Alt+Enter` | replace all (one undo step) |
| `Alt+Enter` | select all matches (multiple cursors) |
| `Escape` | close the find bar |

### Search view (`Ctrl+Shift+F`)

| Keys | Action |
|---|---|
| type | search (runs as you type); `Tab` / `Down` moves through query, replace, include, exclude, results |
| `Alt+C` / `Alt+W` / `Alt+R` | match case / whole word / regex |
| `./Root Name/…` in "files to include" | limit the search to one workspace folder (Explorer → `Shift+Alt+F` "Find in Folder…") |
| results: `Enter` / `Space` | open / preview the match |
| results: `Left` / `Right` | collapse / expand a file |
| results: `Delete` | exclude a match or file from replacement |
| `Ctrl+Alt+Enter` | replace all (with confirmation; open files are edited but not saved) |
| `Escape` | back to the query, then to the editor |

### Editor (`editorFocus`)

| Keys | Command |
|---|---|
| Arrows, `Home` (smart), `End`, `PgUp`, `PgDn` (+ `Shift` to select) | cursor movement |
| `Ctrl+Left/Right` (+ `Shift`) | word movement |
| `Ctrl+Home/End` (+ `Shift`) | document start / end |
| `Ctrl+Up/Down`, `Alt+PgUp/PgDn` | scroll |
| `Backspace`, `Delete`, `Ctrl+Backspace`, `Ctrl+Delete` | delete |
| `Enter` | newline with auto-indent |
| `Tab` / `Shift+Tab`, `Ctrl+]` / `Ctrl+[` | indent / outdent |
| `Ctrl+Enter` / `Ctrl+Shift+Enter` | insert line below / above |
| `Ctrl+A`, `Ctrl+L` | select all / select line |
| `Ctrl+Z`, `Ctrl+Y`, `Ctrl+Shift+Z` | undo / redo |
| `Ctrl+C`, `Ctrl+X`, `Ctrl+V` (+ `Ctrl+Insert`, `Shift+Delete`, `Shift+Insert`) | clipboard (whole line when nothing is selected) |
| `Ctrl+D` | add selection to next match |
| `Ctrl+Shift+L` | select all occurrences |
| `Ctrl+Shift+Up/Down` | add cursor above / below |
| `Alt+Click` | add cursor |
| `Escape` | remove secondary cursors / cancel selection |
| `Alt+Up/Down` | move line(s) |
| `Shift+Alt+Up/Down`, `Ctrl+Shift+Alt+Up/Down` | copy line(s) |
| `Ctrl+Shift+K` | delete line(s) |
| `Ctrl+/` | toggle line comment (block comment in CSS/HTML) |
| `Shift+Alt+A` | toggle block comment |
| `Ctrl+Shift+[` / `Ctrl+Shift+]` | fold / unfold (legacy terminals: `Ctrl+K Ctrl+L` toggles) |
| `Ctrl+K Ctrl+L` | toggle fold |
| `Ctrl+K Ctrl+0` / `Ctrl+K Ctrl+J` | fold all / unfold all |
| click `▾` / `▸` in the gutter | fold / unfold |
| `Alt+Z` | toggle word wrap |
| `Ctrl+Shift+\` | jump to matching bracket |
| `Ctrl+K Ctrl+X` | trim trailing whitespace |

### Git (see [git.md](git.md))

| Keys | Command | Id |
|---|---|---|
| `Ctrl+Shift+G` | Show Source Control | `workbench.view.scm` |
| `Alt+F5` / `Shift+Alt+F5` | Go to Next / Previous Change | `workbench.action.editor.nextChange` … |

In the Source Control view: `Enter` opens changes, `o` the file, `s`/`u`
stage/unstage, `c` commit, `b` switch branch, `r` refresh, arrows move and
collapse/expand.

### Notifications

| Keys | Command |
|---|---|
| `Esc` | dismiss the current notification |
| `Ctrl+K Ctrl+N` | notification history: searchable, `Enter` copies the message |
| — | "View: Open Notifications in Editor" (select and copy anything) |

### Language features (with a language server)

| Keys | Command |
|---|---|
| typing, `Ctrl+Space` (enhanced terminals) | completion; `Up`/`Down`, `Enter`/`Tab` accept, `Escape` closes |
| `Ctrl+Shift+Space` | parameter hints (also on `(` and `,`) |
| `Ctrl+K Ctrl+I` | hover |
| `F12`, `Ctrl+Click` | go to definition |
| `Ctrl+F12` | go to implementations |
| `Shift+F12` | references |
| `F2` | rename symbol |
| `Ctrl+.` | quick fix / code actions |
| `Ctrl+Shift+I` (legacy: `Alt+I`), `Ctrl+K Ctrl+F` | format document / selection |
| `F8` / `Shift+F8` | next / previous problem |
| `Ctrl+Shift+M` (legacy: `Alt+M`) | Problems |
| `Ctrl+Shift+O` (legacy: `Alt+O`), `Ctrl+P` then `@` | go to symbol in editor |
| `Ctrl+T`, `Ctrl+P` then `#` | go to symbol in workspace |

In legacy terminals `Ctrl+Space` is what `` Ctrl+` `` sends, so it toggles
the terminal; completion still opens on its own while you type (or run
"Trigger Suggest" from the palette).

### Explorer (`explorerFocus`)

| Keys | Action |
|---|---|
| `Up` / `Down` / `Home` / `End` / `PgUp` / `PgDn` | move selection |
| `Right` / `Left` | expand (or step in) / collapse (or go to parent) |
| `Enter` | open file (pinned) / toggle folder |
| `Space` | preview file / toggle folder |
| typing | filter visible entries; `Backspace` edits, `Escape` clears |
| `F2` | rename (on a root: rename its display name) |
| `Delete` | move to Trash, after confirmation (on a root: remove it from the workspace) |
| `Ctrl+Alt+N` | new file |
| `Shift+Alt+C` / `Ctrl+Shift+Alt+C` | copy path / relative path |
| `Ctrl+C` / `Ctrl+X` / `Ctrl+V` | copy / cut / paste the file or folder (paste goes into the selected folder, or the selected file's folder; an existing name gets " copy", " copy 2"…; copying runs in the background) |
| `Ctrl+Enter` | open to the side (in the group to the right, splitting if needed) |
| `Shift+Alt+R` | reveal in the file manager (opens the containing folder; works from the editor too) |

## Customising

### Keyboard Shortcuts editor (`Ctrl+K Ctrl+S`)

Lists every command — bound or not — with its keys, when-clause and source
(Default, Plugin or User). Conflicting bindings (same keys and when-clause,
different commands; only the last one is reachable) are marked with ⚠.

| Key | Action |
|---|---|
| typing | filter by command title, id, keys or when-clause; `@conflicts`, `@user`, `@default`, `@unbound` narrow the list |
| `Alt+K` | search by keys: press a key or chord to list what it's bound to (`Alt+K`/`Esc` stops) |
| `Enter` | change the selected binding: press the new key or chord (two presses), then `Enter`; `Esc` cancels. The editor shows what else uses those keys |
| `Delete` | remove the selected binding (at the end of the search text) |
| `Alt+R` | reset the command to its default keys (removes your entries for it) |
| `Esc` | close |

Changes are written to `keybindings.toml` (your comments and order are
kept): replacing a default adds a `-command` entry for the old keys plus the
new binding. "Preferences: Restore Default Keybindings" moves your file to
`keybindings.toml.bak`; "Preferences: Export Keybindings" opens the whole
effective keymap as TOML. Keys your terminal can't send (see above) can be
bound but only work in terminals that send them.

### `keybindings.toml`

`~/.config/yace/keybindings.toml`:

```toml
# Later entries win. The key syntax is VS Code's.
[[bindings]]
key = "ctrl+alt+s"
command = "workbench.action.files.saveAll"

[[bindings]]
key = "ctrl+e"
command = "workbench.action.quickOpen"
when = "editorFocus"

# Remove a default (optionally only for one key or context):
[[bindings]]
command = "-workbench.action.quickOpen"
key = "ctrl+p"

# Commands that take arguments:
[[bindings]]
key = "ctrl+alt+l"
command = "workbench.action.terminal.sendSequence"
args = { text = "npm test\n" }
```

Commands that take `args`:

| Command | `args` |
|---|---|
| `type` | `{ text = "…" }`: type into the editor |
| `editor.action.insertSnippet` | `{ snippet = "console.log($1)$0" }`: VS Code snippet syntax |
| `workbench.action.terminal.sendSequence` | `{ text = "…" }`: written to the active terminal as is (`\u001b` escapes work) |
| `workbench.action.quickOpen` | `">"`, `"@"`, `":"`…: open Quick Open with this prefix |
| `workbench.action.gotoLine` | `{ line = 42, column = 8 }`, `42` or `"42:8"` |
| `workbench.action.files.openFile`, `vscode.open` | `"path"` or `{ path = "…", line = 1, column = 1 }` (relative to the first folder) |
| `git.checkout` | `{ branch = "main" }` (the active file's repository) |

A binding whose `args` don't fit says so when you press it, naming this
file. Plugins pass arguments the same way (`execute_command`); a failure
is written to the plugin's log.

Problems in this file are reported with the entry number, and the valid
entries still apply. Saving the file — in YACE or any other editor —
reloads it; if it no longer parses, the previous bindings stay until it is
fixed. To see bindings
that shadow each other, run "Developer: Show Keybinding Conflicts".
