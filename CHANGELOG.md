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
- Add ASCII banner on top of the dashboard, loaded from `src/banner.sh`.
- Add banner renderer that converts the script's ANSI colors to Ratatui styles and hides the banner on small terminals.

### Changed
- Update panels with one column of inner side padding so all four sides look equally thick.
- Update default scan to cover every mounted drive (internal and external) instead of only the home directory.
- Update default scan depth from 5 to 6 levels.
