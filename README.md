# Oxide

A fast terminal dashboard for git, written in Rust with [Ratatui](https://ratatui.rs).
Oxide finds every repository on your machine, shows the state of all of them in one
screen, and lets you stage, commit, push, pull, fetch and merge without leaving it.

```
┌─┤ ◆ OXIDE v0.1.0 ├───────────────────────── 12 repos · 3 dirty · 2 ahead · 1 behind ┐
│  All 12  ┊  Owned 7  ┊  Cloned 5                                                     │
└──────────────────────────────────────────────────────────────────────────────────────┘
┏─┤ Repositories · all (12) ├━━━━━━━━━━━━━━━┓ ┌─┤ oxide ├─────────────────────────────┐
┃   oxide        own   main     ↑1 ~2 ?7      ┃ │ path     /home/you/oxide              │
┃   ratatui      clone main     ↓3            ┃ │ branch   main  → origin/main          │
┃   notes        own   main                   ┃ │ owner    owned                        │
┗━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━┛ └───────────────────────────────────────┘
```

## Features

- **Whole machine**: scans every mounted drive (internal and external) by default, not just your home directory.
- **Every repo at a glance**: branch, ahead/behind, staged, modified, untracked and conflicted counts.
- **Owned vs cloned filter**: switch between *All*, *Owned*, *Cloned* and *Attention* (dirty, ahead, behind or broken) repositories.
- **Sorting, pinning and hiding**: sort by name, last commit, changes or commits behind; pin favourites to the top and ignore noisy repositories.
- **Git actions**: stage/unstage (per file or all), commit, push, pull, fetch (one repo or all), merge, stash and pop; pull or push every repository in view at once.
- **Diff viewer with syntax highlighting**: read the staged, unstaged or untracked diff of any changed file with line numbers and per-language colors, without leaving Oxide.
- **Commit log and branches**: browse a repository's history and open any commit's diff; list, switch, create, merge and delete branches, with merged and stale ones flagged.
- **`git init` anywhere**: initialise a repository in any folder, existing or new, and see it in the list right away.
- **GitHub** (needs the [`gh`](https://cli.github.com) CLI, logged in): browse a repository's issues and pull requests, read them, diff and check out pull requests, find repositories (yours or anyone's) and clone them, and open things in your browser.
- **Jump out**: open the selected file or repository in yazi, a shell or the file manager.
- **Non-blocking**: scans and git commands run on background threads; the UI never freezes.
- **Search**: filter the list live by name, branch or path.
- **Logs**: an in-app log panel plus a persistent log file.
- **Only needs `git`**: Oxide drives the `git` CLI, so it behaves exactly like your own git setup.

## Requirements

- Rust 1.88 or newer (edition 2024)
- `git` in your `PATH`
- A terminal with Unicode support

## Install and run

```bash
git clone <your-oxide-repo-url> oxide
cd oxide
cargo run --release
```

Install the binary into `~/.cargo/bin`:

```bash
cargo install --path .
oxide
```

## Usage

```
oxide [OPTIONS] [PATH...]

  PATH...        directories to scan (default: config roots, else every mounted drive)
      --list     print the repository table to stdout and exit (no TUI)
  -h, --help     show help
  -V, --version  show the version
```

## Keys

| Key | Action |
| --- | --- |
| `↑` `↓` / `j` `k` | Move selection |
| `g` / `G` | First / last item |
| `Tab`, `←` `→` | Switch between the repository list and the changes list |
| `f`, `1` `2` `3` `4` | Filter: cycle, or All / Owned / Cloned / Attention |
| `s` | Sort: name / recent / changes / behind (pinned repositories always come first) |
| `D` | **Delete the repository's folder from disk** (see below) |
| `*` / `x` | Pin / ignore the selected repository |
| `H` | Show ignored repositories (marked `⊘`) |
| `C` | Toggle colors |
| `/` | Search (live); `Esc` clears |
| `Space` / `Enter` | Stage or unstage the highlighted file |
| `d` | Open the diff viewer on the highlighted file (see below) |
| `a` / `u` | Stage all / unstage all |
| `c` | Commit staged changes (prompts for a message) |
| `p` | Pull (fast-forward only) |
| `P` | Push (a branch without upstream is published with `-u`) |
| `F` | Fetch the selected repository (`--all --prune`) |
| `A` | Fetch every repository in the current view |
| `L` | Pull (fast-forward only) every repository in the view that is behind its upstream |
| `U` | Push every repository in the view that is ahead of its upstream (asks `y` to confirm) |
| `z` / `Z` | Stash all changes including untracked files / pop the latest stash |
| `v` | Commit log of the selected repository (see below) |
| `b` | Branches of the selected repository (see below) |
| `I` | `git init` in a folder: the prompt starts with the current directory, append a name for a new folder |
| `i` / `y` | GitHub issues / pull requests of the selected repository (see below) |
| `B` | Open the selected repository's GitHub page in the browser |
| `S` | Find GitHub repositories and clone them (see below) |
| `e` | Open [yazi](https://yazi-rs.github.io) in the repository, with the highlighted file revealed when the changes list is focused. Open files from yazi with your usual opener (e.g. nvim). Without yazi installed it falls back to `$VISUAL` / `$EDITOR` |
| `t` | Open `$SHELL` in the repository; exit it to return |
| `o` | Open the repository folder in the file manager (`xdg-open`) |
| `m` | Merge a branch into the current one (empty input merges the upstream) |
| `r` / `R` | Reload the selected repository / rescan all roots |
| `:` / `Ctrl+P` | Command palette: starts empty, type a few letters of any action (GitHub ones included: `clone`, `issues`, `browser` ...) (fuzzy matched), `↑` `↓` pick, `Enter` runs it, `Esc` clears the query and a second `Esc` closes it |
| `l` | Hide / show the log panel. While it is hidden, each new log message takes the bottom row for a few seconds so results and errors are not missed |
| `h` | Hide / show the key bar at the bottom (the row goes back to the panels; prompts still use it) |
| `?` | Help |
| `q`, `Ctrl+C` | Quit |

### GitHub

Oxide talks to GitHub through the [`gh`](https://cli.github.com) CLI, so it never handles a token: log in
once with `gh auth login`. A repository needs a `github.com` remote (ssh, https or git URL); others
report "no github.com remote" in the log.

`i` lists the repository's issues and `y` its pull requests (the newest 100), full screen. Issue rows
show the number, title, labels, author and age of the last update; pull request rows show
the source branch instead of labels and mark drafts `◌`, approved `✓` and changes requested `±`.

| Key | Action |
| --- | --- |
| `↑` `↓` / `j` `k`, `g` / `G`, `Space` / `b` | Move |
| `Tab` | Switch between issues and pull requests |
| `s` | State filter: open, closed, (merged for pull requests), all |
| `Enter` | Read the item: description and comments as text (`o` opens it in the browser, `Esc` goes back) |
| `d` | Pull requests: the diff in the diff viewer, one section per file |
| `c` | Pull requests: check the branch out in the local repository (asks `y` to confirm) |
| `o` | Open the highlighted item in the browser |
| `B` | Open the repository in the browser |
| `r` | Reload |
| `Esc`, `q` | Close |

#### Finding and cloning repositories

`S` asks for search terms (the same syntax as GitHub's search, e.g. `tui language:rust`); an empty search
lists your own repositories, private ones included. Results show the stars, language, age of the last
push, `private` / `fork` tags and the description, and repositories you already have locally are marked
`cloned`.

| Key | Action |
| --- | --- |
| `Enter` | Clone the repository: the prompt starts with `clone_dir/<name>`, edit it freely (relative paths start from `clone_dir`). The new repository is added to the list and selected. On one you already have, Enter jumps to the local copy instead |
| `/` or `S` | New search |
| `o` | Open the highlighted repository in the browser |
| `r` | Reload |
| `Esc`, `q` | Close |

The clone uses the protocol configured in `gh` (ssh or https). Like `git init`, a clone outside the scan
roots disappears on the next full rescan (`R`); `clone_dir` is the best place to also list as a `root`.

### Commit log

`v` lists the last 500 commits of the current branch with hash, age, author, branches and tags, and subject.
`Enter` (or `d`) opens the commit in the diff viewer, one section per file; `Esc` there returns to the log.
Keys: `↑` `↓` / `j` `k`, `Space` / `PgDn`, `b` / `PgUp`, `g` / `G`, `Esc` / `q` / `v` to close.

### Branches

`b` lists local branches first, then remote-tracking ones, with the upstream and its ahead/behind state,
a green `merged` tag when the branch is already merged into the current one and a yellow `stale` tag when
its last commit is older than 90 days. The last log message shows below the list, so the result of an
action stays visible.

| Key | Action |
| --- | --- |
| `Enter` | Switch to the branch (a remote branch gets a local tracking branch) |
| `n` | Create a branch from the current one and switch to it |
| `d` / `D` | Delete a local branch (`d` only if it is merged, `D` even if it is not); asks `y` to confirm |
| `m` | Merge the branch into the current one (asks `y` to confirm) |
| `Esc`, `q`, `b` | Close |

### Deleting a repository

`D` removes the selected repository's whole folder from disk, not just its entry in the list (`x` ignores a
repository without touching it). Nothing happens until you type the repository's exact name at the prompt,
which spells out what would be lost: uncommitted files, unpushed commits, stashes, and a warning in capitals
when there is no remote and the folder is the only copy. Any other input, or `Esc`, cancels.

On top of the prompt, Oxide refuses to delete a symlink, a worktree or submodule (whose `.git` is a file),
anything that is not a standalone repository folder, and any folder that is or contains your home directory.
Deleting is final: it does not use a trash.

### git init

`I` asks for a folder. A relative path starts from the directory Oxide was launched in and `~` is expanded;
a folder that does not exist is created. The new repository appears in the list and is selected. A repository
outside the scan roots disappears on the next full rescan (`R`), so add its folder as a `root` to keep it.

### Diff viewer

`d` opens the highlighted file's diff full screen: untracked files are shown as entirely added, and a file
with both staged and unstaged changes starts with the unstaged part (`Tab` switches). Lines carry old and
new line numbers, additions and deletions get a `+` / `-` marker, and the code is highlighted per
language (Rust, Python, JavaScript/TypeScript, Go, C/C++, Java/Kotlin/C#, shell, TOML/INI, JSON, YAML,
Markdown, HTML/XML, CSS, SQL, Lua and Ruby; other files stay plain). With colors off, only weight
differs: bold keywords and dim italic comments.

| Key | Action |
| --- | --- |
| `↑` `↓` / `j` `k` | Scroll one row |
| `Space` / `PgDn`, `b` / `PgUp` | Page down / up |
| `g` / `G` | Top / bottom |
| `]` / `[` (or `n` / `N`) | Next / previous hunk |
| `←` `→` / `h` `l`, `0` | Pan long lines, back to the left edge |
| `Tab` | Staged / unstaged version, when the file has both |
| `s` | Stage the hunk at the top of the screen (unstaged version), or unstage it (staged version) |
| `Esc`, `q`, `d` | Close |

The hunk `s` acts on is marked `▸` and counted in the title (`hunk 2/5`); `[` `]` move between hunks. The
diff reloads after each stage, and once a file has no hunks left on one side the viewer follows it to the other.

Row symbols: `↑n` ahead, `↓n` behind, `+` staged, `~` modified, `?` untracked, `!` conflicts, `$n` stashes;
`★` marks a pinned repository and `⊘` an ignored one.

## Design

Oxide's design is monochrome at its core: a single ink color on your terminal's own background,
with hierarchy from bold, dim, italic and inverse text. On top of it sits an optional **color
layer** of accent hues (ANSI colors, so they follow your terminal theme): cyan for focus, branches
and the logo, green for owned/ahead/staged, yellow for modified and warnings, red for behind,
conflicts and errors, blue for untracked files, magenta for running operations and stashes. Every
colored element keeps the weight and symbol it has without color, so nothing depends on hue.
Press `C` to toggle colors, set `color = off` in the config, or export `NO_COLOR`. The selected row
stays plain inverse.

- **Spacing**: side-by-side panels are 1 empty column apart and stacked panels touch (no empty
  row), which look equal because terminal cells are about twice as tall as wide. The focused
  panel gets a thick border.
- **Selection** is an inverse row (faded in the pane that has no focus), without any marker.
- **Activity**: a spinner and progress bar (`▰▰▰▱▱▱▱▱▱▱`) in the header while scanning, and the
  number of running git operations; busy rows show a spinner next to the running operation.
- **Dimming**: zero counters, hashes, labels and cloned repositories are dimmed, and log
  entries fade as they age (the newest three keep full weight).
- **Key caps** in the footer, `─┤ title ├─` notches on panel titles, a double-border help window.

To recolor the base, change `INK` in [`src/ui.rs`](src/ui.rs) (the default is the terminal's
own foreground); the accent hues are the `ACCENT`, `GOOD`, `WARN`, `BAD`, `INFO` and `PINK` constants
next to it. `GAP_X`, `GAP_Y`, `BANNER_GAP` and `SIDE_PAD` tune the gaps (`BANNER_GAP` is the empty row under the banner) and the inner side padding of panels. The
banner is bold ink, or bold accent color with colors on.

## Owned vs cloned

A repository is **owned** when any of these is true:

- it has no remote (local only), or its remote is a local path;
- the account in its remote URL (`github.com/<account>/repo`) matches one of your owner names.

Everything else is **cloned**. Your owner names are your global git `user.name` and
`github.user`, plus any `owner` entries in the config file.

## Configuration

Optional file: `~/.config/oxide/config` (or `$XDG_CONFIG_HOME/oxide/config`).

```
# directories to scan (repeatable); without any root, every mounted drive is scanned
root = ~/dev
root = /mnt/backup

# extra account names that count as "owned" (repeatable)
owner = my-company

# how many directory levels below each root are searched
depth = 6

# accent colors on top of the monochrome design (on or off; off by default when NO_COLOR is set)
color = on

# start with the key bar at the bottom visible (on) or hidden (off); the h key toggles it
hints = on

# start with the log panel visible (on) or hidden (off); the l key toggles it
logs = on

# folder that GitHub repositories are cloned into by default (home directory when unset)
clone_dir = ~/dev

# repositories listed first / hidden: a directory name, or a path (a value with a "/" or a leading "~"
# also covers every repository below it)
pin = oxide
ignore = ~/dev/vendor
```

The `*` and `x` keys store their marks in `~/.local/state/oxide/marks` (or `$XDG_STATE_HOME/oxide/marks`),
so the config file is never rewritten. A mark that comes from the config file can only be removed there.

## What gets scanned

By default Oxide reads `/proc/mounts` and scans each mounted block-device filesystem
(`/`, `/home`, external drives under `/mnt`, `/media` or `/run/media`, ...). Each drive is
its own root and a scan never crosses onto another filesystem, so nothing is visited twice.
Skipped: `/boot`, snap and squashfs images, network mounts, and the system directories
`/proc`, `/sys`, `/dev`, `/run`, `/usr` (only directly under `/`).

`root` entries in the config file, or paths on the command line, replace this automatic
list; use them to scan only part of the machine or a drive that is not mounted via a block device.
Hidden directories, `node_modules`, `target`, `venv`, `__pycache__`, `snap` and the Windows
`$RECYCLE.BIN` / `System Volume Information` folders are always skipped, and a repository's
subdirectories are not searched for nested repositories. A directory only counts as a repository
when its `.git` is usable, so dangling submodule checkouts and empty `.git` folders are ignored.

## Banner

The art above the dashboard comes from [`src/banner.txt`](src/banner.txt). Oxide embeds the
file when it is built and shows it as is, in bold: in the single ink color, or in the accent
color when colors are on.

- Plain text, any art: edit the file and rebuild to change the banner.
- Trailing spaces and blank lines around the art are ignored; leading spaces are kept.
- The banner is hidden when the terminal is narrower than the art or has fewer than
  24 rows to spare, so the dashboard always keeps its space.

## Logs

Oxide writes every action to `~/.local/state/oxide/oxide.log` (or
`$XDG_STATE_HOME/oxide/oxide.log`), with local-time timestamps that include the UTC offset. The timezone is read once at start-up, so a daylight-saving change while Oxide is open shows after a restart. The file rotates to
`oxide.log.1` at 1 MiB. The last lines are also shown in the Log panel.

## Remotes and credentials

Oxide never prompts for credentials, since a prompt would break the screen. Use
an SSH agent or a key without a passphrase, or an HTTPS credential helper. SSH is
run in batch mode unless you set `core.sshCommand`, `GIT_SSH_COMMAND` or `GIT_SSH`.

## Project layout

| File | Purpose |
| --- | --- |
| `src/main.rs` | CLI arguments, terminal setup, event loop |
| `src/app.rs` | Application state and key handling |
| `src/ui.rs` | Ratatui rendering |
| `src/github.rs` | GitHub through the `gh` CLI: remote parsing, issue and pull request lists |
| `src/git.rs` | `git` CLI wrapper: status parsing, operations and diffs |
| `src/diff.rs` | Parses `git diff` output into rows for the diff viewer |
| `src/syntax.rs` | Dependency-free syntax highlighter used by the diff viewer |
| `src/repo.rs` | Repository model, discovery, owned/cloned classification |
| `src/worker.rs` | Background scan and operation threads |
| `src/config.rs` | Config file, roots and owner names |
| `src/palette.rs` | Command palette entries and fuzzy matcher |
| `src/marks.rs` | Pinned and ignored repositories (config entries and state file) |
| `src/logger.rs` | Persistent log file |
| `src/banner.rs` | Turns `src/banner.txt` into Ratatui lines |
| `src/banner.txt` | The ASCII banner art shown on top of the dashboard |

## Versioning

Oxide follows a weekly release cadence. Changes are recorded in [CHANGELOG.md](CHANGELOG.md).

## License

[MIT](LICENSE)
