# Themes

YACE colours the whole interface and the code from one theme. Themes are
TOML files; six are built in and you can add your own, extend a built-in
theme, override single colours in your settings, or import a VS Code theme.

## Choosing a theme

`Ctrl+K Ctrl+T` (Preferences: Color Theme) opens the theme picker. Moving
through the list previews each theme immediately; `Enter` keeps it and
saves `workbench.colorTheme` in your user `config.toml` (comments and
layout are kept); `Esc` goes back to the previous theme. Or set it
directly:

```toml
[workbench]
colorTheme = "Dracula"
```

| Built-in theme | Id | Type | Palette from |
|---|---|---|---|
| YACE Dark (default) | `yace-dark` | dark | YACE; syntax colours follow VS Code Dark+ |
| YACE Light | `yace-light` | light | YACE; syntax colours follow VS Code Light+ |
| One Dark | `one-dark` | dark | Atom One Dark (MIT) |
| Dracula | `dracula` | dark | Dracula (MIT) |
| Catppuccin Mocha | `catppuccin-mocha` | dark | Catppuccin (MIT) |
| Tokyo Night | `tokyo-night` | dark | Tokyo Night by enkia (MIT) |

When YACE reads a workspace's VS Code settings, `workbench.colorTheme`
names map to the closest built-in theme ("Default Dark Modern" → YACE
Dark, "One Dark Pro" → One Dark, …).

### More themes

[YACE Themes](https://github.com/abedmurrar/yace-themes) is a plugin with
four more: **Retro Blue** (the blue-screen DOS IDEs), **Phosphor** (green
on black), **High Contrast** (every text colour at least 7:1, WCAG AAA)
and **Low Contrast** (soft and muted). It's data only, so no code runs:

```sh
yace --install-plugin io.github.abedmurrar.yace-themes
```

Or "Extensions: Browse Plugins…" inside YACE.

Any plugin can add themes with `[[themes]]` in its manifest
([plugins.md](plugins.md)).

## Overriding colours

Keep a theme but change a few colours in `config.toml` (VS Code's keys and
shape, so existing VS Code overrides can be copied over):

```toml
[workbench.colorCustomizations]
"statusBar.background" = "#3d59a1"
"editor.selectionBackground" = "#264f78"
# Only when Dracula is the active theme:
"[Dracula]" = { "tab.activeBackground" = "#44475a" }

[editor.tokenColorCustomizations]
comments = { foreground = "#7f848e", fontStyle = "italic" }
keywords = "#c678dd"
"function.method" = { fg = "#61afef", bold = true }
```

`tokenColorCustomizations` accepts VS Code's simple keys (`comments`,
`strings`, `keywords`, `numbers`, `types`, `functions`, `variables`),
any syntax scope below, `textMateRules` (mapped onto the scopes YACE
highlights) and `"[Theme Name]"` sections. Overrides apply in this order:
theme → global overrides → overrides for the active theme.

## Writing a theme

Put a file in `~/.config/yace/themes/`. "Preferences: Create Theme from
Current" writes a starting point and opens it; "Preferences: Edit User
Theme…" opens existing ones. Saving the file applies it immediately.

```toml
name = "My Night"          # shown in the picker
type = "dark"              # "dark" or "light"
extends = "tokyo-night"    # optional: a theme id or name

[colors]
"editor.background" = "#16161e"
"editor.selectionBackground" = "#7aa2f740"   # #rrggbbaa: blended over the background

[syntax]
comment = { fg = "#565f89", italic = true }
keyword = "#bb9af7"
"function.method" = { fg = "#7aa2f7", bold = true }

[semantic]                 # reserved for semantic highlighting
parameter = "#e0af68"
```

- **Inheritance.** Keys a theme doesn't set come from `extends`, or from
  YACE Dark / YACE Light (by `type`) when there is no `extends`. Chains
  of any length work; a cycle is an error. When a theme sets
  `editor.background` or `editor.foreground` but not the colours derived
  from them (active tab, terminal, popup text, …), those follow it.
