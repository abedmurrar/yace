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
| `` Ctrl+` ``, `Ctrl+Tab`, `Ctrl+Enter`, `Ctrl+Shift+Enter` | ❌ | ✅ |
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
| `Alt+Z` | Redo | `Ctrl+Shift+Z` (`Ctrl+Y` also works) |
| `Ctrl+PgDn` / `Ctrl+PgUp` | Next / previous editor | `Ctrl+Tab` |

## Defaults

### Workbench

| Keys | Command | Id |
|---|---|---|
| `Ctrl+Shift+P`, `F1` | Show All Commands | `workbench.action.showCommands` |
| `Ctrl+P` | Open File (path prompt; fuzzy Quick Open in M4) | `workbench.action.quickOpen` |
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
| `Ctrl+K Ctrl+S` | Open Keyboard Shortcuts File | `workbench.action.openGlobalKeybindingsFile` |
| `Ctrl+G` | Go to Line/Column | `workbench.action.gotoLine` |
| `Ctrl+Q` | Quit | `workbench.action.quit` |
| `Escape` | Dismiss Notification | `yace.dismissNotification` |

These are reserved for later milestones and currently show a "not available
yet" notice: `Ctrl+J` (panel, M5), `` Ctrl+` `` (terminal, M5),
`Ctrl+Shift+F` (search, M4), `Ctrl+F` / `Ctrl+H` (find/replace, M4),
`Ctrl+K Ctrl+T` (themes, M6), and `Ctrl+Shift+O`, `Ctrl+T`, `Ctrl+Space`,
`F2`, `F12` (language features, M7).

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
| `Ctrl+/` | toggle line comment |
| `Ctrl+Shift+\` | jump to matching bracket |
| `Ctrl+K Ctrl+X` | trim trailing whitespace |

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

## Customising

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
```

Problems in this file are reported with the entry number, and the valid
entries still apply. Saving the file from inside YACE reloads it; so does
running "Preferences: Reload Settings and Keybindings". To see bindings
that shadow each other, run "Developer: Show Keybinding Conflicts".
