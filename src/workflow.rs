//! GitHub Actions workflows: reads the files of `.github/workflows` for what the workflows view needs (the
//! name, whether the workflow can be started by hand and the inputs it asks for) and, further down, builds new
//! workflow files from the choices of the creation wizard.
//!
//! Reading is a line scanner over the few keys that matter, not a YAML parser: it understands the usual layouts
//! (`on: workflow_dispatch`, `on: [push, workflow_dispatch]`, and the nested block with `inputs:`).

use std::path::Path;

/// How the run form asks for an input.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum InputKind {
    Text,
    /// `type: boolean`, a checkbox.
    Bool,
    /// `type: choice` (or `environment`), one of `options`.
    Choice,
}

/// One input of a `workflow_dispatch` trigger.
#[derive(Clone, Debug)]
pub struct Input {
    pub name: String,
    pub description: String,
    pub required: bool,
    pub default: String,
    pub kind: InputKind,
    pub options: Vec<String>,
}

/// A workflow file of the repository.
#[derive(Clone, Debug)]
pub struct Workflow {
    /// File name inside `.github/workflows`, e.g. `ci.yml`.
    pub file: String,
    /// Its `name:`, or the file name without the extension when it has none.
    pub name: String,
    /// It has a `workflow_dispatch` trigger, so it can be started by hand.
    pub dispatch: bool,
    pub inputs: Vec<Input>,
}

/// The folder of the workflow files, relative to the repository.
pub const DIR: &str = ".github/workflows";

/// The workflows of the repository at `repo`, by file name; empty when it has none.
pub fn read_all(repo: &Path) -> Vec<Workflow> {
    let mut found: Vec<Workflow> = std::fs::read_dir(repo.join(DIR))
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| {
            let file = e.file_name().to_string_lossy().into_owned();
            if !(file.ends_with(".yml") || file.ends_with(".yaml")) {
                return None;
            }
            let text = std::fs::read_to_string(e.path()).ok()?;
            Some(parse(&file, &text))
        })
        .collect();
    found.sort_by(|a, b| a.file.cmp(&b.file));
    found
}

/// Indentation (spaces) of a line.
fn indent(line: &str) -> usize {
    line.len() - line.trim_start().len()
}

/// A scalar value from the text after `key:`: no comment, no surrounding quotes.
fn scalar(raw: &str) -> String {
    let mut text = raw.trim();
    // A comment starts at ` #` outside quotes; the values read here are simple, so the first one counts.
    if !text.starts_with(['"', '\'']) {
        if let Some(i) = text.find(" #") {
            text = text[..i].trim_end();
        }
    }
    let unquoted = text
        .strip_prefix('"')
        .and_then(|t| t.strip_suffix('"'))
        .or_else(|| text.strip_prefix('\'').and_then(|t| t.strip_suffix('\'')))
        .unwrap_or(text);
    unquoted.to_string()
}

/// `key: value` in a line, with the key unquoted.
fn pair(line: &str) -> Option<(String, String)> {
    let (key, value) = line.trim().split_once(':')?;
    // `a: b` and `a:` are pairs; `http://x` is not (no space after the colon).
    if !value.is_empty() && !value.starts_with(' ') {
        return None;
    }
    Some((scalar(key), scalar(value)))
}

