# Oxide

A fast terminal dashboard for git, written in Rust with [Ratatui](https://ratatui.rs).
Oxide finds every repository on your machine, shows the state of all of them in one
screen, and lets you stage, commit, push, pull, fetch and merge without leaving it.

```
┌─┤ ◆ OXIDE v0.2.0 ├───────────────────────── 12 repos · 3 dirty · 2 ahead · 1 behind ┐
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
- **Diff viewer with syntax highlighting**: read the staged, unstaged or untracked diff of any changed file with line numbers and per-language colors, one column or side by side, and stage, unstage or discard whole hunks or single lines, without leaving Oxide.
- **Commit log and branches**: browse a repository's history and open any commit's diff; list, switch, create, merge and delete branches, with merged and stale ones flagged.
- **`git init` anywhere**: initialise a repository in any folder, existing or new, and see it in the list right away.
- **GitHub** (needs the [`gh`](https://cli.github.com) CLI, logged in): browse a repository's issues and pull requests, read them, diff and check out pull requests, find repositories (yours or anyone's) and clone them, and open things in your browser.
- **Files and editing**: `Ctrl+P` finds any file of the repository by name (fuzzy, changed files first) and opens it in a read-only viewer with line numbers, highlighting, search and go to line; `e` opens the file in your editor at the cursor line (`Ctrl+E` on the highlighted changed file, `e` in the diff), and Oxide comes back with everything reloaded when the editor exits.
- **Tasks**: `M` lists the build, test, lint and run commands the repository offers (found from `Cargo.toml`, `package.json`, `go.mod`, `Makefile`, `justfile`, Gradle, Maven, CMake, .NET, Python and others, plus your own from `config.lua`), runs the one you pick and streams its output; error locations (`src/main.rs:12:5`, Python tracebacks ...) can be stepped through with `n` and opened in the viewer or your editor.
- **Jump out**: open the selected file or repository in yazi or the file manager.
- **New project**: `K` in an empty repository opens a menu to set it up for 25 languages (Rust, Python, JavaScript, TypeScript, Go, Java, Kotlin, C, C++, C#, PHP, Ruby, Swift, Dart, Lua, Zig, Elixir, Haskell, Scala, R, Julia, Shell, Nim, Perl, Clojure): pick the language (type to filter), the build tool where there is a choice (Maven by default for Java), a license (11 offered, holder and year filled in) and review the exact commands and files before anything is done; the setup then runs in the background, with an optional first commit. *Other / manual…* opens the embedded terminal instead.
- **Embedded terminal**: `t` / `T` opens a shell inside Oxide, in the selected repository, on a pseudo-terminal with colors, full-screen programs and scrollback; `Ctrl+T` hides it and the shell keeps running.
- **Always current**: Oxide watches every repository's `.git` and the selected repository's working tree (Linux inotify, no extra dependency), so commits, checkouts and edits made elsewhere show up on their own; an optional timer fetches every repository.
- **Languages**: what each repository is written in, by share of bytes and in each language's GitHub colour, counted locally from the committed files (no network, no linguist download).
- **CI at a glance**: the latest GitHub Actions run of each repository's branch as `✓` / `✗` / spinner on the row (needs `gh`).
- **Non-blocking**: scans and git commands run on background threads; the UI never freezes.
- **Search**: filter the list live by name, branch or path.
- **Logs**: an in-app log panel plus a persistent log file.
- **Lua config**: `~/.config/oxide/config.lua` is a real Lua script, so the configuration can use variables, lists and logic.
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

  PATH...        directories to scan (default: `root` of config.lua, else every mounted drive)
      --list     print the repository table to stdout and exit (no TUI)
      --check-setups [LANG...]
                 set up every new-project variant in a temporary folder and build it (see below)
  -h, --help     show help
  -V, --version  show the version
```

## Keys

| Key | Action |
| --- | --- |
| `↑` `↓` / `j` `k` | Move selection |
| `g` / `G` | First / last item |
| `Tab` / `Shift+Tab`, `←` `→` | Move the focus between the repository list, the Languages pane and the changes list (see *Languages*) |
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
| `c` | Commit staged changes (prompts for a message; see *Commit helpers* below) |
| `E` | Amend the last commit with the staged changes (empty message keeps the old one) |
| `O` | Reflog of the selected repository: see where HEAD has been and undo (see below) |
| `p` | Pull (fast-forward only) |
| `P` | Push (a branch without upstream is published with `-u`) |
| `F` | Fetch the selected repository (`--all --prune`) |
| `A` | Fetch every repository in the current view |
| `L` | Pull (fast-forward only) every repository in the view that is behind its upstream |
| `U` | Push every repository in the view that is ahead of its upstream (asks `y` to confirm) |
| `z` / `Z` | Stash all changes including untracked files / pop the latest stash |
| `v` | Commit log of the selected repository (see below) |
| `b` | Branches of the selected repository (see below) |
| `W` | Worktrees and submodules of the selected repository (see below) |
| `V` | Tags and releases of the selected repository (see below) |
| `N` | GitHub workflows of the selected repository: start one by hand, or write a new one (see below) |
| `I` | `git init` in a folder: the prompt starts with the current directory, append a name for a new folder |
| `i` / `y` | GitHub issues / pull requests of the selected repository (see below) |
| `B` | Open the selected repository's GitHub page in the browser |
| `S` | Find GitHub repositories and clone them (see below) |
| `Ctrl+P` | Find a file of the selected repository by name and read it (see *File finder and viewer* below) |
| `Ctrl+E` | Edit the highlighted changed file in your editor, at its first line (`editor` in `config.lua`, else `$VISUAL` / `$EDITOR`, else `vi`) |
| `e` | Open [yazi](https://yazi-rs.github.io) in the repository, with the highlighted file revealed when the changes list is focused. Open files from yazi with your usual opener (e.g. nvim). Without yazi installed it falls back to `$VISUAL` / `$EDITOR` |
| `M` | Tasks of the selected repository: run a build, test, lint or any other command and watch its output (see *Tasks* below) |
| `K` | New project in an empty repository: language, build tool, license, then a plan to confirm (`c` toggles a first commit, `Esc` goes back a page) |
| `X` | Undo the last project setup of the selected repository (see "New project") |
| `t` / `T` | Open the embedded terminal (`$SHELL`) in the repository, or, with the changes list focused, in the folder of the highlighted file (each folder has its own shell, named in the title); `Ctrl+T` hides it, `Shift+PgUp` / `Shift+PgDn` scroll its history, `exit` closes it |
| `o` | Open the repository folder in the file manager (`xdg-open`) |
| `m` | Merge a branch into the current one (empty input merges the upstream) |
| `r` | Remotes of the selected repository: add, change URL, rename, remove, fetch (see below) |
| `R` | Rescan all roots |
| `Ctrl+R`, `F5` | Reload the selected repository by hand (changes are also picked up on their own) |
| `w` | Refresh the CI status of the repositories in view (see below) |
| `:` / `Ctrl+K` | Command palette: starts empty, type a few letters of any action (GitHub ones included: `clone`, `issues`, `browser` ...) (fuzzy matched), `↑` `↓` pick, `Enter` runs it, `Esc` clears the query and a second `Esc` closes it |
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
lists your own repositories, private ones included. A prefix narrows what is searched:

| Search | Finds |
| --- | --- |
| `user:torvalds` | Users and organizations |
| `repo:oxide` | Repositories whose **name** matches |
| `file:Cargo.toml ratatui` | Files with that name in any repository; further words must appear inside the file |

Results come 20 per page so the list stays fast (GitHub reaches at most the first 1000 matches). The title
shows the page and the number of matches. Repository rows show the stars, language, age of the last
push, `private` / `fork` tags and the description; file rows show the repository and the path. Repositories
you already have locally are marked `cloned`.

| Key | Action |
| --- | --- |
| `Enter` | On a repository or file: clone the repository (the prompt starts with `clone_dir/<name>`, edit it freely; relative paths start from `clone_dir`) and add it to the list. On one you already have, jump to the local copy instead. On a user: list their repositories, newest work first |
| `→` `n` `]` / `←` `p` `[` | Next / previous page |
| `/` or `S` | New search |
| `o` | Open the highlighted user, repository or file in the browser |
| `r` | Reload the page |
| `Esc`, `q` | Close |

The clone uses the protocol configured in `gh` (ssh or https). Like `git init`, a clone outside the scan
roots disappears on the next full rescan (`R`); `clone_dir` is the best place to also list as a `root`.

### Commit helpers

In the commit (`c`) and amend (`E`) prompts:

- `Tab` / `Shift+Tab` put the next / previous conventional-commit type in front of the message (`feat`, `fix`, `docs`,
  `style`, `refactor`, `perf`, `test`, `build`, `ci`, `chore`, `revert`), replacing the one already there and keeping a
  `(scope)` or `!`.
- If the repository has a `commit.template`, its first line that is not empty or a `#` comment starts the prompt.
- Pre-commit and commit-msg hook output is shown in the log (up to 12 lines). When a hook rejects the commit, the
  message you typed is restored in the next commit prompt instead of being lost.
- A warning appears next to the prompt when the repository would commit with an e-mail other than the expected one:
  the `email` option of `config.lua`, or your global `user.email` without any. It also warns when no `user.email` is set.

Amending a commit that is already pushed logs a reminder that publishing it needs a force push.

### Reflog and undo

`O` lists where HEAD has been (`git reflog`), newest first, with what moved it: commits, checkouts, merges,
pulls, resets, rebases. Every action asks `y` first.

| Key | Action |
| --- | --- |
| `u` | Undo the highlighted entry: a checkout goes back to the branch it came from, anything else moves the branch back to where it was before (`git reset --keep`, so uncommitted changes survive and git refuses if they would be lost) |
| `Enter` | Move the current branch to the highlighted entry's commit (`git reset --keep`) |
| `d` | Show the commit in the diff viewer |
| `↑` `↓` / `j` `k`, `g` / `G`, `Space` / `b` | Move |
| `Esc`, `q`, `O` | Close |

With the first entry highlighted, `u` is a one-key undo of the last thing that happened.

### Languages

The details panel shows a `langs` line for the selected repository: a stacked bar with one segment per
language in its colour (or shade), without names, so it stays readable at a glance. `Tab` (or `→`) moves the focus
to the Languages pane, which then fills the panel (the Changes list shrinks to a few rows) with one row per
language: swatch, name, a bar, the share, the size and the number of files. `↑` `↓` / `g` `G` scroll; `Esc`
goes back to the list.

How it is counted, all locally and without checking anything out:

- One `git ls-tree -r -l HEAD` gives the size of every committed file; shares are by **bytes**, like GitHub.
  Uncommitted files and ignored files do not count. The result is cached per repository and recomputed
  when HEAD moves; only the selected repository is counted.
- A built-in table (`src/langs.rs`, about 100 languages) maps file extensions and a few file names
  (`Makefile`, `Dockerfile`, `CMakeLists.txt` ...) to a language and its GitHub colour. Edit the table to
  add a language or fix a colour.
- Like GitHub, only programming and markup languages count: prose (Markdown, text) and data formats (JSON,
  YAML, TOML, XML) are left out. `vendor`, `node_modules`, `third_party` and similar folders and generated or
  minified files (`*.min.js`, `*.pb.go` ...) are skipped. `.h` headers count as C++ when the repository has
  C++ sources, else as C.
- Colours are true colour when `COLORTERM` says the terminal supports it, else the nearest of the 256
  colours. With colors off (`C`) the languages are told apart by fill shade (`█ ▓ ▒ ░`) instead.

### Live updates and CI

**File watching.** Oxide watches the `.git` directory of every repository (index, HEAD, fetch and branch
refs) and, recursively, the working tree of the *selected* repository, using Linux inotify directly. A
change made by an editor or another terminal reloads the affected repository about 0.4 s after the last
write; operations started from Oxide reload it anyway. Only one working tree is watched at a time to stay
below the kernel's watch limit (`fs.inotify.max_user_watches`); `node_modules`, `target`, `venv` and other
cache folders are skipped. If the watches cannot be set up the log says so and `Ctrl+R` still reloads by hand.

**Auto-fetch.** With `autofetch = "10m"` in `config.lua` every listed repository that has a remote is fetched
(`--all --prune`) on that interval, silently: only the first failure of a round is logged. The header shows `⟳ 10m`
while it is on. It is off by default.

**CI status.** After the scan, after every auto-fetch round and on `w`, Oxide asks GitHub (through `gh run
list`) for the newest workflow run of each branch in view that lives on github.com (at most 100 repositories
per round). The row shows `✓` passing, `✗` failing, a spinner while queued or running and `○` for cancelled or
skipped runs, and the details pane names the state. If `gh` is missing or not logged in, the log says so once
and automatic refreshes stop until you press `w`; `ci = false` in `config.lua` disables the automatic ones.

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

### Remotes

`r` lists the selected repository's remotes with their URLs (a separate push URL is shown after `push →`, and
`github` marks github.com ones). The dashboard row, the owned/cloned class and the GitHub features follow the
change at once, since they read the `origin` URL.

| Key | Action |
| --- | --- |
| `a` | Add a remote: type `<name> <url>`. A lone URL is named `origin` when the repository has no remote yet |
| `e` / `u` | Change the highlighted remote's URL (the prompt starts with the current one) |
| `n` / `F2` | Rename it |
| `d` | Remove it after `y` (its remote-tracking branches are deleted with it) |
| `Enter` / `f` | Fetch just that remote |
| `↑` `↓` / `j` `k`, `g` / `G` | Move |
| `Esc`, `q`, `r` | Close |

Names and URLs that begin with `-` are refused so git can never read them as options.

### Worktrees and submodules

`W` lists the repository's worktrees (the main one first, then linked ones with their branch and any
`locked` / `missing` flag) and its submodules (with `git describe` and `not initialised`, `other commit` or `conflicts`
when they are not in sync). It works from a linked worktree too: the commands then run from the main folder.

