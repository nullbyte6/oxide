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