/// Reads one workflow file.
pub fn parse(file: &str, text: &str) -> Workflow {
    let lines: Vec<&str> = text.lines().collect();
    let stem = file.rsplit_once('.').map_or(file, |(s, _)| s);
    let name = lines
        .iter()
        .find(|l| l.starts_with("name:"))
        .map(|l| scalar(&l["name:".len()..]))
        .filter(|n| !n.is_empty())
        .unwrap_or_else(|| stem.to_string());

    // Only the part before `jobs:` holds triggers; a later `if: github.event_name == 'workflow_dispatch'` does not count.
    let head_end = lines.iter().position(|l| l.starts_with("jobs:")).unwrap_or(lines.len());
    let head = &lines[..head_end];
    let live = |l: &str| !l.trim_start().starts_with('#');
    let dispatch = head.iter().any(|l| live(l) && l.contains("workflow_dispatch"));

    let mut inputs = Vec::new();
    if let Some(at) = head.iter().position(|l| live(l) && l.trim_start().starts_with("workflow_dispatch:") && l.trim() == "workflow_dispatch:") {
        let base = indent(head[at]);
        // The lines that belong to `workflow_dispatch:`, up to the next line that is not indented below it.
        let block: Vec<&str> = head[at + 1..].iter().copied().take_while(|l| l.trim().is_empty() || indent(l) > base).collect();
        if let Some(i) = block.iter().position(|l| l.trim() == "inputs:") {
            let inputs_indent = indent(block[i]);
            let rest: Vec<&str> = block[i + 1..].iter().copied().take_while(|l| l.trim().is_empty() || indent(l) > inputs_indent).collect();
            let rest: Vec<&str> = rest.into_iter().filter(|l| !l.trim().is_empty() && !l.trim_start().starts_with('#')).collect();
            let Some(child) = rest.first().map(|l| indent(l)) else { return Workflow { file: file.into(), name, dispatch, inputs } };
            let mut k = 0;
            while k < rest.len() {
                // An input starts at the child indent; its properties are the deeper lines after it.
                if indent(rest[k]) == child && rest[k].trim_end().ends_with(':') {
                    let input_name = scalar(rest[k].trim().trim_end_matches(':'));
                    let mut end = k + 1;
                    while end < rest.len() && indent(rest[end]) > child {
                        end += 1;
                    }
                    inputs.push(read_input(input_name, &rest[k + 1..end]));
                    k = end;
                } else {
                    k += 1;
                }
            }
        }
    }
    Workflow { file: file.into(), name, dispatch, inputs }
}

/// The properties of one input (the lines below its name).
fn read_input(name: String, props: &[&str]) -> Input {
    let mut input = Input { name, description: String::new(), required: false, default: String::new(), kind: InputKind::Text, options: Vec::new() };
    let mut kind = String::new();
    let mut i = 0;
    while i < props.len() {
        if let Some((key, value)) = pair(props[i]) {
            match key.as_str() {
                "description" => input.description = value,
                "required" => input.required = value == "true",
                "default" => input.default = value,
                "type" => kind = value,
                "options" => {
                    if let Some(list) = value.strip_prefix('[').and_then(|v| v.strip_suffix(']')) {
                        input.options = list.split(',').map(scalar).filter(|o| !o.is_empty()).collect();
                    } else {
                        // A block list: the `- item` lines right below.
                        let own = indent(props[i]);
                        while i + 1 < props.len() && indent(props[i + 1]) >= own && props[i + 1].trim_start().starts_with("- ") {
                            input.options.push(scalar(props[i + 1].trim_start().trim_start_matches("- ")));
                            i += 1;
                        }
                    }
                }
                _ => {}
            }
        }
        i += 1;
    }
    input.kind = match kind.as_str() {
        "boolean" => InputKind::Bool,
        "choice" | "environment" if !input.options.is_empty() => InputKind::Choice,
        _ => InputKind::Text,
    };
    // A choice starts on its first option unless it has a default; a checkbox starts unchecked.
    if input.kind == InputKind::Choice && !input.options.contains(&input.default) {
        input.default = input.options[0].clone();
    }
    if input.kind == InputKind::Bool && input.default != "true" {
        input.default = "false".to_string();
    }
    input
}

// ---- writing new workflows ----

/// What a new workflow is for; it sets the defaults of the wizard and the shape of the jobs.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Kind {
    /// Build and test on pushes and pull requests.
    Ci,
    /// Build on a version tag and publish a GitHub release with the files.
    Release,
    /// Build and test on a schedule.
    Scheduled,
    /// A command that is started by hand from GitHub or from Oxide.
    Manual,
}

impl Kind {
    pub const ALL: [Kind; 4] = [Kind::Ci, Kind::Release, Kind::Scheduled, Kind::Manual];

    pub fn name(self) -> &'static str {
        match self {
            Kind::Ci => "CI",
            Kind::Release => "Release",
            Kind::Scheduled => "Scheduled",
            Kind::Manual => "Manual task",
        }
    }

    pub fn note(self) -> &'static str {
        match self {
            Kind::Ci => "build and test on every push and pull request",
            Kind::Release => "build on a version tag and publish a GitHub release with the files",
            Kind::Scheduled => "build and test on a schedule, e.g. every Monday",
            Kind::Manual => "run a command when you start it, from GitHub or from Oxide (N)",
        }
    }

    /// The file name suggested for a workflow of this kind.
    pub fn file(self) -> &'static str {
        match self {
            Kind::Ci => "ci.yml",
            Kind::Release => "release.yml",
            Kind::Scheduled => "scheduled.yml",
            Kind::Manual => "manual.yml",
        }
    }
}