- **Colours**: `#rgb`, `#rrggbb`, `#rrggbbaa`, ANSI names (`red`,
  `bright-blue`, …), `ansi:N` (0–255, the terminal's palette) or
  `default` (the terminal's own colour).
- **Syntax entries** are a colour or `{ fg, bold, italic, underline }`.
- **Errors.** A theme that can't be loaded (unreadable, a cycle, an unknown
  `extends`) is reported with the file and line, and the current theme
  stays. Unknown keys and bad values are reported as warnings; the rest of
  the theme still loads ("Developer: Show Configuration Errors").
- "Developer: Inspect Editor Tokens" shows the scope, colour and style of
  the text under the cursor.

### Colour keys

Keys are VS Code colour ids wherever VS Code has an equivalent. Some other
VS Code ids are also accepted as aliases (`quickInput.background`,
`panel.border`, `list.focusBackground`, …).

| Key | Colours |
|---|---|
| `editor.background` | Editor and main background |
| `editor.foreground` | Default text |
| `descriptionForeground` | Secondary text (details, hints, placeholders) |
| `sideBar.background` | Explorer and search sidebar |
| `sideBar.foreground` | Sidebar text |
| `sideBarTitle.foreground` | Sidebar section titles |
| `tab.activeBackground` | Active tab |
| `tab.activeForeground` | Active tab text |
| `tab.inactiveBackground` | Inactive tabs |
| `tab.inactiveForeground` | Inactive tab text |
| `editorGroupHeader.tabsBackground` | Tab bar |
| `editorGroup.border` | Borders between parts |
| `focusBorder` | Focused part border and titles |
| `editor.selectionBackground` | Selection |
| `editor.inactiveSelectionBackground` | Selection in an unfocused editor |
| `editor.lineHighlightBackground` | Current line |
| `editorLineNumber.foreground` | Line numbers |
| `editorLineNumber.activeForeground` | Current line number |
| `editorCursor.background` | Character under a drawn block cursor |
| `editorCursor.foreground` | Drawn cursor (secondary cursors) |
| `editorBracketMatch.background` | Matching bracket |
| `statusBar.background` | Status bar |
| `statusBar.foreground` | Status bar text |
| `statusBar.noFolderBackground` | Status bar with no folder open |
| `list.activeSelectionBackground` | Selected row in a focused list |
| `list.activeSelectionForeground` | Selected row text |
| `list.inactiveSelectionBackground` | Selected row in an unfocused list |
| `list.highlightForeground` | Matched characters in lists |
| `editorWidget.background` | Pickers, prompts and popups |
| `editorWidget.foreground` | Popup text |
| `editorWidget.border` | Popup border |
| `quickInputList.focusBackground` | Selected row in a picker |
| `input.background` | Input fields |
| `editorError.foreground` | Errors |
| `editorWarning.foreground` | Warnings |
| `editorInfo.foreground` | Information |
| `gitDecoration.modifiedResourceForeground` | Git: modified |
| `gitDecoration.addedResourceForeground` | Git: added |
| `gitDecoration.deletedResourceForeground` | Git: deleted |
| `yace.dirtyForeground` | Unsaved-changes marker |
| `yace.missingRootForeground` | Unavailable workspace folder |
| `scrollbarSlider.background` | Scrollbar thumb |
| `editorWhitespace.foreground` | Rendered whitespace |
| `editor.findMatchHighlightBackground` | Find matches |
| `editor.findMatchBackground` | Current find match |
| `editorSuggestWidget.highlightForeground` | Matched characters in completion and pickers |
| `terminal.background` | Integrated terminal |
| `terminal.foreground` | Terminal text |
| `terminal.ansiBlack` | Terminal ANSI 0 |
| `terminal.ansiRed` | Terminal ANSI 1 |
| `terminal.ansiGreen` | Terminal ANSI 2 |
| `terminal.ansiYellow` | Terminal ANSI 3 |
| `terminal.ansiBlue` | Terminal ANSI 4 |
| `terminal.ansiMagenta` | Terminal ANSI 5 |
| `terminal.ansiCyan` | Terminal ANSI 6 |
| `terminal.ansiWhite` | Terminal ANSI 7 |
| `terminal.ansiBrightBlack` | Terminal ANSI 8 |
| `terminal.ansiBrightRed` | Terminal ANSI 9 |
| `terminal.ansiBrightGreen` | Terminal ANSI 10 |
| `terminal.ansiBrightYellow` | Terminal ANSI 11 |
| `terminal.ansiBrightBlue` | Terminal ANSI 12 |
| `terminal.ansiBrightMagenta` | Terminal ANSI 13 |
| `terminal.ansiBrightCyan` | Terminal ANSI 14 |
| `terminal.ansiBrightWhite` | Terminal ANSI 15 |

### Syntax scopes

`none`, `comment`, `string`, `string.special`, `string.escape`, `character`, `number`, `boolean`, `constant`, `constant.builtin`, `keyword`, `keyword.control`, `operator`, `punctuation`, `punctuation.bracket`, `function`, `function.builtin`, `function.method`, `function.macro`, `type`, `type.builtin`, `constructor`, `variable`, `variable.builtin`, `variable.parameter`, `property`, `attribute`, `tag`, `namespace`, `label`, `markup.heading`, `markup.bold`, `markup.italic`, `markup.link`, `markup.raw`, `markup.list`, `embedded`

A key may also be any tree-sitter capture name; it applies to the scope it
maps to (`keyword.function` → `keyword`, `function.call` → `function`).

## Importing VS Code themes

"Preferences: Import VS Code Color Theme…" lists the themes of extensions
installed in VS Code (`~/.vscode/extensions`, also `~/.vscode-server`,
`~/.vscode-oss`, `~/.cursor`, `~/.windsurf` and the themes bundled with
VS Code). "…from File…" takes a theme `.json` path. YACE reads the theme's
colour data only — no extension code runs and no JavaScript is involved —
and writes `~/.config/yace/themes/<name>.toml`, which you can then edit:

- `colors` map through the table above (with aliases);
- `tokenColors` (TextMate rules) are resolved onto YACE's syntax scopes,
  most specific selector first, as VS Code does; `include`d base themes
  are followed;
- `semanticTokenColors` go to `[semantic]`;
- themes whose token colours live in a `.tmTheme` file keep the base
  theme's syntax colours (reported).

## Colour depth

YACE detects how many colours the terminal supports and converts every
theme colour: true colour, 256 colours, the terminal's 16 colours, or none
(`NO_COLOR` or `workbench.colorDepth = "mono"`). Without colours, state
is shown with text attributes so nothing disappears: the selection, the
cursor and the selected list row are reverse video, the current find match
is bold reverse video, other matches and problems are underlined, and the
active tab is bold.

## Fonts and glyphs

YACE runs inside a terminal, so **the terminal emulator controls the font**:
family, size, ligatures and rendering are set in your terminal's settings,
not in YACE. YACE controls colours, bold/italic/underline, and which
characters it uses for icons and borders (`workbench.glyphs`: `"unicode"`,
`"nerdFont"` for a Nerd Font, or `"ascii"`).
