# Credits

YACE stands on the shoulders of excellent open-source work. These projects
do the heavy lifting; thank you to their authors and contributors.

## Core building blocks

| Project | What it does in YACE | Authors | Licence |
|---|---|---|---|
| [alacritty_terminal](https://github.com/alacritty/alacritty) and [vte](https://github.com/alacritty/vte) | Terminal emulation and PTY handling for the integrated terminal | Christian Duerr, Joe Wilm and the Alacritty contributors | Apache-2.0 (vte: Apache-2.0 OR MIT) |
| [ripgrep's crates](https://github.com/BurntSushi/ripgrep): `ignore`, `grep-searcher`, `grep-regex`, `globset` | File discovery that honours `.gitignore`, workspace content search, glob matching | Andrew Gallant and contributors | Unlicense OR MIT |
| [regex](https://github.com/rust-lang/regex) | Regular expressions in find, replace and search | The Rust Project Developers, Andrew Gallant | MIT OR Apache-2.0 |
| [tree-sitter](https://github.com/tree-sitter/tree-sitter) | Incremental parsing and syntax highlighting queries | Max Brunsfeld, Amaan Qureshi and the tree-sitter contributors | MIT |
| tree-sitter grammars: [rust](https://github.com/tree-sitter/tree-sitter-rust), [javascript](https://github.com/tree-sitter/tree-sitter-javascript), [typescript](https://github.com/tree-sitter/tree-sitter-typescript), [json](https://github.com/tree-sitter/tree-sitter-json), [html](https://github.com/tree-sitter/tree-sitter-html), [css](https://github.com/tree-sitter/tree-sitter-css), [python](https://github.com/tree-sitter/tree-sitter-python), [go](https://github.com/tree-sitter/tree-sitter-go), [bash](https://github.com/tree-sitter/tree-sitter-bash), [c](https://github.com/tree-sitter/tree-sitter-c), [cpp](https://github.com/tree-sitter/tree-sitter-cpp), [java](https://github.com/tree-sitter/tree-sitter-java), [ruby](https://github.com/tree-sitter/tree-sitter-ruby) (tree-sitter org); [markdown](https://github.com/tree-sitter-grammars/tree-sitter-markdown), [yaml](https://github.com/tree-sitter-grammars/tree-sitter-yaml), [toml](https://github.com/tree-sitter-grammars/tree-sitter-toml) (tree-sitter-grammars); [scss](https://github.com/serenadeai/tree-sitter-scss); [sql](https://github.com/derekstride/tree-sitter-sql); [containerfile](https://github.com/wharflab/tree-sitter-containerfile); [lua](https://github.com/tree-sitter-grammars/tree-sitter-lua), [make](https://github.com/tree-sitter-grammars/tree-sitter-make) (tree-sitter-grammars); [ini](https://github.com/justinmk/tree-sitter-ini) (Justin M. Keyes, Apache-2.0, also used for git config files) | The built-in languages and their highlight/injection queries | their respective authors | MIT (ini: Apache-2.0) |
| [Ratatui](https://github.com/ratatui/ratatui) | Terminal UI rendering and cell diffing | Florian Dehau and the Ratatui developers | MIT |
| [crossterm](https://github.com/crossterm-rs/crossterm) | Terminal input/output, the keyboard protocol, mouse | T. Post and contributors | MIT |
| [Ropey](https://github.com/cessen/ropey) | The rope text buffer behind every document | Nathan Vegdahl | MIT |
| [jsonc-parser](https://github.com/dprint/jsonc-parser) | Reading and non-destructively editing `.code-workspace` files | David Sherret | MIT |
| [notify](https://github.com/notify-rs/notify) | Filesystem watching | Félix Saparelli, Daniel Faust and contributors | CC0-1.0 |
| [unicode-segmentation](https://github.com/unicode-rs/unicode-segmentation) and [unicode-width](https://github.com/unicode-rs/unicode-width) | Grapheme-correct cursors and display widths | kwantam, Manish Goregaokar | MIT OR Apache-2.0 |
| [Git](https://git-scm.com) | Repository status, diffs and operations (run as an external program, not bundled) | the Git developers | GPL-2.0 (not linked) |
| [imara-diff](https://github.com/pascalkuthe/imara-diff) | Line diffs for the gutter | Pascal Kuthe | Apache-2.0 |
| [libloading](https://github.com/nagisa/rust_libloading) | Loading native plugins and grammar libraries | Simonas Kazlauskas and contributors | ISC |
| [sha2](https://github.com/RustCrypto/hashes), [tar](https://github.com/alexcrichton/tar-rs), [flate2](https://github.com/rust-lang/flate2-rs), [semver](https://github.com/dtolnay/semver) | Plugin approval hashes, packages, version requirements | RustCrypto developers, Alex Crichton, David Tolnay and contributors | MIT OR Apache-2.0 |
| [arboard](https://github.com/1Password/arboard) | System clipboard | 1Password and contributors | MIT OR Apache-2.0 |
| [trash](https://github.com/ArturKovacs/trash) | Move to Trash from the Explorer | Artur Kovacs | MIT |
| [clap](https://github.com/clap-rs/clap), [serde](https://github.com/serde-rs/serde), [toml and toml_edit](https://github.com/toml-rs/toml), [crossbeam](https://github.com/crossbeam-rs/crossbeam), [directories](https://github.com/soc/directories-rs), [tracing](https://github.com/tokio-rs/tracing), [tempfile](https://github.com/Stebalien/tempfile), [signal-hook](https://github.com/vorner/signal-hook), [smallvec](https://github.com/servo/rust-smallvec) | CLI, configuration, channels, XDG paths, logging, atomic files, signals | their respective authors | MIT and/or Apache-2.0 |

## Theme palettes

The built-in themes' colours come from these themes (all MIT); their
licence notices are in [`THIRD_PARTY_LICENSES.md`](THIRD_PARTY_LICENSES.md#theme-palettes).
YACE ships its own theme files (`crates/yace/themes/`), not the original
packages.

| Theme in YACE | Palette from | Authors |
|---|---|---|
| One Dark | [Atom One Dark](https://github.com/atom/atom/tree/master/packages/one-dark-syntax); terminal colours from [One Dark Pro](https://github.com/Binaryify/OneDark-Pro) | GitHub Inc.; Binaryify |
| Dracula | [Dracula](https://draculatheme.com) | Zeno Rocha and the Dracula Theme contributors |
| Catppuccin Mocha | [Catppuccin](https://catppuccin.com) | the Catppuccin organisation |
| Tokyo Night | [Tokyo Night](https://github.com/enkia/tokyo-night-vscode-theme) | enkia |
| YACE Dark / YACE Light | YACE; syntax colours modelled on VS Code's Dark+ / Light+ | — |

The complete list of the crates compiled into the `yace` binary, with each
crate's version, licence and licence texts, is in
[`THIRD_PARTY_LICENSES.md`](THIRD_PARTY_LICENSES.md). It is generated by
`scripts/third_party.py`. One dependency, `option-ext`, is MPL-2.0; it is
used unmodified, and its source is available from crates.io.

## Inspiration

- **[Visual Studio Code](https://github.com/microsoft/vscode)** (MIT):
  YACE follows its keyboard shortcuts, its command ids, the
  `.code-workspace` format and its multi-root workflow, so existing muscle
  memory carries over. YACE contains no VS Code code.
- **[Alacritty](https://github.com/alacritty/alacritty)**,
  **[Helix](https://github.com/helix-editor/helix)** and
  **[Zed](https://github.com/zed-industries/zed)** showed that native editors
  and terminals can be fast and pleasant.

## Development

YACE was designed and is maintained by Abed Murrar. It was developed with
the help of Anthropic's Claude (Claude Code), which wrote much of the code
under the maintainer's direction; commits carry a `Co-Authored-By` line where that
applies.