/// A language or build tool a workflow can set up and run.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Tc {
    Rust,
    Node,
    Python,
    Go,
    Maven,
    Gradle,
    DotNet,
    Ruby,
    Php,
    Elixir,
    Zig,
    CMake,
    Make,
    Swift,
    Dart,
    /// No setup: the commands are typed in the wizard.
    Custom,
}

/// The toolchains offered, with the files of a repository that suggest each one.
pub const TOOLCHAINS: &[(Tc, &str, &[&str])] = &[
    (Tc::Rust, "Rust (cargo)", &["Cargo.toml"]),
    (Tc::Node, "Node.js (npm, yarn, pnpm)", &["package.json"]),
    (Tc::Python, "Python (pip, pytest)", &["pyproject.toml", "requirements.txt", "setup.py"]),
    (Tc::Go, "Go", &["go.mod"]),
    (Tc::Maven, "Java (Maven)", &["pom.xml"]),
    (Tc::Gradle, "Java / Kotlin (Gradle)", &["build.gradle", "build.gradle.kts"]),
    (Tc::DotNet, "C# (.NET)", &["*.csproj", "*.sln"]),
    (Tc::Ruby, "Ruby (bundler)", &["Gemfile"]),
    (Tc::Php, "PHP (composer)", &["composer.json"]),
    (Tc::Elixir, "Elixir (mix)", &["mix.exs"]),
    (Tc::Zig, "Zig", &["build.zig"]),
    (Tc::CMake, "C / C++ (CMake)", &["CMakeLists.txt"]),
    (Tc::Make, "C / C++ (Make)", &["Makefile"]),
    (Tc::Swift, "Swift", &["Package.swift"]),
    (Tc::Dart, "Dart", &["pubspec.yaml"]),
    (Tc::Custom, "Other: type the commands", &[]),
];

/// What the repository itself says, to choose commands that fit it.
#[derive(Clone, Debug, Default)]
pub struct Facts {
    /// `name` of the Rust package, the binary a release ships.
    pub cargo_name: Option<String>,
    /// `gradlew` is committed.
    pub gradlew: bool,
    pub requirements: bool,
    /// `npm`, `yarn` or `pnpm`, by lock file.
    pub node_pm: String,
    pub node_build: bool,
    pub node_test: bool,
    pub phpunit: bool,
    pub make_test: bool,
}

/// The index in `TOOLCHAINS` of the toolchain a repository looks like it uses, and what it says about itself.
pub fn detect(repo: &Path) -> (Option<usize>, Facts) {
    let has = |name: &str| repo.join(name).exists();
    let any = |pattern: &str| match pattern.strip_prefix('*') {
        Some(ext) => std::fs::read_dir(repo).into_iter().flatten().flatten().any(|e| e.file_name().to_string_lossy().ends_with(ext)),
        None => has(pattern),
    };
    let found = TOOLCHAINS.iter().position(|(_, _, files)| files.iter().any(|f| any(f)));
    let text = |name: &str| std::fs::read_to_string(repo.join(name)).unwrap_or_default();
    let cargo_name = text("Cargo.toml").lines().skip_while(|l| l.trim() != "[package]").skip(1).take_while(|l| !l.trim_start().starts_with('[')).find_map(|l| {
        let (k, v) = l.split_once('=')?;
        (k.trim() == "name").then(|| v.trim().trim_matches('"').to_string())
    });
    let package = text("package.json");
    let facts = Facts {
        cargo_name,
        gradlew: has("gradlew"),
        requirements: has("requirements.txt"),
        node_pm: if has("pnpm-lock.yaml") { "pnpm" } else if has("yarn.lock") { "yarn" } else { "npm" }.to_string(),
        node_build: package.contains("\"build\":"),
        node_test: package.contains("\"test\":"),
        phpunit: has("phpunit.xml") || has("phpunit.xml.dist"),
        make_test: text("Makefile").lines().any(|l| l.starts_with("test:")),
    };
    (found, facts)
}

