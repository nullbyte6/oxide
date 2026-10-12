# Changelog

All notable changes to Oxide are recorded here, newest version first.

## [0.1.0] - 2026-10-12

### Added
- Add Ratatui dashboard with repository list, details pane, changes list, log panel and help overlay.
- Add git backend driving the `git` CLI: status parsing, stage/unstage, commit, push, pull, fetch and merge.
- Add background repository scanner and operation workers so the UI never blocks.
- Add owned vs cloned filter based on remote account and git identity.
- Add live search, fetch-all and `--list` plain-text output.
- Add config file (`root`, `owner`, `depth`) and persistent log file with rotation.
- Add LICENSE (MIT) and README with usage, keys and configuration.
- Add ASCII banner on top of the dashboard, rendered from `src/banner.sh`: its ANSI colors are converted to Ratatui styles and it is hidden on small terminals.

- Add *Attention* filter (key `4`) for repositories that are dirty, ahead, behind or broken, and a `s` key that sorts the list by name, last commit, changes or commits behind; the order is shown in the panel title.
- Add pinned and ignored repositories: `pin` / `ignore` config entries, `*` / `x` keys that save to a `marks` state file, and `H` to show ignored ones.
- Add stash and stash pop (`z` / `Z`) with a `$n` stash counter on rows and in the details pane.
- Add `e` / `t` / `o` keys to open the selected file or repository in `$VISUAL` / `$EDITOR`, a shell or the file manager.
- Add a color layer on top of the monochrome design: accent hues for focus, branches, owned/ahead/staged, modified, behind/conflicts/errors, untracked files and running operations, plus the banner's original colors. The weights, symbols and inverse selection stay as they were, so the monochrome look is unchanged with colors off. Toggle with `C`, `color = on|off` in the config, or `NO_COLOR`.
- Add a diff viewer (`d`) with line numbers, hunk navigation, panning and staged/unstaged switching, loaded in the background; untracked files are shown as added.
- Add a dependency-free syntax highlighter for the diff viewer covering Rust, Python, JS/TS, Go, C/C++, Java-like languages, shell, TOML, JSON, YAML, Markdown, HTML, CSS, SQL, Lua and Ruby.
- Add commit log view (`v`) listing the last 500 commits with refs, authors and ages; `Enter` opens a commit in the diff viewer, which now handles several files per diff.
- Add branch management (`b`): list local and remote branches with upstream state and merged/stale tags, switch, create, merge and delete (with confirmation).
- Add `git init` (`I`) for any folder, creating it when missing; the new repository is added to the list and selected.
- Add command palette (`:` or `Ctrl+P`): a fuzzy launcher for every dashboard action that runs the action's own key, so both always behave the same.
- Add `h` to hide and show the key bar at the bottom, freeing its row for the panels; `hints = off` in the config starts it hidden. Text prompts and confirmations still use the row.
- Add bulk pull (`L`) and bulk push (`U`, asks for confirmation) for the repositories in view.

### Fixed
- Fix the key bar losing its last keys on narrow terminals: hints are now dropped from the middle by importance, always keeping `h`, `?` and `q`.
- Fix `e` opening the editor (nvim with its plugins): it now opens yazi in the repository with the highlighted file revealed, so files are opened individually from there; the editor is only used when yazi is not installed.
- Fix leftover status circles on repository rows from the old colored design, leaving two columns of left padding in their place; dirty, conflict and error state remains visible through the row counters and labels.
- Fix log timestamps to use the local timezone instead of UTC, in the log file (with UTC offset) and in the Log panel title.
- Fix scan listing broken repositories: a `.git` that is empty or points to a removed gitdir (dangling submodule) is no longer treated as a repository.

### Changed
- Update the command palette so the first `Esc` clears the typed query and a second one closes it.
- Update the command palette to start empty: the action list only appears once something is typed, and `Enter` does nothing until then.
- Update `h` and `l` (pane switching in the dashboard) to `h` hiding the key bar and `l` doing nothing; use `←` `→` or `Tab` to switch panes. `h` `l` still pan inside the diff viewer.
- Update default scan to cover every mounted drive (internal and external) instead of only the home directory, with a default depth of 6 levels.
- Update panel spacing: one column of inner side padding, one empty column between side-by-side panels and no empty row between stacked ones, which look equal.
- Update design to a monochrome look, banner included: a single ink color on the terminal background, with bold/dim/italic/inverse text and symbols instead of hues, thick focus borders, title notches, a scan progress bar and running-operations indicator, dimmed zero counters, fading log entries, footer key caps, a double-border help window and an inverse-only selected row (no marker).
