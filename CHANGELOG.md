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

### Changed
- Update default scan to cover every mounted drive (internal and external) instead of only the home directory, with a default depth of 6 levels.
- Update panel spacing: one column of inner side padding, one empty column between side-by-side panels and no empty row between stacked ones, which look equal.
- Update design to a monochrome look, banner included: a single ink color on the terminal background, with bold/dim/italic/inverse text and symbols instead of hues, thick focus borders, title notches, a scan progress bar and running-operations indicator, dimmed zero counters, fading log entries, footer key caps, a double-border help window and an inverse-only selected row (no marker).
