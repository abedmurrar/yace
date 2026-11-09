# Performance

**Performance is a requirement in YACE, and every claim comes with a
measurement you can repeat.** The numbers below are summarised from the
measurement notes in the repository's `TODO.md`. Unless a line says
otherwise, they were taken on an Intel i5-1135G7 laptop running Linux
6.11, with a release build.

## Targets and results

| Metric | Target | Measured | When |
|---|---|---|---|
| Cold start to a visible UI | 50 ms or less | 5.1 ms median, 5.7 ms 95th percentile (20 runs) | 2026-11-03 |
| Memory at idle, minimal | 20 MB or less | 5.4 MiB at the first frame | 2026-11-03 |
| Typing to render, 95th percentile | 5 ms or less | about 0.4 ms (0.24 ms to handle a key plus 0.18 ms to render), typing into a 6,000-line file; the terminal emulator's own drawing is not included | 2026-11-03 |
| CPU at idle | about zero | 0.00% of one core over 5 s | 2026-11-03 |

## A large repository (version 0.9.0, 2026-11-09)

Measured on a 44,100-file repository with 3,300 to 4,000 folders:

- **First frame:** median 3.90 to 4.06 ms.
- **Memory at idle:** 8.4 MiB before Quick Open is used; 15.2 to 15.6 MiB
  after the first Quick Open, which then watches 3,328 folders. Indexing
  at startup was tried and cost 17 MiB at idle, so indexing stays lazy.
- **Search as you type:** making the query longer searches only the files
  that matched before, 25 to 28 ms instead of a full walk of 183 to
  259 ms.
- **Live results:** updating the results for one changed file takes 2.7
  to 3.3 ms, on a background thread.
- **Find in a 100,000-line file with 10,000 matches:** 0.23 to 0.31 ms per
  keystroke, instead of 1.7 to 2.2 ms when the whole file was searched
  again.
- **Watching 4,001 folders:** 41 to 72 ms, on the watcher's thread.
- **CPU at idle:** 0.

## The terminal (version 0.10.0, 2026-11-09)

- **Find in the scrollback:** 14 to 21 ms over 10,000 lines, 140 to 160 ms
  over 100,000 lines. It runs on a worker in chunks of 1,000 lines, each
  holding the terminal for about 1.5 ms, so the shell's output keeps
  flowing.
- **Heavy output:** with `seq 1 2000000` running, editor input stays
  between 0.30 and 0.71 ms at the 95th percentile, with one pane or two.
- **Memory:** one terminal 10.6 to 10.9 MiB in total; four terminals 34.2
  to 34.4 MiB. A split costs nothing measurable.

## Earlier measurements (2026-11-03)

- **Text buffer:** typing one character into a 10 MB file takes 0.76 to
  1.15 µs; typing with 1,000 cursors takes 2.0 ms.
- **Syntax:** in a 10,000-line TypeScript file, a full parse takes 80 ms
  on a worker thread and re-parsing after a keystroke 0.92 ms;
  highlighting a 60-line screen takes 0.58 ms and is cached.
- **Quick Open:** a query over 200,000 paths takes 9 to 19 ms.
- **Git:** on a 9-folder workspace, the first frame takes the same time
  with and without Git (8.2 ms), and all 9 repository statuses arrive
  0.2 s after start.
- **Plugins:** calling into a plugin takes 76 ns; with 20 plugins
  installed, the first frame takes 4.77 ms against 4.75 ms with none.
- **Themes:** loading a built-in theme takes under 1 µs.
- **Language servers:** a Node.js TypeScript server and its helpers used
  538 MiB on a two-file project, while YACE used 32 MiB. YACE shows the
  servers' memory separately.

## Reproduce them

```sh
scripts/bench.sh 20 .                 # startup time and memory
scripts/bench_interactive.py 300      # idle CPU and typing latency
scripts/bench_terminal.py             # typing while a terminal floods output
cargo bench -p yace-buffer            # text buffer
cargo bench -p yace-syntax            # parsing and highlighting
cargo bench -p yace-search            # Quick Open and search
```

The repository's `TODO.md` has the full notes, including the exact
commands, the workspace each number came from, and how each feature's
cost was compared with the release before it.