/// The choices of the wizard, which `compile` turns into a file.
#[derive(Clone, Debug)]
pub struct Spec {
    pub kind: Kind,
    /// An index into `TOOLCHAINS`.
    pub toolchain: usize,
    pub facts: Facts,
    pub name: String,
    pub branches: Vec<String>,
    pub push: bool,
    pub pull_request: bool,
    /// A push of a `v*` tag.
    pub tags: bool,
    pub schedule: bool,
    pub cron: String,
    pub dispatch: bool,
    pub linux: bool,
    pub macos: bool,
    pub windows: bool,
    pub cache: bool,
    /// Keep the built files as run artifacts.
    pub artifacts: bool,
    /// Cancel a run that a newer push makes obsolete.
    pub concurrency: bool,
    /// Publish a GitHub release (kind `Release`).
    pub release: bool,
    /// The command of a manual task, or the commands (separated by `;`) of the custom toolchain.
    pub command: String,
}

impl Spec {
    /// The defaults for a workflow of `kind` on the repository's branch `branch`.
    pub fn new(kind: Kind, toolchain: usize, facts: Facts, branch: &str) -> Spec {
        Spec {
            kind,
            toolchain,
            facts,
            name: kind.name().to_string(),
            branches: vec![branch.to_string()],
            push: kind == Kind::Ci,
            pull_request: kind == Kind::Ci,
            tags: kind == Kind::Release,
            schedule: kind == Kind::Scheduled,
            cron: "0 6 * * 1".to_string(),
            dispatch: true,
            linux: true,
            macos: false,
            windows: false,
            cache: true,
            artifacts: kind == Kind::Release,
            concurrency: kind == Kind::Ci,
            release: kind == Kind::Release,
            command: String::new(),
        }
    }

    pub fn tc(&self) -> Tc {
        TOOLCHAINS[self.toolchain].0
    }

    /// Why this spec cannot be written, if it cannot.
    pub fn check(&self) -> Result<(), String> {
        if self.name.trim().is_empty() {
            return Err("the workflow needs a name".to_string());
        }
        if !(self.push || self.pull_request || self.tags || self.schedule || self.dispatch) {
            return Err("pick at least one trigger".to_string());
        }
        if (self.push || self.pull_request) && self.branches.is_empty() {
            return Err("name at least one branch for the push and pull request triggers".to_string());
        }
        if self.schedule && self.cron.split_whitespace().count() != 5 {
            return Err("the schedule is a cron expression with 5 fields, e.g. 0 6 * * 1".to_string());
        }
        if !(self.linux || self.macos || self.windows) {
            return Err("pick at least one system to run on".to_string());
        }
        if (self.kind == Kind::Manual || self.tc() == Tc::Custom) && self.command.trim().is_empty() {
            return Err("type the command to run".to_string());
        }
        Ok(())
    }
}

/// `s` as a YAML scalar: as it is when that is safe, else in single quotes.
fn yq(s: &str) -> String {
    let plain = !s.is_empty()
        && s.chars().all(|c| c.is_ascii_alphanumeric() || " _./-".contains(c))
        && !s.starts_with([' ', '-', '.'])
        && !s.ends_with(' ')
        && !matches!(s.to_lowercase().as_str(), "true" | "false" | "yes" | "no" | "on" | "off" | "null" | "~")
        && s.parse::<f64>().is_err();
    if plain { s.to_string() } else { format!("'{}'", s.replace('\'', "''")) }
}

/// An action at the version of pin `key` (see `scaffold::PINS`): `actions/checkout@v4`.
fn action(name: &str, key: &str) -> String {
    format!("{name}@{}", crate::scaffold::pin_value(key))
}

/// The value of pin `key` as a quoted YAML string, so `3.10` stays text.
fn quoted(key: &str) -> String {
    format!("'{}'", crate::scaffold::pin_value(key))
}

/// A step that uses an action, with its `with:` settings.
fn uses(action: &str, with: &[(&str, &str)]) -> String {
    let mut s = format!("- uses: {action}");
    if !with.is_empty() {
        s.push_str("\n  with:");
        for (k, v) in with {
            s.push_str(&format!("\n    {k}: {v}"));
        }
    }
    s
}

/// A step that runs a command.
fn run(name: &str, command: &str) -> String {
    format!("- name: {}\n  run: {command}", yq(name))
}