| Key | Action |
| --- | --- |
| `n` | New worktree: asks for a branch; the folder is created next to the main one as `<repo>-<branch>` (`/` becomes `-`), the branch is made from HEAD when it does not exist. It joins the repository list at once |
| `Enter` | Select the highlighted worktree on the dashboard (it must be in the list; a worktree outside the scan roots needs `R` or an extra `root`) |
| `d` / `D` | Remove the linked worktree after `y`; `D` also discards its uncommitted changes. The main worktree and locked ones are refused |
| `p` | `git worktree prune`: forget worktrees whose folder is gone |
| `u` / `U` | `git submodule update --init --recursive` for the highlighted submodule / for all of them (asks `y`) |
| `↑` `↓` / `j` `k`, `g` / `G`, `Space` / `b` | Move |
| `Esc`, `q`, `W` | Close |

Linked worktrees are repositories like any other and appear in the list; submodules do not, since nested
repositories are not scanned.

### GitHub workflows

`N` lists the files of `.github/workflows` (read locally, so it works offline): the workflow's name, its file,
`manual` when it has a `workflow_dispatch` trigger (so it can be started by hand) or `auto`, and, from `gh`, how its
latest run went (`✓`, `✗`, `○`, or a spinner while it runs) with its age and branch. Without any workflow the view
says so and offers to create one.

