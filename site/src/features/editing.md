# Editing

**Editing works like VS Code: no modes, the same shortcuts, multiple
cursors, and saves that never corrupt a file.** Prefer Vim's modes? See
[Vim keys](#vim-keys).

![Multiple cursors: Ctrl+D adds the next match, then typing changes them all](../media/multi-cursor.anim.svg)

*Recording: `Ctrl+D` four times, then typing a new name.
[asciicast](../media/multi-cursor.cast)*

## Cursors and selections

- Multiple cursors: `Ctrl+D` adds the next match, `Ctrl+Shift+L` selects
  every occurrence, `Ctrl+Shift+Up` / `Ctrl+Shift+Down` add a cursor above
  or below, and `Alt+Click` adds one anywhere.
- Copy, cut and paste work with every cursor, and copy the whole line when
  nothing is selected. YACE uses the system clipboard, OSC 52 or an
  internal one, whichever is available.
- Movement is Unicode-correct: graphemes, wide characters and emoji count
  as one character. `Home` is smart (first non-blank character first).

## Changing text

- Word-wise undo and redo (`Ctrl+Z`, `Ctrl+Y`).
- Auto-indent, auto-closing brackets and quotes, and matching-bracket
  highlight.
- Move, copy and delete lines (`Alt+Up/Down`, `Shift+Alt+Up/Down`,
  `Ctrl+Shift+K`), toggle comments (`Ctrl+/`, `Shift+Alt+A`).
- Snippets and typed text can be bound to keys (see
  [commands that take arguments](../docs/keybindings.md#keybindingstoml)).

## Find and replace in a file

`Ctrl+F` opens the find bar and `Ctrl+H` adds the replace field. Match
case, whole word, regular expressions and "in selection" are toggles
(`Alt+C`, `Alt+W`, `Alt+R`, `Alt+L`). `Alt+Enter` turns every match into a
cursor; replace all is one undo step. The search follows your edits: in a
100,000-line file with 10,000 matches, a keystroke costs about 0.3 ms.

![The find bar, with every match highlighted](../media/find.svg)

## Files

- Saves are atomic and keep the file's permissions; symlinks stay
  symlinks.
- Line endings are kept byte for byte (LF, CRLF and even mixed files), and
  UTF-8 with or without a BOM. Binary files and invalid UTF-8 are refused
  instead of being corrupted.
- When a file changes on disk, an unmodified editor reloads it; one with
  unsaved changes asks you what to do.
- Files above `files.largeFileThresholdMb` (50 MiB by default) open, after
  asking, in a reduced-feature mode.
- Auto-save, trimming trailing whitespace and a final newline are
  settings (`files.autoSave`, `files.trimTrailingWhitespace`,
  `files.insertFinalNewline`).
- **Unsaved work survives a crash or a closed terminal.** A second after
  you edit, YACE backs up every unsaved file and untitled tab. If it ends
  without a clean quit (it crashes, is killed, or its terminal is closed),
  the next start in that folder reopens them with the unsaved text, still
  unsaved, and says so. The files on disk are never changed: Undo shows a
  file as it is on disk, and a file changed elsewhere in the meantime is
  named. Backups live in YACE's state folder
  (`~/.local/state/yace/backups/`, readable only by you) and are deleted
  when you save, close without saving, or quit.

## Tabs and splits

Preview tabs (a single click or `Space` in the Explorer opens a tab that
the next preview replaces), reopen closed editors (`Ctrl+Shift+T`),
most-recently-used switching (`Ctrl+Tab`), split right or down
(`Ctrl+\`, `Ctrl+K Ctrl+\`), moving editors between groups, maximising a
group (`Ctrl+K Ctrl+M`), and resizing with the mouse.

See the [editor keys](../docs/keybindings.md#editor-editorfocus) for the
full list.

## Vim keys

The [Vim plugin](https://github.com/abedmurrar/yace-vim) adds Normal,
Insert, Visual and Visual Line modes to every editor:

```sh
yace --install-plugin io.github.abedmurrar.yace-vim
```

- Motions, operators and text objects (`dw`, `ci"`, `yap`, `>j`), counts,
  registers (`"a`, `"_`), `.`, macros (`qa` … `q`, `@a`), `/` and `?`
  search with `n`, `*`, and `:w`, `:q`, `:wq`, `:%s/old/new/g`.
- The status bar shows the mode; the cursor is a block in Normal mode and a
  bar in Insert mode.
- Insert mode is YACE's own typing: completion, auto-closing brackets and
  snippets work as usual.
- YACE's shortcuts keep working next to it (`Ctrl+P`, `Ctrl+Shift+P`,
  `Ctrl+S`, `Ctrl+B`, `Ctrl+F`, `` Ctrl+` ``). "Vim: Toggle Vim Keys" turns
  it off and on.

Not covered yet: Vim script and `.vimrc`, marks, the `"+` register (use
`Ctrl+C`/`Ctrl+V`), regular expressions in search, and block Visual mode.
