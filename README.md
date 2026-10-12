# Oxide

A fast terminal dashboard for git, written in Rust with [Ratatui](https://ratatui.rs).
Oxide finds every repository on your machine, shows the state of all of them in one
screen, and lets you stage, commit, push, pull, fetch and merge without leaving it.

```
┌ ◆ OXIDE v0.1.0 ───────────────────────────── 12 repos · 3 dirty · 2 ahead · 1 behind ┐
│ All (12) │ Owned (7) │ Cloned (5)                                                    │
└──────────────────────────────────────────────────────────────────────────────────────┘
┌ Repositories · all (12) ─────────────────┐┌ oxide ──────────────────────────────────┐
│▶ ● oxide        own   main     ↑1 ~2 ?7  ││ path     /home/you/oxide                │
│  ● ratatui      clone main     ↓3        ││ branch   main  → origin/main            │
│  ● notes        own   main               ││ owner    owned                          │
└──────────────────────────────────────────┘└─────────────────────────────────────────┘
```

## Features

- **Whole machine**: scans every mounted drive (internal and external) by default, not just your home directory.
- **Every repo at a glance**: branch, ahead/behind, staged, modified, untracked and conflicted counts.
- **Owned vs cloned filter**: switch between *All*, *Owned* and *Cloned* repositories.
- **Git actions**: stage/unstage (per file or all), commit, push, pull, fetch (one repo or all), merge.
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
| `Tab`, `←` `→`, `h` `l` | Switch between the repository list and the changes list |
| `f`, `1` `2` `3` | Filter: cycle, or All / Owned / Cloned |
| `/` | Search (live); `Esc` clears |
| `Space` / `Enter` | Stage or unstage the highlighted file |
| `a` / `u` | Stage all / unstage all |
| `c` | Commit staged changes (prompts for a message) |
| `p` | Pull (fast-forward only) |
| `P` | Push (a branch without upstream is published with `-u`) |
| `F` | Fetch the selected repository (`--all --prune`) |
| `A` | Fetch every repository in the current view |
| `m` | Merge a branch into the current one (empty input merges the upstream) |
| `r` / `R` | Reload the selected repository / rescan all roots |
| `?` | Help |
| `q`, `Ctrl+C` | Quit |

Status symbols: `○` clean, `●` dirty, `✖` conflicts or error.
Row symbols: `↑n` ahead, `↓n` behind, `+` staged, `~` modified, `?` untracked, `!` conflicts.

## Design

Oxide is monochrome: a single ink color on your terminal's own background. Hierarchy comes
from bold, dim, italic and inverse text rather than hues:

- **Spacing**: side-by-side panels are 1 empty column apart and stacked panels touch (no empty
  row), which look equal because terminal cells are about twice as tall as wide. The focused
  panel gets a thick border.
- **Selection** is an inverse row (faded in the pane that has no focus), without any marker.
- **Activity**: a spinner and progress bar (`▰▰▰▱▱▱▱▱▱▱`) in the header while scanning, and the
  number of running git operations; a spinner replaces the status symbol of busy rows.
- **Dimming**: zero counters, hashes, labels and cloned repositories are dimmed, and log
  entries fade as they age (the newest three keep full weight).
- **Key caps** in the footer, `─┤ title ├─` notches on panel titles, a double-border help window.

To recolor everything, change `INK` in [`src/ui.rs`](src/ui.rs) (the default is the terminal's
own foreground). `GAP_X`, `GAP_Y` and `SIDE_PAD` tune the gaps and the inner side padding of panels. The banner follows the
same rule.

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
```

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
subdirectories are not searched for nested repositories.

## Banner

The art above the dashboard comes from [`src/banner.sh`](src/banner.sh). Oxide embeds the
file when it is built and converts its colors itself; the script is never run. Preview
it in any terminal with `bash src/banner.sh`.

- Every art line must be `echo -e "..."`; other lines in the file are ignored.
- The art's colors are flattened to the single ink color: bold and bright colors (`97`, `91`-`96`)
  render bold, dark ones (`30`, `34`, `90`) dim, the rest normal. Background colors are ignored.
- Edit the script and rebuild to change the banner.
- The banner is hidden when the terminal is narrower than the art or has fewer than
  24 rows to spare, so the dashboard always keeps its space.

## Logs

Oxide writes every action to `~/.local/state/oxide/oxide.log` (or
`$XDG_STATE_HOME/oxide/oxide.log`), with UTC timestamps. The file rotates to
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
| `src/git.rs` | `git` CLI wrapper: status parsing and operations |
| `src/repo.rs` | Repository model, discovery, owned/cloned classification |
| `src/worker.rs` | Background scan and operation threads |
| `src/config.rs` | Config file, roots and owner names |
| `src/logger.rs` | Persistent log file |
| `src/banner.rs` | Turns `src/banner.sh` into monochrome Ratatui lines |
| `src/banner.sh` | The ASCII banner art shown on top of the dashboard |

## Versioning

Oxide follows a weekly release cadence. Changes are recorded in [CHANGELOG.md](CHANGELOG.md).

## License

[MIT](LICENSE)