/// The runner systems of a spec, as `(runs-on names)`.
fn systems(spec: &Spec) -> Vec<&'static str> {
    [(spec.linux, "ubuntu-latest"), (spec.macos, "macos-latest"), (spec.windows, "windows-latest")].iter().filter(|(on, _)| *on).map(|(_, n)| *n).collect()
}

/// The steps that set a toolchain up, the commands that build and test with it and the files it produces.
/// `packaging` asks for build output to keep or publish.
fn recipe(spec: &Spec, packaging: bool) -> (Vec<String>, Vec<String>, Vec<String>) {
    let f = &spec.facts;
    let cache = spec.cache;
    let repo_name = "${{ github.event.repository.name }}";
    let (mut setup, mut cmds, mut files): (Vec<String>, Vec<String>, Vec<String>) = (Vec::new(), Vec::new(), Vec::new());
    let cmd = |name: &str, c: &str| run(name, c);
    match spec.tc() {
        Tc::Rust => {
            setup.push(uses(&action("dtolnay/rust-toolchain", "gha_rust_toolchain"), &[]));
            if cache {
                setup.push(uses(&action("Swatinem/rust-cache", "gha_rust_cache"), &[]));
            }
            cmds.push(cmd("Build", "cargo build --release"));
            cmds.push(cmd("Test", "cargo test"));
            let bin = f.cargo_name.clone().unwrap_or_else(|| repo_name.to_string());
            files.extend([format!("target/release/{bin}"), format!("target/release/{bin}.exe")]);
        }
        Tc::Node => {
            let pm = f.node_pm.as_str();
            if pm == "pnpm" {
                setup.push(uses(&action("pnpm/action-setup", "gha_pnpm"), &[]));
            }
            let node = quoted("ci_node");
            let mut with = vec![("node-version", node.as_str())];
            if cache {
                with.push(("cache", pm));
            }
            setup.push(uses(&action("actions/setup-node", "gha_node"), &with));
            let install = match pm {
                "pnpm" => "pnpm install --frozen-lockfile",
                "yarn" => "yarn install --frozen-lockfile",
                _ => "npm ci",
            };
            cmds.push(cmd("Install", install));
            if f.node_build {
                cmds.push(cmd("Build", &format!("{pm} run build")));
            }
            if f.node_test {
                cmds.push(cmd("Test", &format!("{pm} test")));
            }
            files.extend(["dist".to_string(), "build".to_string()]);
        }
        Tc::Python => {
            let python = quoted("ci_python");
            let mut with = vec![("python-version", python.as_str())];
            if cache {
                with.push(("cache", "pip"));
            }
            setup.push(uses(&action("actions/setup-python", "gha_python"), &with));
            cmds.push(cmd("Install", "python -m pip install --upgrade pip pytest"));
            cmds.push(cmd("Install the project", if f.requirements { "python -m pip install -r requirements.txt" } else { "python -m pip install -e ." }));
            cmds.push(cmd("Test", "python -m pytest"));
            if packaging {
                cmds.push(cmd("Build the package", "python -m pip install build && python -m build"));
                files.push("dist/*".to_string());
            }
        }
        Tc::Go => {
            let mut with = vec![("go-version-file", "go.mod")];
            if !cache {
                with.push(("cache", "false"));
            }
            setup.push(uses(&action("actions/setup-go", "gha_go"), &with));
            cmds.push(cmd("Build", if packaging { "mkdir -p dist && go build -o dist/ ./..." } else { "go build ./..." }));
            cmds.push(cmd("Test", "go test ./..."));
            if packaging {
                files.push("dist/*".to_string());
            }
        }
        Tc::Maven | Tc::Gradle => {
            let java = crate::scaffold::pin_value("java");
            let java = format!("'{java}'");
            let mut with = vec![("distribution", "temurin"), ("java-version", java.as_str())];
            let tool = if spec.tc() == Tc::Maven { "maven" } else { "gradle" };
            if cache {
                with.push(("cache", tool));
            }
            setup.push(uses(&action("actions/setup-java", "gha_java"), &with));
            if spec.tc() == Tc::Maven {
                cmds.push(cmd("Build and test", "mvn -B -ntp verify"));
                files.push("target/*.jar".to_string());
            } else {
                if f.gradlew {
                    cmds.push(cmd("Build and test", "bash ./gradlew build"));
                } else {
                    setup.push(uses(&action("gradle/actions/setup-gradle", "gha_gradle"), &[("gradle-version", crate::scaffold::pin_value("ci_gradle").as_str())]));
                    cmds.push(cmd("Build and test", "gradle build"));
                }
                files.push("build/libs/*.jar".to_string());
            }
        }
        Tc::DotNet => {
            let dotnet = crate::scaffold::pin_value("dotnet");
            let version = format!("'{}.x'", dotnet.trim_start_matches("net"));
            setup.push(uses(&action("actions/setup-dotnet", "gha_dotnet"), &[("dotnet-version", version.as_str())]));
            cmds.push(cmd("Build", "dotnet build -c Release"));
            cmds.push(cmd("Test", "dotnet test -c Release --no-build"));
            files.push("**/bin/Release/**".to_string());
        }
        Tc::Ruby => {
            let ruby = quoted("ci_ruby");
            let with: Vec<(&str, &str)> = if cache { vec![("ruby-version", ruby.as_str()), ("bundler-cache", "true")] } else { vec![("ruby-version", ruby.as_str())] };
            setup.push(uses(&action("ruby/setup-ruby", "gha_ruby"), &with));
            if !cache {
                cmds.push(cmd("Install", "bundle install"));
            }
            cmds.push(cmd("Test", "bundle exec rake"));
        }
        Tc::Php => {
            setup.push(uses(&action("shivammathur/setup-php", "gha_php"), &[("php-version", quoted("ci_php").as_str()), ("tools", "composer")]));
            if cache {
                setup.push(uses(&action("actions/cache", "gha_cache"), &[("path", "vendor"), ("key", "composer-${{ hashFiles('**/composer.lock') }}")]));
            }
            cmds.push(cmd("Install", "composer install --no-progress --prefer-dist"));
            cmds.push(cmd("Validate", "composer validate --strict"));
            if f.phpunit {
                cmds.push(cmd("Test", "vendor/bin/phpunit"));
            }
        }
        Tc::Elixir => {
            setup.push(uses(&action("erlef/setup-beam", "gha_beam"), &[("elixir-version", quoted("ci_elixir").as_str()), ("otp-version", quoted("ci_otp").as_str())]));
            if cache {
                setup.push(uses(&action("actions/cache", "gha_cache"), &[("path", "|\n      deps\n      _build"), ("key", "mix-${{ hashFiles('mix.lock') }}")]));
            }
            cmds.push(cmd("Install", "mix deps.get"));
            cmds.push(cmd("Test", "mix test"));
        }
        Tc::Zig => {
            setup.push(uses(&action("mlugg/setup-zig", "gha_zig"), &[]));
            cmds.push(cmd("Build", "zig build"));
            cmds.push(cmd("Test", "zig build test"));
            files.push("zig-out/bin/*".to_string());
        }
        Tc::CMake => {
            cmds.push(cmd("Configure", "cmake -S . -B build -DCMAKE_BUILD_TYPE=Release"));
            cmds.push(cmd("Build", "cmake --build build --config Release"));
            cmds.push(cmd("Test", "ctest --test-dir build -C Release --output-on-failure"));
            files.extend([format!("build/{repo_name}"), format!("build/Release/{repo_name}.exe")]);
        }
        Tc::Make => {
            cmds.push(cmd("Build", "make"));
            if f.make_test {
                cmds.push(cmd("Test", "make test"));
            }
            files.push(repo_name.to_string());
        }
        Tc::Swift => {
            setup.push(uses(&action("swift-actions/setup-swift", "gha_swift"), &[]));
            cmds.push(cmd("Build", "swift build -c release"));
            cmds.push(cmd("Test", "swift test"));
            files.push(format!(".build/release/{repo_name}"));
        }
        Tc::Dart => {
            setup.push(uses(&action("dart-lang/setup-dart", "gha_dart"), &[]));
            cmds.push(cmd("Install", "dart pub get"));
            cmds.push(cmd("Analyze", "dart analyze"));
        }
        Tc::Custom => {
            for (i, c) in spec.command.split(';').map(str::trim).filter(|c| !c.is_empty()).enumerate() {
                cmds.push(cmd(&format!("Step {}", i + 1), c));
            }
        }
    }
    (setup, cmds, files)
}