| Key | Action |
| --- | --- |
| `Enter` / `r` | Start the highlighted workflow by hand (`gh workflow run`) |
| `n` | Write a new workflow with the wizard (below) |
| `o` | Open it on GitHub |
| `e` | Edit the file (yazi, or your editor) |
| `f` | Read the files and ask GitHub for the latest runs again |

**Starting a workflow.** The form has the branch or tag to run (the repository's current branch to begin with)
and one line per input the workflow declares, read from its `workflow_dispatch: inputs:`: text is typed,
`type: boolean` is a checkbox (`Space`), `type: choice` cycles its options (`←` `→`), defaults are filled in and
`required` inputs are marked `*`. `Enter` asks for a `y` first, since a run uses Actions minutes. A workflow without
`workflow_dispatch` cannot be started this way, and the form warns when the file is new or changed here, because
GitHub runs the version that is pushed.

**Writing a workflow.** `n` opens a wizard; `Esc` goes back a page and nothing is written until the last one.

1. **What for**: *CI* (build and test on push and pull request), *Release* (build on a `v*` tag and publish a
   GitHub release with the files), *Scheduled* (build and test on a cron time) or *Manual task* (run a command you
   type when you start it).
2. **Language**: Rust, Node.js (npm, yarn or pnpm by lock file), Python, Go, Java (Maven), Java / Kotlin (Gradle,
   with or without the wrapper), C# (.NET), Ruby, PHP, Elixir, Zig, C / C++ (CMake or Make), Swift, Dart, or
   commands you type. The one the repository looks like it uses is marked.
3. **When it runs**: boxes for push, pull request, version tag, schedule and run by hand.
4. **Options**: boxes for Linux, macOS and Windows (several make a matrix), dependency caching, keeping the build
   output as an artifact, cancelling obsolete runs and, for a release, publishing it.
5. **Details**: the name, the file, the branches, the cron time and the command, where they apply.
6. **Preview**: the finished file with line numbers. `Enter` writes it, and you run it later from `N`. `c` writes it,
   commits only that file, pushes the branch and, when it can be started by hand and the remote is on GitHub,
   starts it for the first time (GitHub needs a few seconds to know a new workflow, so starting is retried).

The generated file asks for read-only `contents` permission (the release job alone gets write), and a manual
task passes the command to the shell through an environment variable instead of pasting it into the script. The
version of every action (`actions/checkout@v4`, `actions/setup-node@v4` ...) and of the toolchains they set up
(Node 22, Python 3.12 ...) can be changed with `versions` in `config.lua` (`gha_*` and `ci_*` keys, listed there),
so a stale one is a one-line fix; Java and .NET follow the `java` and `dotnet` entries.

### Tags and releases

`V` lists the repository's tags, newest first. Each row shows whether the tag is annotated (`◆`) or lightweight
(`·`), the commit, its age, whether the remote has it (`pushed`, `local`, or `…` / `?` while the server is being
asked or cannot be reached), its GitHub release (`release`, `latest`, `pre` or `draft`) and its message. The remote
and the releases load in the background, so the list is there at once.

| Key | Action |
| --- | --- |
| `n` | New tag on `HEAD`. The prompt starts with the next version (`v0.2.0` suggests `v0.2.1`); type a name, optionally followed by a message, which makes it an annotated tag |
| `p` | Push the highlighted tag to the remote (`origin`, else the first) |
| `d` / `D` | Delete the tag here / here and on the remote, after a confirmation |
| `r` / `R` | Create a GitHub release / draft release for the tag with generated notes (needs `gh`, a `github.com` remote and the tag already pushed; a confirmation first) |
| `o` | Open the tag's release in the browser |
| `Enter` | Show the commit the tag points at in the diff viewer |
| `f` | Ask the remote and GitHub again |

### git init

`I` asks for a folder. A relative path starts from the directory Oxide was launched in and `~` is expanded;
a folder that does not exist is created. The new repository appears in the list and is selected. A repository
outside the scan roots disappears on the next full rescan (`R`), so add its folder as a `root` to keep it.

### New project

`K` on an empty repository (nothing in it but `.git`) opens a menu with up to five pages; `Esc` goes back one page.

1. **Language**: 25 languages, narrowed by typing. *Other / manual…* closes the menu and opens the embedded
   terminal in the repository, for anything the menu does not cover.
2. **Package** (called **module** for Go and **namespace** for C#): for every language that has packages, modules or namespaces, which is all of
   them except Rust, JavaScript, TypeScript, C, C++, Zig and Shell. For Go it is the module path of `go.mod` (suggested `github.com/<you>/<project>`, used by `go mod init`);
   for C# the root namespace (suggested `<Project>` in PascalCase), written to the `.csproj` and the source; for
   Python the name the code is imported by (suggested `my_project`), which becomes `src/<name>/` (and the project
   name with `uv`); for PHP the root namespace with backslashes (suggested `<You>\<Project>`), which is mapped to
   `src/` by PSR-4 in `composer.json`; for Ruby and Dart the library name in `lowercase_snake_case` (suggested
   `my_project`), which names `lib/<name>.rb` and its `module MyProject` for Ruby, and the `name` in
   `pubspec.yaml` with `bin/<name>.dart` and `lib/<name>.dart` for Dart; for Swift a PascalCase name (suggested
   `MyProject`) passed to `swift package init --name`; for Elixir a snake_case app name (`mix new . --app`); for
   Haskell a Cabal package name of lowercase words joined by hyphens (suggested `my-project`, passed to
   `cabal init --package-name`); for Lua a snake_case module name (`src/<name>.lua`, loaded by `src/main.lua`); for Julia a PascalCase module name (`Project.toml`, `src/<Name>.jl`); for Nim a
   snake_case name (`<name>.nimble`, `src/<name>.nim`); for Perl a module name with `::` (suggested `MyProject`),
   which becomes `lib/My/Project.pm`; for R a package name of letters, digits and dots (suggested `my.project`),
   used in `DESCRIPTION`; for Clojure the main namespace (suggested `my-project.core`), which becomes
   `src/my_project/core.clj` and the `:run` alias; for the JVM languages, type one or more package names separated by commas (the
   suggestion is `com.example.<project>`). The first holds the main code; the build tool page and the source
   directories (`src/main/java/org/acme/app`, plus `src/test/...`) use it, and every further package gets its
   own empty folders. Names are checked: dot-separated parts of letters, digits and `_`, none starting with a digit.
3. **Variant**: only for languages with a real choice, such as Maven, Gradle (Groovy) or Gradle (Kotlin DSL)
   for Java (Maven is the default), `venv` or `uv` for Python, `npm`, `pnpm` or Bun for JavaScript.
4. **License**: MIT, Apache-2.0, GPL-3.0, AGPL-3.0, LGPL-3.0, BSD-2-Clause, BSD-3-Clause, MPL-2.0, ISC,
   Unlicense, CC0-1.0 or none. `Tab` edits the copyright holder (your global `user.name` by default).
5. **Confirm**: every command and file, in order, and the programs that are not installed (their commands are
   skipped). `c` ticks *Add initial commit and projects' initial files*, which stays off unless you tick it.

`Enter` on the last page does the work on a background thread, with one log line per step. **`X` undoes it** (see below). The project name is
the directory's name. Besides what the language needs (`cargo init`, `pom.xml`, `src/`, `go mod init` ...) it
writes a `.gitignore` for the language, merged into any the tools made, a `README.md` unless a tool made one,
and the `LICENSE`. A directory that would stay empty gets a `.gitkeep`, since Git does not keep empty ones.
If a step fails the rest is not run and the log says which one; the files made so far stay.

Add languages, or replace a built-in one of the same name, with `languages` in `config.lua` (see below).

**Undoing a setup.** When a setup finishes, Oxide records what it made in `oxide-setup` inside the git directory:
the top-level entries of the folder (it was empty before) and the first commit, if you asked for one. `X` reads
that record, asks with the exact entries, the number of files and the changes made since, and after a `y`
removes them, drops the setup commit and leaves the repository empty again (`K` works again). The commit is not
lost at once: its hash is logged and `git branch rescue <hash>` brings it back until git cleans up. It refuses
when commits were made after the setup, when the files are already gone, and when nothing was recorded, and it
never touches anything that was not in the folder when the setup ended. A setup that failed halfway can be
undone the same way.

**Checking the setups.** `oxide --check-setups` (optionally followed by words that language names must contain,
like `oxide --check-setups java rust`) sets up every variant of the menu in a temporary folder, exactly as the
menu would, and then runs the variant's `check` commands (`cargo build`, `mvn -q -B compile`, `lua src/main.lua`,
...). Each variant is `ok`, `setup` (made, but the program that checks it is missing), `skipped` (a program the
setup runs is missing) or `FAILED` with the command and the end of its output; a language whose table has a
placeholder that nothing fills in, or whose suggested package its own page would refuse, fails too. The exit
status is 1 when anything failed, and the folders are then kept for a look. Some checks download dependencies,
so they need the network. A language from `config.lua` can add a `check` list to its variants to take part.

### File finder and viewer

`Ctrl+P` lists the files of the selected repository (tracked ones and untracked ones that are not ignored),
changed files first. Type a few letters of the path: matches inside the file name rank above matches spread
over folders, and `●` marks files with changes. `Enter` opens the file in the viewer, `Ctrl+E` in the editor,
`Esc` clears the query and then closes the finder.

The viewer is read-only: line numbers, the diff viewer's per-language highlighting, a cursor line (`▸`) and
files up to 2 MB or 50 000 lines (it says when it cuts one off; binary files are refused).

| Key | Action |
| --- | --- |
| `↑` `↓` / `j` `k`, `Space` / `b` | Move the cursor a line / a page |
| `g` / `G` | Top / bottom |
| `←` `→` / `h` `l`, `0` | Pan long lines, back to the left edge |
| `/` | Search (case does not matter); lines that hold it are marked `•`, and the title counts them |
| `n` / `N` | Next / previous match, wrapping around |
| `:` | Go to a line number |
| `e` | Open the file in the editor at the cursor line |
| `r` | Read the file again |
| `Esc`, `q` | Close (back to the finder when it came from there) |

The editor runs in the whole terminal, and Oxide returns when it exits. It is `editor` in `config.lua` if
set, else `$VISUAL`, else `$EDITOR`, else `vi`. The line is passed the way the editor expects it: `+12 file`
for vi, vim, nvim, nano, micro, emacs, kak and gedit; `file:12` for helix, zed and sublime; `-g file:12` for
VS Code and Cursor. For any other editor use `{file}` and `{line}` in `editor`, for example
`editor = "kate -l {line} {file}"`. A graphical editor that returns at once is fine; `r` re-reads the file.

### Tasks

`M` lists what the selected repository can run. Oxide finds the commands from the project files it knows:

| File | Tasks |
| --- | --- |
| `Cargo.toml` | `build`, `test`, `check`, `lint` (clippy), `fmt`, `run` |
| `package.json`, `composer.json` | every script, run with pnpm, yarn, bun or npm by the lock file (composer: `composer run-script`) |
| `go.mod` | `build`, `test`, `vet`, `run` |
| `pom.xml`, `build.gradle(.kts)`, `build.sbt` | build, test, package / check, run (`./gradlew` when the wrapper exists) |
| `CMakeLists.txt`, `build.zig`, `Package.swift`, `mix.exs`, `pubspec.yaml`, `project.clj`, `stack.yaml`, `Rakefile` | the usual build, test and run commands of each |
| `*.sln`, `*.csproj`, `*.fsproj` | `dotnet build`, `test`, `run` |
| `pyproject.toml`, `setup.py`, `pytest.ini`, `tox.ini` | `test` (`python -m pytest`), and `lint` (`ruff check .`) when ruff is configured |
| `Makefile`, `justfile` | every target / recipe |

Names that could not be typed safely into a shell (a script called `x; rm -rf ~`) are left out. Add your own, for
every repository or for one, with `tasks` in `config.lua`; a task with the name of a detected one replaces it:

```lua
tasks = {
  { name = "deploy", run = "./scripts/deploy.sh --dry-run" },              -- every repository
  { name = "serve",  run = "python -m http.server 8000", repo = "site" },  -- only the one called "site"
}
```

`Enter` runs the highlighted task in the repository and opens its output: standard output and error together,
in order, as they arrive, with colors and progress rewrites reduced to the text. The menu starts on the task
that ran last. One task runs at a time; closing Oxide stops it, and the repository is reloaded when it ends.
The result is also written to the log (`test: passed in 3.2 s`, `build: failed (exit 101) in 1.1 s`).

| Key | Action |
| --- | --- |
| `↑` `↓` / `j` `k`, `Space` / `b`, `g` / `G` | Scroll; `G` follows the output again |
| `← →` / `h` `l`, `0` | Pan long lines |
| `n` / `N` | Select the next / previous line that points at a place in a file (`›` marks them, `▸` the selected one) |
| `Enter` | Open that place in the code viewer (`Esc` comes back here) |
| `e` | Open that place in your editor |
| `c` | Stop the task (it gets `SIGTERM`, and `SIGKILL` after three seconds) |
| `r` | Run it again, when it has ended |
| `Esc`, `q` | Close; while the task still runs it only hides the output (`M`, then `o`, shows it again) |

Places are found in compiler and test output (`src/main.rs:12:5`, `--> src/main.rs:12:5`, `file.ts(12,5)`,
Python's `File "x.py", line 12`, Maven's `File.java:[12,5]`), only when the file exists inside the repository.
Lines that read as errors are red and warnings yellow.

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
| `v` | Pick single lines of that hunk (see below) |
| `x` | Discard the hunk at the top of the screen from the working tree (unstaged version; asks `y`) |
| `m` | Side by side / one column |
| `e` | Edit the file in your editor, at the first line shown (working-tree diffs only) |
| `Esc`, `q`, `d` | Close |

**Picking lines** (`v`): the cursor `▸` moves over the added and removed lines of the hunk at the top of the
screen. `Space` picks a line (shown as `[+]` / `[-]`) and moves on, `a` picks every line of the hunk or none,
`s` or `Enter` stages the picked lines (unstages them in the staged version), `x` throws them away from the
working tree (unstaged version, after a `y`), and `Esc` goes back. With nothing picked, `s` and `x` act on the
line under the cursor. The lines you leave alone stay as they are, so a changed line can be staged as only
its removal or only its addition. Discarding cannot be undone.

**Side by side** (`m`): the old file on the left and the new one on the right, each run of removed lines next
to the added lines that replaced it. `[` `]`, paging and panning work as in one column; picking lines
switches back to one column.

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
Press `C` to toggle colors, set `color = false` in `config.lua`, or export `NO_COLOR`. The selected row
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
`github.user`, plus any `owner` entries in `config.lua`.

## Configuration

Optional file: `~/.config/oxide/config.lua` (or `$XDG_CONFIG_HOME/oxide/config.lua`). It is a plain Lua script
(Lua 5.4, built into Oxide): set the options below as global variables, or `return` a table with them. Strings
take `~`, lists are Lua tables, and anything else Lua can do (`os.getenv`, conditionals ...) works too. If the
file fails to run or an option has the wrong type, Oxide ignores it, uses the defaults and says why in the log
(and on stderr with `--list`).

```lua
-- directories to scan; without any root, every mounted drive is scanned
root = { "~/dev", "/mnt/backup" }

-- extra account names that count as "owned" (a string or a list)
owner = { "my-company" }

-- how many directory levels below each root are searched
depth = 6

-- accent colors on top of the monochrome design (false by default when NO_COLOR is set)
color = true

-- start with the key bar at the bottom visible; the h key toggles it
hints = true

-- start with the log panel visible; the l key toggles it
logs = true

-- e-mail addresses commits should be made with; a warning shows in the commit prompt for any other.
-- Without entries the global user.email is the expected one.
email = "me@example.com"

-- fetch every repository on this interval: "30s", "10m", "1h" (a bare number is minutes); false = off (default)
autofetch = "10m"

-- load the CI status of GitHub repositories in the background (w always refreshes it)
ci = true

-- folder that GitHub repositories are cloned into by default (home directory when unset)
clone_dir = "~/dev"

-- commands for the task menu (M): a name and a command, for every repository or only for one (a directory name,
-- or a path when it holds a "/" or starts with "~"); a name that a project file also offers replaces it
tasks = { { name = "deploy", run = "./scripts/deploy.sh" }, { name = "serve", run = "python -m http.server", repo = "site" } }

-- the editor `e` and `Ctrl+E` start, with its arguments; {file} and {line} are replaced if you write them.
-- Default: $VISUAL, then $EDITOR, then vi
editor = "nvim"

-- repositories listed first / hidden: a directory name, or a path (a value with a "/" or a leading "~"
-- also covers every repository below it)
pin = { "oxide" }
ignore = { "~/dev/vendor" }

-- versions the new-project templates and the generated workflows pin; a string (or a whole number) replaces the
-- default, with no spaces, quotes or braces. Unset ones keep it.
-- java 21, junit 5.11.4, surefire 3.5.2, kotlin 2.1.0, scala 3.3.4, sbt 1.10.7, munit 1.0.3, clojure 1.12.0,
-- dotnet net8.0, php 8.1, python 3.10, dart 3.0.0, nim 2.0.0, c_std 11, cpp_std 20, cmake 3.16
-- (java also sets the Kotlin JVM target; php, python, dart and nim are minimum versions)
-- generated workflows: the version of each action, as what follows the @ (gha_checkout v4, gha_cache v4,
-- gha_upload v4, gha_download v4, gha_release v2, gha_node v4, gha_pnpm v4, gha_python v5, gha_go v5, gha_java v4,
-- gha_gradle v4, gha_dotnet v4, gha_ruby v1, gha_php v2, gha_beam v1, gha_zig v1, gha_swift v2, gha_dart v1,
-- gha_rust_cache v2, gha_rust_toolchain stable) and the toolchains they set up (ci_node 22, ci_python 3.12,
-- ci_ruby 3.3, ci_php 8.3, ci_elixir 1.17, ci_otp 27, ci_gradle current); java and dotnet apply there too
versions = { java = "17", kotlin = "2.0.21", dotnet = "net9.0", gha_checkout = "v5", ci_node = "24" }

-- languages for the new-project menu (K), added to the built-in ones. A language has a name, optional
-- .gitignore lines (ignore) and either variants or, for a single setup, run / dirs / files directly.
-- A language that keeps its code in packages sets package (a suggestion); the menu then asks for the names and
-- steps can use {package} (org.acme.app) and {package_path} (org/acme/app). Names are checked like Java's; use `{package}` as it is typed.
-- A variant is done in this order: run (command lines, no shell), dirs, files (path = contents). Its optional
-- check (a command line or a list) is only used by oxide --check-setups, run inside the finished project.
-- Command lines and contents may use {name} (the directory name), {slug} (my-app), {pkg} (my_app),
-- {user} (your git user name) and a pinned version as {v_java}, {v_junit} ... (the names of `versions`). A built-in language is replaced by one with the same name.
languages = {
  {
    name = "OCaml",
    ignore = { "_build/", "*.install" },
    variants = {
      { name = "dune", note = "dune init", run = { "dune init project {slug} ." } },
      { name = "plain", note = "bin/ and test/", dirs = { "test" },
        files = { ["bin/main.ml"] = 'let () = print_endline "hello"\n' } },
    },
  },
}
```

The `*` and `x` keys store their marks in `~/.local/state/oxide/marks` (or `$XDG_STATE_HOME/oxide/marks`),
so `config.lua` is never rewritten. A mark that comes from `config.lua` can only be removed there.

## What gets scanned

By default Oxide reads `/proc/mounts` and scans each mounted block-device filesystem
(`/`, `/home`, external drives under `/mnt`, `/media` or `/run/media`, ...). Each drive is
its own root and a scan never crosses onto another filesystem, so nothing is visited twice.
Skipped: `/boot`, snap and squashfs images, network mounts, and the system directories
`/proc`, `/sys`, `/dev`, `/run`, `/usr` (only directly under `/`).

`root` in `config.lua`, or paths on the command line, replace this automatic
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
| `src/tasks.rs` | Finds a repository's tasks from its project files, runs one in the background and reads its output (places, errors) |
| `src/code.rs` | Reads and highlights a source file for the code viewer |
| `src/diff.rs` | Parses `git diff` output into rows for the diff viewer |
| `src/syntax.rs` | Dependency-free syntax highlighter used by the diff viewer |
| `src/langs.rs` | Built-in language table (extensions, colours) and the per-repository language count |
| `src/term.rs` | Embedded terminal: a shell on a pseudo-terminal, parsed by `vt100`, and the encoding of keys for it |
| `src/scaffold.rs` | New-project menu data: languages and variants, the plan built from a choice, and running it |
| `src/check.rs` | `--check-setups`: sets up and builds every variant of the new-project menu in a temporary folder |
| `src/workflow.rs` | Reads workflow files (name, `workflow_dispatch` inputs) and compiles the wizard's choices into one |
| `src/ignore.rs` | `.gitignore` templates per language family |
| `src/license.rs` | License list and the text of each (`src/licenses/` holds the long ones) |
| `src/watch.rs` | inotify file watching of `.git` directories and the selected working tree |
| `src/repo.rs` | Repository model, discovery, owned/cloned classification |
| `src/worker.rs` | Background scan and operation threads |
| `src/config.rs` | `config.lua` (run with embedded Lua), roots and owner names |
| `src/palette.rs` | Command palette entries and fuzzy matcher |
| `src/marks.rs` | Pinned and ignored repositories (`config.lua` and state file) |
| `src/logger.rs` | Persistent log file |
| `src/banner.rs` | Turns `src/banner.txt` into Ratatui lines |
| `src/banner.txt` | The ASCII banner art shown on top of the dashboard |

## Versioning

Oxide follows a weekly release cadence. Changes are recorded in [CHANGELOG.md](CHANGELOG.md).

## License

[MIT](LICENSE)
