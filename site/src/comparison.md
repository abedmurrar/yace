# How YACE compares

YACE sits between VS Code and the terminal editors: VS Code's keys,
commands and multi-root workspaces, in a single native program that runs
in your terminal. Here is how it compares with editors people usually
weigh it against. Every cell for another editor comes from that project's
own documentation (sources below); if something there is wrong or out of
date, please [open an issue](https://github.com/abedmurrar/yace/issues).

| | YACE | Helix | Micro | Neovim | VS Code |
|---|---|---|---|---|---|
| Runs in | terminal | terminal | terminal | terminal | desktop window (Electron) |
| Keys | VS Code's, or Vim's with a plugin | modal (Vim-like)[^hx-modal] | Ctrl+S, Ctrl+C…[^mi-keys] | modal (Vim) | VS Code's |
| Opens `.code-workspace` files (multi-root) | yes | no | no | no | yes[^vsc-multi] |
| Language servers | built in | built in[^hx-readme] | plugin[^mi-plugins] | built in[^nv-lsp] | built in |
| Syntax highlighting | tree-sitter | tree-sitter[^hx-readme] | regex (YAML files)[^mi-syntax] | tree-sitter[^nv-ts] or regex | TextMate grammars[^vsc-syntax] |
| Integrated terminal | yes, with splits | no[^hx-faq] | yes (`term`)[^mi-term] | yes (`:terminal`)[^nv-term] | yes |
| Git | changes, diffs, stage, commit, branches | diff gutter, branch name[^hx-git] | diff gutter (plugin)[^mi-git] | plugins | built in |
| Plugins | native (Rust or C ABI) | not yet[^hx-faq] | Lua[^mi-plugins] | Lua, Vimscript | JavaScript/TypeScript[^vsc-ext] |
| First frame[^bench] | 12 ms | 286 ms | 49 ms | 58 ms | — |
| Memory when idle[^bench] | 16.8 MiB | 26.3 MiB | 18.8 MiB | 14.0 MiB | — |

**When another editor fits better.**
- If you like modal editing, Helix and Neovim are built around it. YACE's
  [Vim plugin](https://github.com/abedmurrar/yace-vim) covers everyday Vim
  (modes, motions, operators, text objects, registers, `.`, macros, `/`,
  `:w`), not Vim script or Neovim's plugins.
- Neovim has a far larger plugin ecosystem, and it used slightly less
  memory than YACE in this measurement.
- VS Code runs extensions written for it, debugs programs and works on
  macOS and Windows. YACE runs on Linux only, for now, and has no debugger.

YACE is for you if you want VS Code's keys and workspaces without leaving
the terminal, for example over SSH.

## How the numbers were measured

`scripts/bench_compare.sh` installs YACE 0.11.3 (from its APT
repository), Helix 25.07.1, Micro 2.0.15 and Neovim 0.12.6 (their official
release builds) in one Ubuntu 24.04 container. Each editor runs with its
defaults: no configuration and no language server. Each one opens the same
2,567-line Rust file (`crates/yace/src/ui.rs`) in a 120×34 tmux pane.

- **First frame:** the time from starting the editor until the pane shows
  the file's first line.
- **Memory:** the proportional set size (PSS) of the editor and every
  process it started, after 3 seconds idle. PSS shares libraries fairly
  between processes, so Neovim's two processes (interface and server)
  aren't counted twice.

The table shows the median of 5 runs, measured on 2026-11-09 on an Intel Core
i5-1135G7 laptop. Run it yourself with `scripts/bench_compare.sh`. VS Code is a
desktop app, so these terminal measurements don't apply to it.

## Sources

[^hx-modal]: Helix keymap: "Helix is designed to be a modal editor",
    <https://docs.helix-editor.com/keymap.html>
[^hx-readme]: Helix features: "Built-in language server support" and
    "syntax highlighting … via tree-sitter",
    <https://github.com/helix-editor/helix>
[^hx-faq]: Helix FAQ: run code in a separate terminal pane; a plugin
    system is being worked on, <https://github.com/helix-editor/helix/wiki/FAQ>
[^hx-git]: Helix diff gutter and `]g` / `[g` to move between changes,
    <https://docs.helix-editor.com/editor.html>,
    <https://docs.helix-editor.com/keymap.html>
[^mi-keys]: Micro default keys,
    <https://github.com/zyedidia/micro/blob/master/runtime/help/keybindings.md>
[^mi-plugins]: Micro plugins are written in Lua; language server support
    comes from a plugin, not the editor,
    <https://github.com/zyedidia/micro/blob/master/runtime/help/plugins.md>
[^mi-syntax]: Micro syntax files,
    <https://github.com/zyedidia/micro/tree/master/runtime/syntax>
[^mi-term]: Micro's `term` command,
    <https://github.com/zyedidia/micro/blob/master/runtime/help/commands.md>
[^mi-git]: Micro's `diffgutter` option and `diff` plugin,
    <https://github.com/zyedidia/micro/blob/master/runtime/help/options.md>
[^nv-lsp]: Neovim LSP, <https://neovim.io/doc/user/lsp/>
[^nv-ts]: Neovim tree-sitter, <https://neovim.io/doc/user/treesitter/>
[^nv-term]: Neovim terminal, <https://neovim.io/doc/user/terminal/>
[^vsc-multi]: VS Code multi-root workspaces,
    <https://code.visualstudio.com/docs/editing/workspaces/multi-root-workspaces>
[^vsc-syntax]: VS Code syntax highlighting,
    <https://code.visualstudio.com/api/language-extensions/syntax-highlight-guide>
[^vsc-ext]: VS Code extension API,
    <https://code.visualstudio.com/api/get-started/your-first-extension>
[^bench]: See "How the numbers were measured" above.
