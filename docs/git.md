# Git

YACE shows Git information for **every repository in the workspace,
separately**: each workspace folder belongs to its own repository (or none),
with its own branch, changes and index. YACE uses the `git` program you
have installed, on background threads, so your Git configuration applies and
the editor never waits for it.

> **What runs automatically.** Opening a workspace and editing only *read*
> repositories (`git rev-parse`, `git status`, `git show`, `git diff`), and
> those reads never take the index lock (`--no-optional-locks`), so they
> can't get in the way of git commands you run yourself. Anything that
> changes a repository — staging, committing, switching branches — happens
> only when you ask for it. "Git: Show Output" lists every git command YACE
> ran.

## What you see

**Status bar.** The branch of the active file's repository (or of the
selected Explorer folder): `⎇ main* ↑1 ↓2` — `*` when there are changes,
`↑`/`↓` commits ahead of / behind its upstream, `(a1b2c3d)` when HEAD is
detached, `⚠ n` for merge conflicts.

**Explorer.** Changed files are coloured and marked: `M` modified, `A`
added, `U` untracked, `D` deleted, `R` renamed, `!` conflict; ignored files
are dimmed; folders containing changes get the colour and a `•`. Each
folder shows only its own repository's state.

**Gutter.** A bar before the text marks lines that differ from the
version in the index (staged, or the last commit): green for added,
the theme's "modified" colour for changed, and a red mark where lines were
deleted. It updates as you type, before you save. `Alt+F5` /
`Shift+Alt+F5` go to the next / previous change; "Git: Show Change Under
Cursor" shows the lines that were there before.

**Source Control view** (`Ctrl+Shift+G`). One section per repository with
its branch, then **Merge Conflicts**, **Staged Changes**, **Changes** and
**Untracked** with counts.

| Key | Action |
|---|---|
| `↑` `↓`, `←` `→` | move; collapse / expand a repository or group |
| `Enter` | open the file's changes (a diff); on a repository or group: collapse / expand |
| `o` | open the file |
| `s` / `u` | stage / unstage the file (on a group or repository: all of it) |
| `c` | commit (asks for the message) |
| `b` | switch branch (or create one) |
| `r` | refresh |

**Changes view.** A read-only diff (`file.ts (Working Tree)`, `(Index)` for
staged changes), coloured by line. `Enter` on a line opens the file at
that line.

## Commands

| Command | Default keys | |
|---|---|---|
| View: Show Source Control | `Ctrl+Shift+G` | `workbench.view.scm` |
| Git: Open Changes | | the active file's diff |
| Git: Stage Changes / Unstage Changes | | the active file (or the selection in the view) |
| Git: Stage All Changes / Unstage All Changes | | |
| Git: Commit | | staged changes; with nothing staged it offers to commit every change to tracked files |
| Git: Commit All | | `git commit --all` |
| Git: Checkout to… | | branches, most recently used first; a remote branch creates a local tracking branch; "Create new branch…" |
| Git: Create Branch… | | from the current commit, then switches to it |
| Git: Refresh | | |
| Git: Show Output | | every git command run, with timings and errors |
| Go to Next / Previous Change | `Alt+F5` / `Shift+Alt+F5` | |
| Git: Show Change Under Cursor | | |

When it isn't clear which repository a command is for (no file open,
several repositories), YACE asks — it never assumes the workspace is one
repository.

Errors from git (a failing hook, a branch that already exists, local changes
that would be overwritten) are shown with git's own message.

## When it refreshes

- when a workspace or folder is opened (after the first frame is drawn);
- when files change in YACE or on disk, and when the repository changes
  outside YACE (the `.git` folder is watched: branch switches, commits,
  staging and merges from a terminal show up);
- when the terminal window gets focus back;
- every 10 s while the Source Control view is visible;
- right after an operation.

Bursts are coalesced (300 ms) and each repository is refreshed on its own,
at most one `git status` at a time — a big repository never delays the
others.

## Settings

```toml
[git]
enabled = true              # git.enabled
path = "git"                # the git program (user settings only)
decorationsEnabled = true   # git.decorations.enabled: Explorer colours and letters
autoRefresh = true          # git.autorefresh
diffDecorations = "all"     # scm.diffDecorations: "all", "gutter" or "none"
```

The VS Code keys in brackets are read from workspace and folder settings.

## Not yet

Staging individual hunks, discarding changes, history, comparing revisions,
stash, and a side-by-side diff are planned. Plugins receive a
`gitStatusChanged` event (`{repo, roots, branch}`) when a repository's
state changes.