/// `steps` as the lines of a `steps:` list indented by `by` spaces.
fn indent_steps(steps: &[String], by: usize) -> String {
    let pad = " ".repeat(by);
    steps.iter().flat_map(|s| s.lines().map(|l| format!("{pad}{l}\n")).collect::<Vec<_>>()).collect()
}

/// The workflow file for `spec`, which has been through `Spec::check`.
pub fn compile(spec: &Spec) -> String {
    let mut y = format!("name: {}\n\n", yq(spec.name.trim()));

    // Triggers.
    y.push_str("on:\n");
    let branches = format!("[{}]", spec.branches.iter().map(|b| yq(b)).collect::<Vec<_>>().join(", "));
    if spec.push || spec.tags {
        y.push_str("  push:\n");
        if spec.push {
            y.push_str(&format!("    branches: {branches}\n"));
        }
        if spec.tags {
            y.push_str("    tags: ['v*']\n");
        }
    }
    if spec.pull_request {
        y.push_str(&format!("  pull_request:\n    branches: {branches}\n"));
    }
    if spec.schedule {
        y.push_str(&format!("  schedule:\n    - cron: '{}'\n", spec.cron.split_whitespace().collect::<Vec<_>>().join(" ")));
    }
    if spec.dispatch {
        y.push_str("  workflow_dispatch:\n");
        if spec.kind == Kind::Manual {
            y.push_str(&format!(
                "    inputs:\n      command:\n        description: Command to run\n        required: true\n        default: {}\n",
                yq(spec.command.trim())
            ));
        }
    }

    y.push_str("\npermissions:\n  contents: read\n");
    if spec.concurrency {
        y.push_str(&format!("\nconcurrency:\n  group: ${{{{ github.workflow }}}}-${{{{ github.ref }}}}\n  cancel-in-progress: {}\n", spec.kind != Kind::Release));
    }

    // The build job.
    let packaging = spec.artifacts || spec.kind == Kind::Release;
    let (setup, cmds, files) = recipe(spec, packaging);
    let systems = systems(spec);
    let multi = systems.len() > 1;
    let job = if spec.kind == Kind::Manual { "run" } else { "build" };
    y.push_str(&format!("\njobs:\n  {job}:\n"));
    if multi {
        y.push_str(&format!("    name: {job} (${{{{ matrix.os }}}})\n    runs-on: ${{{{ matrix.os }}}}\n    strategy:\n      fail-fast: false\n      matrix:\n        os: [{}]\n", systems.join(", ")));
    } else {
        y.push_str(&format!("    runs-on: {}\n", systems[0]));
    }
    if spec.windows {
        y.push_str("    defaults:\n      run:\n        shell: bash\n");
    }
    y.push_str("    steps:\n");
    let mut steps = vec![uses(&action("actions/checkout", "gha_checkout"), &[])];
    steps.extend(setup);
    if spec.kind == Kind::Manual {
        // The typed command comes in as an input; through an environment variable it is never pasted into the script.
        steps.push("- name: Run\n  env:\n    COMMAND: ${{ inputs.command }}\n  run: bash -c \"$COMMAND\"".to_string());
    } else {
        steps.extend(cmds);
    }
    if packaging && !files.is_empty() {
        let name = if multi { "dist-${{ matrix.os }}".to_string() } else { format!("dist-{}", systems[0]) };
        let list = files.join("\n      ");
        steps.push(format!("- uses: {}\n  with:\n    name: {name}\n    path: |\n      {list}\n    if-no-files-found: warn", action("actions/upload-artifact", "gha_upload")));
    }
    y.push_str(&indent_steps(&steps, 6));

    // The release job.
    if spec.kind == Kind::Release && spec.release {
        y.push_str("\n  release:\n    needs: build\n    if: startsWith(github.ref, 'refs/tags/')\n    runs-on: ubuntu-latest\n    permissions:\n      contents: write\n    steps:\n");
        let release = vec![
            uses(&action("actions/download-artifact", "gha_download"), &[("path", "dist"), ("pattern", "dist-*"), ("merge-multiple", "true")]),
            uses(&action("softprops/action-gh-release", "gha_release"), &[("files", "dist/**"), ("generate_release_notes", "true")]),
        ];
        y.push_str(&indent_steps(&release, 6));
    }
    y
}
