# Changelog

All notable changes to Oxide are recorded here, newest version first.

## [0.2.0] - 2026-10-13

### Added
- Add an embedded terminal (`src/term.rs`): `T` opens a shell in the selected repository on a pseudo-terminal (`portable-pty`) parsed by a `vt100` screen and drawn full-screen with the shell's own colors, cursor and full-screen programs; `Ctrl+T` hides it while the shell keeps running, `Shift+PgUp` / `Shift+PgDn` scroll 5000 lines of history, and `exit` closes it and reloads the repository.
- Add the new-project menu (`K`, also in the command palette) for empty repositories: a language page with a type-to-filter box and an *Other / manual…* row that opens the embedded terminal, a variant page for languages with several build tools, a license page with an editable holder and the year, and a confirmation page listing every command and file plus an opt-in first commit. It builds on: 25 languages with their variants in `src/scaffold.rs`, short `.gitignore` templates per language family in `src/ignore.rs`, 11 licenses in `src/license.rs` (full texts under `src/licenses/`), and a `Plan` that resolves a choice into the exact commands, directories and files, leaves out commands whose program is not installed and refuses anything but an empty repository.
- Add the setup itself: `Enter` on the confirmation page runs the plan on a background thread (`Op::Setup`, so the repository shows the running indicator and reloads when done) with one log line per step, stops at the first failing step and says which one. Files are written as the plan says: replaced, kept when a tool already made them (`README.md`), or merged line by line (`.gitignore`, so `cargo init`'s own entries are not duplicated); ticking the commit option runs `git add -A` and commits with the message `Add initial commit and projects' initial files`.
- Add a `languages` option to `config.lua` for the new-project menu: a list of tables with a `name`, optional `ignore` lines and `key`, and either `variants` or `run` / `dirs` / `files` directly. They are added after the built-in languages, or replace the one with the same name; an entry that does not fit is skipped and the first problem is logged.
- Add a package page to the new-project menu for Java, Kotlin, Scala, Go (the module path, passed to `go mod init`), C# (the root namespace, written to the project file and the source), Python (the import name, which becomes the folder under `src/`), PHP (the root namespace, mapped to `src/` by PSR-4), Ruby (the library name of `lib/<name>.rb` and its module) Dart (the package name of `pubspec.yaml`, `bin/` and `lib/`), Swift (the PascalCase package and module name), Elixir (the app name given to `mix new`), Haskell (the Cabal package name), Julia (the module name), Nim (the package name of the `.nimble` file and source), Perl (the module name, which becomes `lib/<Module>.pm`), R (the package name of `DESCRIPTION`), Clojure (the main namespace, which becomes its source file and the `:run` alias) and Lua (the module name of `src/<name>.lua`, which `src/main.lua` requires), between the language and the variant: one or more comma-separated package names (suggested `com.example.<project>`), validated, with the first one used by the build file (`groupId`, `mainClass`), the `package` line and the source and test directories of every variant, and the others getting their own folders marked with `.gitkeep`. A language from `config.lua` opts in with `package = "com.example.{pkg}"` and uses `{package}` and `{package_path}` in its steps.
- Add `oxide --check-setups [LANG...]`, a dry run of the new-project menu: every language variant is set up in a temporary folder as the menu would and then checked with its own commands (`cargo build`, `mvn -q -B compile`, `lua src/main.lua` ...; 35 built-in variants have one), reporting `ok`, `setup` (made, checker not installed), `skipped` (a setup program not installed) or `FAILED` with the command's output, and exiting 1 on any failure. It also fails a language that uses a placeholder nothing fills in or whose suggested package its own page refuses. A language in `config.lua` can add a `check` list to its variants.
- Add a `versions` option to `config.lua` for the versions the new-project templates pin (Java release, JUnit, Surefire, Kotlin, Scala, sbt, MUnit, Clojure, the .NET target, minimum PHP, Python, Dart and Nim, the C and C++ standards, minimum CMake): every version in the language table is now a `{v_name}` placeholder with a default in one table (`scaffold::PINS`), a language from `config.lua` can use the same placeholders, and `--check-setups` fails a misspelled one. A wrong key or a non-string version is logged and ignored.
- Add a tags and releases view (`V`, also in the command palette): tags newest first with annotated/lightweight, commit, age, whether the remote has the tag and its GitHub release, loaded in the background; make a tag (`n`, suggesting the next version, a message makes it annotated), push it (`p`), delete it here or also on the remote (`d` / `D`, with confirmation), create a GitHub release or draft release with generated notes (`r` / `R`, refused unless the tag is on the remote, so `gh` never creates it from the default branch), open the release (`o`) and show the tagged commit (`Enter`).
- Add `portable-pty` and `vt100` as dependencies for the embedded terminal.

### Changed
- Update the C# variants to write the `.csproj` and source themselves (target `net8.0`) instead of running `dotnet new`, so the chosen namespace is in the project and the setup needs no installed tool.
- Update the PHP variant to write `composer.json` (with the PSR-4 autoload for the chosen namespace) and `src/Greeter.php` itself instead of running `composer init`, so it needs no installed tool.
- Update the Dart variant to write `pubspec.yaml`, `bin/<name>.dart`, `lib/<name>.dart` and `test/` itself instead of running `dart create`, which takes the package name from the directory and fails for names such as `my-app`.
- Update the R variant to a package (`DESCRIPTION`, `NAMESPACE`, `R/hello.R`) instead of loose scripts, and the Perl variant to put its code in a module under `lib/` that `script/main.pl` loads.
- Update the command palette shortcut from `Ctrl+P` to `Ctrl+K` (`:` still works); inside the palette `Ctrl+N` / `Ctrl+P` still move the selection.
- Update `t` / `T` so that, with the changes list focused, the embedded terminal opens in the folder of the highlighted file (a whole untracked folder opens itself; a file whose folder is gone opens the nearest one left), with that folder in the title. Every folder has its own shell, which keeps running while hidden, and `T` on the same folder returns to it.
- Update `t` to open the embedded terminal instead of suspending Oxide to run `$SHELL` outside it.

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
- Add GitHub integration through the `gh` CLI: `i` and `y` list a repository's issues and pull requests (state filter, labels, branches, draft and review marks), `o` opens an item and `B` the repository in the browser.
- Add reading of GitHub issues and pull requests (`Enter` in the list): description and comments as scrollable text, pull request diffs in the diff viewer (`d`), and `c` to check a pull request's branch out after a confirmation.
- Add GitHub repository search and cloning (`S`): search all of GitHub or list your own repositories, clone the chosen one with `gh` into `clone_dir` (or any folder), and jump to repositories that are already cloned.
- Add the GitHub actions (issues, pull requests, open in the browser, find and clone) to the command palette.
- Add repository deletion from disk (`D`), guarded by a prompt that lists what would be lost and requires typing the repository's name, and by refusals for symlinks, worktrees, submodules and the home directory.
- Add `l` to hide and show the log panel (`logs = off` in the config starts it hidden); while hidden, new log messages appear briefly in the bottom row.
- Add bulk pull (`L`) and bulk push (`U`, asks for confirmation) for the repositories in view.
- Add hunk staging to the diff viewer: `s` stages the hunk at the top of the screen (`git apply --cached`) or, in the staged version, unstages it; the current hunk is marked `▸` and counted in the title, and the viewer reloads and follows the file to its other version when one side runs out of hunks.
- Add file watching through inotify (`src/watch.rs`, no new dependency): every repository's `.git` and the selected repository's working tree are watched, and a repository reloads shortly after the last change made outside Oxide. Status is read with `--no-optional-locks` so reloading never triggers the watcher itself.
- Add `autofetch` config option (`30s`, `10m`, `1h`; off by default) that fetches every repository on a timer, logging only the first failure of a round, with `⟳` and the interval in the header.
- Add CI status on repository rows (`✓` passing, `✗` failing, spinner running, `○` inconclusive) and in the details pane, loaded through `gh run list` after the scan, after auto-fetch and on `w`; `ci = off` turns the automatic refresh off.
- Add `E` to amend the last commit (staged changes are folded in, an empty message keeps the old one) and a reminder when the commit is already pushed.
- Add a reflog view (`O`) with one-key undo (`u`: checkouts go back to the previous branch, other entries move the branch back with `git reset --keep`), `Enter` to move the branch to an entry and `d` to show its commit; every action asks for confirmation.
- Add commit helpers: `Tab` cycles conventional-commit types in the commit and amend prompts, the first line of `commit.template` prefills the prompt, up to 12 lines of hook output are logged, a rejected commit's message is restored on the next try, and an `email` config option (else the global `user.email`) drives an identity warning next to the prompt.
- Add a worktree and submodule view (`W`): list both with their state, add a worktree for a branch next to the repository, remove one (with a force option), prune stale ones, and update one or all submodules; a new worktree joins the repository list and a removed one leaves it.
- Add `config.lua`: the config file is now a Lua script (embedded Lua 5.4 through `mlua`, vendored) that sets globals or returns a table; lists are Lua tables, flags are booleans and `autofetch` is `"10m"` or `false`. The old `~/.config/oxide/config` (`key = value`) is no longer read. A script error or a wrong-typed option is logged and the defaults apply.
- Add language statistics: a `langs` line in the details panel and a focusable Languages pane (`Tab` is now repositories → languages → changes) with each language's share, size, file count and GitHub colour, counted locally by bytes from `git ls-tree` with a built-in extension table (`src/langs.rs`), cached until HEAD moves.
- Update the `langs` line in the details panel to show only the colored (or shaded) bar, without language names or percentages; those stay in the Languages pane.
- Add a remotes view (`r`): list a repository's remotes with fetch/push URLs, add one (a lone URL becomes `origin`), change a URL, rename, remove (with confirmation) and fetch a single remote; the repository row, owner class and GitHub slug update right away.
- Add search prefixes to the GitHub finder (`S`): `user:` finds users and organizations (`Enter` lists their repositories), `repo:` matches repository names only and `file:` finds files by name in any repository; every search is paged 20 results at a time (`←` `→`, `n` `p`, `[` `]`) with the page and match count in the title.

### Fixed
- Fix the key bar losing its last keys on narrow terminals: hints are now dropped from the middle by importance, always keeping `h`, `?` and `q`.
- Fix `e` opening the editor (nvim with its plugins): it now opens yazi in the repository with the highlighted file revealed, so files are opened individually from there; the editor is only used when yazi is not installed.
- Fix leftover status circles on repository rows from the old colored design, leaving two columns of left padding in their place; dirty, conflict and error state remains visible through the row counters and labels.
- Fix log timestamps to use the local timezone instead of UTC, in the log file (with UTC offset) and in the Log panel title.
- Fix scan listing broken repositories: a `.git` that is empty or points to a removed gitdir (dangling submodule) is no longer treated as a repository.

### Changed
- Update the reload key from `r` to `Ctrl+R` / `F5` (the command palette entry too), freeing `r` for remotes; the file watcher already keeps repositories current.
- Update the dashboard with an empty row between the banner and the header (`BANNER_GAP`); the banner now needs one more row to show.
- Update the banner to a plain `src/banner.txt` shown in bold (accent color with colors on) and remove `src/banner.sh` with its ANSI color converter.
- Update the command palette so the first `Esc` clears the typed query and a second one closes it.
- Update the command palette to start empty: the action list only appears once something is typed, and `Enter` does nothing until then.
- Update `h` and `l` (pane switching in the dashboard) to `h` hiding the key bar and `l` doing nothing; use `←` `→` or `Tab` to switch panes. `h` `l` still pan inside the diff viewer.
- Update default scan to cover every mounted drive (internal and external) instead of only the home directory, with a default depth of 6 levels.
- Update panel spacing: one column of inner side padding, one empty column between side-by-side panels and no empty row between stacked ones, which look equal.
- Update design to a monochrome look, banner included: a single ink color on the terminal background, with bold/dim/italic/inverse text and symbols instead of hues, thick focus borders, title notches, a scan progress bar and running-operations indicator, dimmed zero counters, fading log entries, footer key caps, a double-border help window and an inverse-only selected row (no marker).
