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
| `e` | Open [yazi](https://yazi-rs.github.io) in the repository, with the highlighted file revealed when the changes list is focused. Open files from yazi with your usual opener (e.g. nvim). Without yazi installed it falls back to `$VISUAL` / `$EDITOR` |
| `t` | Open `$SHELL` in the repository; exit it to return |
| `o` | Open the repository folder in the file manager (`xdg-open`) |
| `m` | Merge a branch into the current one (empty input merges the upstream) |
| `r` / `R` | Reload the selected repository / rescan all roots |
| `:` / `Ctrl+P` | Command palette: starts empty, type a few letters of any action (fuzzy matched), `↑` `↓` pick, `Enter` runs it, `Esc` clears the query and a second `Esc` closes it |
| `h` | Hide / show the key bar at the bottom (the row goes back to the panels; prompts still use it) |
| `?` | Help |
| `q`, `Ctrl+C` | Quit |

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
| `Esc`, `q`, `d` | Close |

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
next to it. `GAP_X`, `GAP_Y` and `SIDE_PAD` tune the gaps and the inner side padding of panels. The
banner has a monochrome and a colored version and follows the same switch.

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

The art above the dashboard comes from [`src/banner.sh`](src/banner.sh). Oxide embeds the
file when it is built and converts its colors itself; the script is never run. Preview
it in any terminal with `bash src/banner.sh`.

- Every art line must be `echo -e "..."`; other lines in the file are ignored.
- With colors off, the art's colors are flattened to the single ink color: bold and bright colors
  (`97`, `91`-`96`) render bold, dark ones (`30`, `34`, `90`) dim, the rest normal, and background
  colors are ignored. With colors on, the art keeps its own foreground and background colors.
- Edit the script and rebuild to change the banner.
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
| `src/git.rs` | `git` CLI wrapper: status parsing, operations and diffs |
| `src/diff.rs` | Parses `git diff` output into rows for the diff viewer |
| `src/syntax.rs` | Dependency-free syntax highlighter used by the diff viewer |
| `src/repo.rs` | Repository model, discovery, owned/cloned classification |
| `src/worker.rs` | Background scan and operation threads |
| `src/config.rs` | Config file, roots and owner names |
| `src/palette.rs` | Command palette entries and fuzzy matcher |
| `src/marks.rs` | Pinned and ignored repositories (config entries and state file) |
| `src/logger.rs` | Persistent log file |
| `src/banner.rs` | Turns `src/banner.sh` into monochrome and colored Ratatui lines |
| `src/banner.sh` | The ASCII banner art shown on top of the dashboard |

## Versioning

Oxide follows a weekly release cadence. Changes are recorded in [CHANGELOG.md](CHANGELOG.md).

## License

[MIT](LICENSE)
