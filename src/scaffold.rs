//! New-project scaffolding: the languages the project menu offers and what setting each one up involves.
//!
//! A language has one or more variants (build tools, usually); a variant is a list of steps: commands to
//! run, directories to create and files to write. Anything a step says may use the placeholders
//! `{name}` (the directory name as it is), `{slug}` (lowercase words joined by `-`: `My App` -> `my-app`),
//! `{pkg}` (the same joined by `_`) and `{user}` (the Git user name as `{pkg}`). Names that go into a
//! command, a file name or a build file use `{slug}` or `{pkg}`, so a directory with spaces still works.
//! Commands are split on spaces and run without a shell.
//!
//! `Plan::build` turns a chosen variant into the exact list of actions, which the project menu shows before running them.

use std::{
    collections::HashSet,
    fs,
    path::Path,
    process::{Command, Stdio},
    sync::OnceLock,
};

use crate::{ignore, license::License};

/// One thing a variant does inside the new project.
pub enum Step {
    /// A command line, e.g. `cargo init --name {pkg}`.
    Run(&'static str),
    /// A directory, kept in Git with a `.gitkeep` when nothing else ends up in it.
    Dir(&'static str),
    /// A file with its contents.
    File(&'static str, &'static str),
}

use Step::{Dir, File, Run};

/// A way to set a language up (usually a build tool).
#[derive(Clone, Copy)]
pub struct Variant {
    pub name: &'static str,
    /// What it does in a few words, shown in the menu.
    pub note: &'static str,
    pub steps: &'static [Step],
}

/// What a language calls the name the menu asks for, which decides how it is checked.
#[derive(Clone, Copy, PartialEq)]
pub enum PackageKind {
    /// Packages: one or more dot-separated names (Java, Kotlin, Scala).
    Packages,
    /// A module path such as `github.com/you/app` (Go).
    Module,
    /// A single dot-separated namespace (C#).
    Namespace,
}

impl PackageKind {
    /// Name of the menu page and of the field on it.
    pub fn noun(self) -> &'static str {
        match self {
            PackageKind::Packages => "package",
            PackageKind::Module => "module",
            PackageKind::Namespace => "namespace",
        }
    }

    /// The line above the field.
    pub fn help(self) -> &'static str {
        match self {
            PackageKind::Packages => "Package names, separated by commas; the first one holds the main code.",
            PackageKind::Module => "Module path of go.mod, e.g. github.com/you/app.",
            PackageKind::Namespace => "Root namespace of the project, e.g. Company.App.",
        }
    }
}

/// The question a package-like language asks, with its suggested answer. The suggestion may use `{pkg}`,
/// `{slug}`, `{Pascal}` (`MyApp`) and `{user}`.
#[derive(Clone, Copy)]
pub struct Package {
    pub kind: PackageKind,
    pub suggest: &'static str,
}

#[derive(Clone, Copy)]
pub struct Lang {
    pub name: &'static str,
    /// Selects the `.gitignore` template (see `ignore::gitignore`); empty for none.
    pub key: &'static str,
    /// Extra `.gitignore` lines for languages defined in `config.lua`; empty for the built-in ones.
    pub ignore: &'static str,
    /// For languages whose code lives in packages, modules or namespaces: what the menu asks about before the
    /// variant. Steps use the answer as `{package}` (`com.example.app`) and `{package_path}` (`com/example/app`).
    pub package: Option<Package>,
    /// Never empty; the first one is the default.
    pub variants: &'static [Variant],
}

const fn v(name: &'static str, note: &'static str, steps: &'static [Step]) -> Variant {
    Variant { name, note, steps }
}

/// Every language in the menu: the built-in ones, then those of `config.lua` (one with the name of a built-in
/// language replaces it). Fixed once `register` or the first `langs` call has run.
static ALL: OnceLock<Vec<Lang>> = OnceLock::new();

/// Makes the languages of `config.lua` part of the menu; call it once, before the menu is first used.
pub fn register(user: Vec<Lang>) {
    let mut all = BUILTIN.to_vec();
    for lang in user {
        match all.iter().position(|l| l.name.eq_ignore_ascii_case(lang.name)) {
            Some(i) => all[i] = lang,
            None => all.push(lang),
        }
    }
    let _ = ALL.set(all);
}

pub fn langs() -> &'static [Lang] {
    ALL.get_or_init(|| BUILTIN.to_vec())
}

/// The built-in languages, most popular first.
const BUILTIN: &[Lang] = &[
    Lang {
        name: "Rust",
        key: "rust",
        ignore: "",
        package: None,
        variants: &[
            v("Binary", "cargo init", &[Run("cargo init --name {pkg}")]),
            v("Library", "cargo init --lib", &[Run("cargo init --lib --name {pkg}")]),
        ],
    },
    Lang {
        name: "Python",
        key: "python",
        ignore: "",
        package: None,
        variants: &[
            v("venv + pip", "src/ layout, pyproject.toml, .venv", &[
                Run("python3 -m venv .venv"),
                File("pyproject.toml", PY_PROJECT),
                File("src/{pkg}/__init__.py", ""),
                File("src/{pkg}/main.py", PY_MAIN),
                Dir("tests"),
            ]),
            v("uv", "uv init --package", &[Run("uv init --package --name {pkg}"), Dir("tests")]),
        ],
    },
    Lang {
        name: "JavaScript",
        key: "node",
        ignore: "",
        package: None,
        variants: &[
            v("npm", "npm init -y, src/", &[Run("npm init -y"), File("src/index.js", JS_MAIN)]),
            v("pnpm", "pnpm init, src/", &[Run("pnpm init"), File("src/index.js", JS_MAIN)]),
            v("Bun", "bun init -y", &[Run("bun init -y")]),
        ],
    },
    Lang {
        name: "TypeScript",
        key: "node",
        ignore: "",
        package: None,
        variants: &[
            v("npm", "npm init -y, typescript, tsconfig.json", &[
                Run("npm init -y"),
                Run("npm install --save-dev typescript @types/node"),
                File("tsconfig.json", TS_CONFIG),
                File("src/index.ts", TS_MAIN),
            ]),
            v("Bun", "bun init -y (TypeScript by default)", &[Run("bun init -y")]),
        ],
    },
    Lang {
        name: "Go",
        key: "go",
        ignore: "",
        package: Some(Package { kind: PackageKind::Module, suggest: "github.com/{user}/{slug}" }),
        variants: &[v("Module", "go mod init, cmd/ and internal/", &[
            Run("go mod init {package}"),
            File("cmd/{pkg}/main.go", GO_MAIN),
            Dir("internal"),
        ])],
    },
    Lang {
        name: "Java",
        key: "java",
        ignore: "",
        package: Some(Package { kind: PackageKind::Packages, suggest: "com.example.{pkg}" }),
        variants: &[
            v("Maven", "pom.xml, src/main/java, src/test/java", &[
                File("pom.xml", JAVA_POM),
                File("src/main/java/{package_path}/Main.java", JAVA_MAIN),
                Dir("src/main/java/{package_path}"),
                Dir("src/test/java/{package_path}"),
            ]),
            v("Gradle (Groovy)", "build.gradle, settings.gradle", &[
                File("settings.gradle", "rootProject.name = '{slug}'\n"),
                File("build.gradle", JAVA_GRADLE),
                File("src/main/java/{package_path}/Main.java", JAVA_MAIN),
                Dir("src/main/java/{package_path}"),
                Dir("src/test/java/{package_path}"),
            ]),
            v("Gradle (Kotlin DSL)", "build.gradle.kts, settings.gradle.kts", &[
                File("settings.gradle.kts", "rootProject.name = \"{slug}\"\n"),
                File("build.gradle.kts", JAVA_GRADLE_KTS),
                File("src/main/java/{package_path}/Main.java", JAVA_MAIN),
                Dir("src/main/java/{package_path}"),
                Dir("src/test/java/{package_path}"),
            ]),
        ],
    },
    Lang {
        name: "Kotlin",
        key: "kotlin",
        ignore: "",
        package: Some(Package { kind: PackageKind::Packages, suggest: "com.example.{pkg}" }),
        variants: &[v("Gradle (Kotlin DSL)", "build.gradle.kts, src/main/kotlin", &[
            File("settings.gradle.kts", "rootProject.name = \"{slug}\"\n"),
            File("build.gradle.kts", KOTLIN_GRADLE),
            File("src/main/kotlin/{package_path}/Main.kt", KOTLIN_MAIN),
            Dir("src/main/kotlin/{package_path}"),
            Dir("src/test/kotlin/{package_path}"),
        ])],
    },
    Lang {
        name: "C",
        key: "c",
        ignore: "",
        package: None,
        variants: &[v("Makefile", "Makefile, src/, include/", &[
            File("Makefile", C_MAKEFILE),
            File("src/main.c", C_MAIN),
            Dir("include"),
        ])],
    },
    Lang {
        name: "C++",
        key: "cpp",
        ignore: "",
        package: None,
        variants: &[
            v("CMake", "CMakeLists.txt, src/, include/", &[
                File("CMakeLists.txt", CPP_CMAKE),
                File("src/main.cpp", CPP_MAIN),
                Dir("include"),
            ]),
            v("Makefile", "Makefile, src/, include/", &[
                File("Makefile", CPP_MAKEFILE),
                File("src/main.cpp", CPP_MAIN),
                Dir("include"),
            ]),
        ],
    },
    Lang {
        name: "C#",
        key: "csharp",
        ignore: "",
        package: Some(Package { kind: PackageKind::Namespace, suggest: "{Pascal}" }),
        variants: &[
            v("Console", "{slug}.csproj, Program.cs", &[
                File("{slug}.csproj", CSHARP_EXE),
                File("src/Program.cs", CSHARP_MAIN),
            ]),
            v("Class library", "{slug}.csproj, Class1.cs", &[
                File("{slug}.csproj", CSHARP_LIB),
                File("src/Class1.cs", CSHARP_CLASS),
            ]),
        ],
    },
    Lang {
        name: "PHP",
        key: "php",
        ignore: "",
        package: None,
        variants: &[v("Composer", "composer init, src/", &[
            Run("composer init --no-interaction --name={user}/{pkg}"),
            File("src/index.php", PHP_MAIN),
        ])],
    },
    Lang {
        name: "Ruby",
        key: "ruby",
        ignore: "",
        package: None,
        variants: &[v("Bundler", "bundle init, lib/", &[
            Run("bundle init"),
            File("lib/{pkg}.rb", RUBY_MAIN),
            Dir("spec"),
        ])],
    },
    Lang {
        name: "Swift",
        key: "swift",
        ignore: "",
        package: None,
        variants: &[
            v("Executable", "swift package init", &[Run("swift package init --type executable --name {pkg}")]),
            v("Library", "swift package init --type library", &[Run("swift package init --type library --name {pkg}")]),
        ],
    },
    Lang {
        name: "Dart",
        key: "dart",
        ignore: "",
        package: None,
        variants: &[v("Console", "dart create", &[Run("dart create --force --template console .")])],
    },
    Lang {
        name: "Lua",
        key: "lua",
        ignore: "",
        package: None,
        variants: &[v("Plain", "src/main.lua, tests/", &[File("src/main.lua", LUA_MAIN), Dir("tests")])],
    },
    Lang {
        name: "Zig",
        key: "zig",
        ignore: "",
        package: None,
        variants: &[v("zig init", "zig init", &[Run("zig init")])],
    },
    Lang {
        name: "Elixir",
        key: "elixir",
        ignore: "",
        package: None,
        variants: &[v("Mix", "mix new", &[Run("mix new . --app {pkg}")])],
    },
    Lang {
        name: "Haskell",
        key: "haskell",
        ignore: "",
        package: None,
        variants: &[v("Cabal", "cabal init, app/", &[Run("cabal init --non-interactive --exe --package-name={slug}")])],
    },
    Lang {
        name: "Scala",
        key: "scala",
        ignore: "",
        package: Some(Package { kind: PackageKind::Packages, suggest: "com.example.{pkg}" }),
        variants: &[v("sbt", "build.sbt, src/main/scala", &[
            File("build.sbt", SCALA_SBT),
            File("project/build.properties", "sbt.version=1.10.7\n"),
            File("src/main/scala/{package_path}/Main.scala", SCALA_MAIN),
            Dir("src/main/scala/{package_path}"),
            Dir("src/test/scala/{package_path}"),
        ])],
    },
    Lang {
        name: "R",
        key: "r",
        ignore: "",
        package: None,
        variants: &[v("Scripts", "R/, data/, main.R", &[File("main.R", R_MAIN), Dir("R"), Dir("data")])],
    },
    Lang {
        name: "Julia",
        key: "julia",
        ignore: "",
        package: None,
        variants: &[v("Package", "Project.toml, src/, test/", &[
            File("Project.toml", JULIA_PROJECT),
            File("src/{pkg}.jl", JULIA_MAIN),
            Dir("test"),
        ])],
    },
    Lang {
        name: "Shell",
        key: "shell",
        ignore: "",
        package: None,
        variants: &[v("Script", "bin/ script, lib/", &[
            File("bin/{slug}", SHELL_MAIN),
            Run("chmod +x bin/{slug}"),
            Dir("lib"),
        ])],
    },
    Lang {
        name: "Nim",
        key: "nim",
        ignore: "",
        package: None,
        variants: &[v("Nimble", "nimble file, src/", &[
            File("{slug}.nimble", NIM_NIMBLE),
            File("src/{pkg}.nim", NIM_MAIN),
            Dir("tests"),
        ])],
    },
    Lang {
        name: "Perl",
        key: "perl",
        ignore: "",
        package: None,
        variants: &[v("Plain", "script/, lib/, t/, cpanfile", &[
            File("script/main.pl", PERL_MAIN),
            File("cpanfile", "# requires 'Some::Module';\n"),
            Dir("lib"),
            Dir("t"),
        ])],
    },
    Lang {
        name: "Clojure",
        key: "clojure",
        ignore: "",
        package: None,
        variants: &[v("deps.edn", "deps.edn, src/, test/", &[
            File("deps.edn", CLJ_DEPS),
            File("src/{pkg}/core.clj", CLJ_MAIN),
            Dir("test"),
        ])],
    },
];

// ---- plan ----

/// How a file is written when something may already be there.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Write {
    /// Replace whatever is there.
    Always,
    /// Leave an existing file alone (tools like `uv init` or `dotnet new` write a README of their own).
    IfMissing,
    /// Add the lines that are not in the file yet (`cargo init` writes a `.gitignore` of its own).
    Merge,
}

/// A step with its placeholders filled in.
#[derive(Clone, Debug)]
pub enum Action {
    Run { program: String, args: Vec<String> },
    File { path: String, text: String, mode: Write },
}

impl Action {
    /// One line for the confirmation screen.
    pub fn label(&self) -> String {
        match self {
            Action::Run { program, args } => format!("$ {program} {}", args.join(" ")).trim_end().to_string(),
            Action::File { path, mode: Write::Merge, .. } => format!("write {path} (merged with the existing one)"),
            Action::File { path, mode: Write::IfMissing, .. } => format!("write {path} (unless a tool made it)"),
            Action::File { path, .. } => format!("write {path}"),
        }
    }
}

/// Everything setting a project up will do, in order, without having done any of it yet.
#[derive(Clone, Debug)]
pub struct Plan {
    pub actions: Vec<Action>,
    /// Programs the variant needs that are not on `PATH`; their commands are left out of `actions`.
    pub missing: Vec<String>,
}

impl Plan {
    /// The plan for `root`: the variant's steps, then a `.gitignore` for the language, a README
    /// and the license (when one was chosen) with `holder` and `year` in its copyright line.
    pub fn build(root: &Path, lang: &Lang, variant: &Variant, packages: &[String], license: Option<&License>, holder: &str, year: i64) -> Plan {
        let name = project_name(root);
        let (pkg, slug, user) = (ident(&name, '_', "app"), ident(&name, '-', "app"), ident(holder, '_', "app"));
        let pascal = pascal(&name);
        // `{package}` is the first package: the one that holds the main code. Directories that mention a
        // package are made for every package, so the others get their (empty) folders too.
        let fill_for = |s: &str, package: &str| {
            s.replace("{name}", &name)
                .replace("{pkg}", &pkg)
                .replace("{slug}", &slug)
                .replace("{user}", &user)
                .replace("{Pascal}", &pascal)
                .replace("{package_path}", &package.replace('.', "/"))
                .replace("{package}", package)
        };
        let main = packages.first().map_or("", String::as_str);
        let fill = |s: &str| fill_for(s, main);

        let mut actions = Vec::new();
        let mut missing: Vec<String> = Vec::new();
        for step in variant.steps {
            match step {
                Run(line) => {
                    let line = fill(line);
                    let mut words = line.split_whitespace().map(str::to_string);
                    let Some(program) = words.next() else { continue };
                    if on_path(&program) {
                        actions.push(Action::Run { program, args: words.collect() });
                    } else if !missing.contains(&program) {
                        missing.push(program);
                    }
                }
                Dir(path) => {
                    let each: Vec<String> = if path.contains("{package") && packages.len() > 1 {
                        packages.iter().map(|p| fill_for(path, p)).collect()
                    } else {
                        vec![fill(path)]
                    };
                    for path in each {
                        // An empty directory would not exist in Git, so it gets a marker file, unless a file lands in it.
                        let prefix = format!("{path}/");
                        if !variant.steps.iter().any(|s| matches!(s, File(f, _) if fill(f).starts_with(&prefix))) {
                            actions.push(Action::File { path: format!("{path}/.gitkeep"), text: String::new(), mode: Write::IfMissing });
                        }
                    }
                }
                File(path, text) => actions.push(Action::File { path: fill(path), text: fill(text), mode: Write::Always }),
            }
        }
        let mut ignored = ignore::gitignore(lang.key);
        if !lang.ignore.trim().is_empty() {
            ignored = format!("# {}\n{}\n\n{ignored}", lang.name, lang.ignore.trim_end());
        }
        actions.push(Action::File { path: ".gitignore".to_string(), text: ignored, mode: Write::Merge });
        actions.push(Action::File { path: "README.md".to_string(), text: format!("# {name}\n"), mode: Write::IfMissing });
        if let Some(l) = license {
            actions.push(Action::File { path: "LICENSE".to_string(), text: l.text(holder, year), mode: Write::Always });
        }
        Plan { actions, missing }
    }
}

impl Plan {
    /// Does every action inside `root`, in order, and stops at the first one that fails. `say` is called
    /// with one line per finished action.
    pub fn apply(&self, root: &Path, mut say: impl FnMut(String)) -> Result<(), String> {
        for action in &self.actions {
            match action {
                Action::Run { program, args } => {
                    let out = Command::new(program)
                        .args(args)
                        .current_dir(root)
                        .stdin(Stdio::null())
                        .output()
                        .map_err(|e| format!("{}: {e}", action.label()))?;
                    if !out.status.success() {
                        // Tools explain themselves on stderr; some (npm) use stdout.
                        let text = String::from_utf8_lossy(if out.stderr.is_empty() { &out.stdout } else { &out.stderr }).trim().to_string();
                        return Err(format!("{} failed: {}", action.label(), text.lines().last().unwrap_or("no output")));
                    }
                }
                Action::File { path, text, mode } => {
                    let target = root.join(path);
                    if let Some(dir) = target.parent() {
                        fs::create_dir_all(dir).map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
                    }
                    let done = match (mode, target.exists()) {
                        (Write::IfMissing, true) => Ok(false),
                        (Write::Merge, true) => {
                            let old = fs::read_to_string(&target).map_err(|e| format!("cannot read {path}: {e}"))?;
                            match merge_lines(&old, text) {
                                Some(merged) => fs::write(&target, merged).map(|()| true),
                                None => Ok(false),
                            }
                        }
                        _ => fs::write(&target, text).map(|()| true),
                    };
                    if !done.map_err(|e| format!("cannot write {path}: {e}"))? {
                        say(format!("{path}: kept as it was"));
                        continue;
                    }
                }
            }
            say(action.label());
        }
        Ok(())
    }
}

/// `old` with the entries of `template` that it lacks appended, or None when it has them all. The template
/// is handled in blocks (separated by blank lines) so a comment travels with its entries.
fn merge_lines(old: &str, template: &str) -> Option<String> {
    let have: HashSet<&str> = old.lines().map(str::trim).collect();
    let mut add: Vec<String> = Vec::new();
    for block in template.split("\n\n") {
        let entries = |l: &&str| !l.trim().is_empty() && !l.trim_start().starts_with('#');
        if block.lines().filter(entries).all(|l| have.contains(l.trim())) {
            continue;
        }
        // Keep the block's comments, drop the entries that are already there.
        let kept: Vec<&str> = block.lines().filter(|l| !(entries(l) && have.contains(l.trim()))).collect();
        add.push(kept.join("\n"));
    }
    if add.is_empty() {
        return None;
    }
    let sep = if old.is_empty() { "" } else if old.ends_with('\n') { "\n" } else { "\n\n" };
    Some(format!("{old}{sep}{}\n", add.join("\n\n").trim_end()))
}

/// The answer suggested on the package page for a project in `root` whose owner is `holder`.
pub fn default_package(root: &Path, lang: &Lang, holder: &str) -> Option<String> {
    let name = project_name(root);
    lang.package.map(|p| {
        p.suggest
            .replace("{pkg}", &ident(&name, '_', "app"))
            .replace("{slug}", &ident(&name, '-', "app"))
            .replace("{Pascal}", &pascal(&name))
            .replace("{user}", &ident(holder, '-', "user"))
    })
}

/// What was typed on the package page, checked for `kind`: for packages, names separated by commas (the first
/// holds the main code); for a module or namespace, exactly one. The error says what is wrong.
pub fn parse_packages(text: &str, kind: PackageKind) -> Result<Vec<String>, String> {
    let names: Vec<&str> = text.split(',').map(str::trim).filter(|n| !n.is_empty()).collect();
    let mut out: Vec<String> = Vec::new();
    for name in &names {
        let dotted = |name: &str| {
            name.split('.').all(|part| {
                let mut chars = part.chars();
                chars.next().is_some_and(|c| c.is_ascii_alphabetic() || c == '_') && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
            })
        };
        let ok = match kind {
            PackageKind::Packages | PackageKind::Namespace => dotted(name),
            // A module path is a URL-like path: no spaces, no empty parts.
            PackageKind::Module => {
                name.split('/').all(|part| !part.is_empty() && part.chars().all(|c| c.is_ascii_alphanumeric() || "._~-".contains(c)))
            }
        };
        if !ok {
            return Err(match kind {
                PackageKind::Module => format!("'{name}' is not a module path (letters, digits and . _ ~ - between slashes)"),
                _ => format!("'{name}' is not a valid {} (letters, digits, _ and dots; no leading digit)", kind.noun()),
            });
        }
        if !out.iter().any(|p| p == name) {
            out.push((*name).to_string());
        }
    }
    match (out.len(), kind) {
        (0, _) => Err(format!("write a {} name", kind.noun())),
        (n, PackageKind::Module | PackageKind::Namespace) if n > 1 => Err(format!("only one {} is possible", kind.noun())),
        _ => Ok(out),
    }
}

/// A repository can be set up when nothing is in it but Git's own `.git` (a folder, or a file in worktrees).
pub fn is_empty(root: &Path) -> bool {
    std::fs::read_dir(root).is_ok_and(|mut entries| entries.all(|e| e.is_ok_and(|e| e.file_name() == ".git")))
}

/// The name of the project in `root`: the name of its directory.
fn project_name(root: &Path) -> String {
    root.file_name().map_or_else(|| "project".to_string(), |n| n.to_string_lossy().into_owned())
}

/// `s` in PascalCase (`my-app` -> `MyApp`), or `App` when nothing is left of it.
fn pascal(s: &str) -> String {
    let out: String = s
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(|w| {
            let mut chars = w.chars();
            chars.next().map_or_else(String::new, |c| c.to_uppercase().chain(chars).collect())
        })
        .collect();
    match out.chars().next() {
        None => "App".to_string(),
        Some(c) if c.is_ascii_digit() => format!("P{out}"),
        Some(_) => out,
    }
}

/// `s` as a lowercase identifier with `sep` between words (`My App` -> `my_app`), or `fallback` when nothing is left of it.
fn ident(s: &str, sep: char, fallback: &str) -> String {
    let mut out = String::new();
    for c in s.chars() {
        if c.is_alphanumeric() {
            out.extend(c.to_lowercase());
        } else if !out.is_empty() && !out.ends_with(sep) {
            out.push(sep);
        }
    }
    let out = out.trim_end_matches(sep);
    match out.chars().next() {
        None => fallback.to_string(),
        Some(c) if c.is_ascii_digit() => format!("p_{out}"), // identifiers cannot start with a digit
        Some(_) => out.to_string(),
    }
}

/// `program` is an executable file in one of the `PATH` directories.
fn on_path(program: &str) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::env::var_os("PATH").is_some_and(|paths| {
        std::env::split_paths(&paths).any(|dir| std::fs::metadata(dir.join(program)).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0))
    })
}

// ---- file contents ----

const PY_PROJECT: &str = r##"[build-system]
requires = ["setuptools>=68"]
build-backend = "setuptools.build_meta"

[project]
name = "{slug}"
version = "0.1.0"
requires-python = ">=3.10"
dependencies = []

[tool.setuptools.packages.find]
where = ["src"]
"##;

const PY_MAIN: &str = r##"def main() -> None:
    print("Hello, world!")


if __name__ == "__main__":
    main()
"##;

const JS_MAIN: &str = "console.log(\"Hello, world!\");\n";

const TS_CONFIG: &str = r##"{
  "compilerOptions": {
    "target": "ES2022",
    "module": "NodeNext",
    "moduleResolution": "NodeNext",
    "rootDir": "src",
    "outDir": "dist",
    "strict": true,
    "esModuleInterop": true,
    "skipLibCheck": true
  },
  "include": ["src"]
}
"##;

const TS_MAIN: &str = "const greeting: string = \"Hello, world!\";\nconsole.log(greeting);\n";

const GO_MAIN: &str = r##"package main

import "fmt"

func main() {
	fmt.Println("Hello, world!")
}
"##;

const JAVA_MAIN: &str = r##"package {package};

public class Main {
    public static void main(String[] args) {
        System.out.println("Hello, world!");
    }
}
"##;

const JAVA_POM: &str = r##"<?xml version="1.0" encoding="UTF-8"?>
<project xmlns="http://maven.apache.org/POM/4.0.0"
         xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance"
         xsi:schemaLocation="http://maven.apache.org/POM/4.0.0 https://maven.apache.org/xsd/maven-4.0.0.xsd">
  <modelVersion>4.0.0</modelVersion>

  <groupId>{package}</groupId>
  <artifactId>{slug}</artifactId>
  <version>0.1.0-SNAPSHOT</version>
  <packaging>jar</packaging>

  <properties>
    <maven.compiler.release>21</maven.compiler.release>
    <project.build.sourceEncoding>UTF-8</project.build.sourceEncoding>
  </properties>

  <dependencies>
    <dependency>
      <groupId>org.junit.jupiter</groupId>
      <artifactId>junit-jupiter</artifactId>
      <version>5.11.4</version>
      <scope>test</scope>
    </dependency>
  </dependencies>

  <build>
    <plugins>
      <plugin>
        <groupId>org.apache.maven.plugins</groupId>
        <artifactId>maven-surefire-plugin</artifactId>
        <version>3.5.2</version>
      </plugin>
    </plugins>
  </build>
</project>
"##;

const JAVA_GRADLE: &str = r##"plugins {
    id 'java'
    id 'application'
}

repositories {
    mavenCentral()
}

dependencies {
    testImplementation 'org.junit.jupiter:junit-jupiter:5.11.4'
    testRuntimeOnly 'org.junit.platform:junit-platform-launcher'
}

java {
    toolchain {
        languageVersion = JavaLanguageVersion.of(21)
    }
}

application {
    mainClass = '{package}.Main'
}

tasks.named('test') {
    useJUnitPlatform()
}
"##;

const JAVA_GRADLE_KTS: &str = r##"plugins {
    java
    application
}

repositories {
    mavenCentral()
}

dependencies {
    testImplementation("org.junit.jupiter:junit-jupiter:5.11.4")
    testRuntimeOnly("org.junit.platform:junit-platform-launcher")
}

java {
    toolchain {
        languageVersion = JavaLanguageVersion.of(21)
    }
}

application {
    mainClass = "{package}.Main"
}

tasks.test {
    useJUnitPlatform()
}
"##;

const KOTLIN_GRADLE: &str = r##"plugins {
    kotlin("jvm") version "2.1.0"
    application
}

repositories {
    mavenCentral()
}

dependencies {
    testImplementation(kotlin("test"))
}

kotlin {
    jvmToolchain(21)
}

application {
    mainClass = "{package}.MainKt"
}

tasks.test {
    useJUnitPlatform()
}
"##;

const CSHARP_EXE: &str = r##"<Project Sdk="Microsoft.NET.Sdk">

  <PropertyGroup>
    <OutputType>Exe</OutputType>
    <TargetFramework>net8.0</TargetFramework>
    <RootNamespace>{package}</RootNamespace>
    <AssemblyName>{slug}</AssemblyName>
    <ImplicitUsings>enable</ImplicitUsings>
    <Nullable>enable</Nullable>
  </PropertyGroup>

</Project>
"##;

const CSHARP_LIB: &str = r##"<Project Sdk="Microsoft.NET.Sdk">

  <PropertyGroup>
    <TargetFramework>net8.0</TargetFramework>
    <RootNamespace>{package}</RootNamespace>
    <AssemblyName>{slug}</AssemblyName>
    <ImplicitUsings>enable</ImplicitUsings>
    <Nullable>enable</Nullable>
  </PropertyGroup>

</Project>
"##;

const CSHARP_MAIN: &str = "namespace {package};\n\npublic static class Program\n{\n    public static void Main()\n    {\n        Console.WriteLine(\"Hello, world!\");\n    }\n}\n";

const CSHARP_CLASS: &str = "namespace {package};\n\npublic class Class1\n{\n}\n";

const KOTLIN_MAIN: &str = "package {package}\n\nfun main() {\n    println(\"Hello, world!\")\n}\n";

const C_MAKEFILE: &str = "\
CC      ?= cc
CFLAGS  ?= -Wall -Wextra -std=c11 -O2
TARGET  := {slug}
SRC     := $(wildcard src/*.c)
OBJ     := $(SRC:src/%.c=build/%.o)

all: $(TARGET)

$(TARGET): $(OBJ)
\t$(CC) $(CFLAGS) -o $@ $^

build/%.o: src/%.c | build
\t$(CC) $(CFLAGS) -Iinclude -c $< -o $@

build:
\tmkdir -p build

clean:
\trm -rf build $(TARGET)

.PHONY: all clean
";

const C_MAIN: &str = "#include <stdio.h>\n\nint main(void) {\n    puts(\"Hello, world!\");\n    return 0;\n}\n";

const CPP_CMAKE: &str = r##"cmake_minimum_required(VERSION 3.16)
project({slug} LANGUAGES CXX)

set(CMAKE_CXX_STANDARD 20)
set(CMAKE_CXX_STANDARD_REQUIRED ON)
set(CMAKE_EXPORT_COMPILE_COMMANDS ON)

add_executable({slug} src/main.cpp)
target_include_directories({slug} PRIVATE include)
"##;

const CPP_MAKEFILE: &str = "\
CXX      ?= c++
CXXFLAGS ?= -Wall -Wextra -std=c++20 -O2
TARGET   := {slug}
SRC      := $(wildcard src/*.cpp)
OBJ      := $(SRC:src/%.cpp=build/%.o)

all: $(TARGET)

$(TARGET): $(OBJ)
\t$(CXX) $(CXXFLAGS) -o $@ $^

build/%.o: src/%.cpp | build
\t$(CXX) $(CXXFLAGS) -Iinclude -c $< -o $@

build:
\tmkdir -p build

clean:
\trm -rf build $(TARGET)

.PHONY: all clean
";

const CPP_MAIN: &str = "#include <iostream>\n\nint main() {\n    std::cout << \"Hello, world!\\n\";\n    return 0;\n}\n";

const PHP_MAIN: &str = "<?php\n\ndeclare(strict_types=1);\n\necho \"Hello, world!\\n\";\n";

const RUBY_MAIN: &str = "# frozen_string_literal: true\n\nputs 'Hello, world!'\n";

const LUA_MAIN: &str = "print(\"Hello, world!\")\n";

const SCALA_SBT: &str = r##"ThisBuild / scalaVersion := "3.3.4"
ThisBuild / version      := "0.1.0"

lazy val root = (project in file("."))
  .settings(
    name := "{slug}",
    libraryDependencies += "org.scalameta" %% "munit" % "1.0.3" % Test
  )
"##;

const SCALA_MAIN: &str = "package {package}\n\n@main def main(): Unit =\n  println(\"Hello, world!\")\n";

const R_MAIN: &str = "cat(\"Hello, world!\\n\")\n";

const JULIA_PROJECT: &str = "name = \"{pkg}\"\nversion = \"0.1.0\"\n";

const JULIA_MAIN: &str = "module {pkg}\n\ngreet() = println(\"Hello, world!\")\n\nend # module\n";

const SHELL_MAIN: &str = "#!/usr/bin/env bash\nset -euo pipefail\n\necho \"Hello, world!\"\n";

const NIM_NIMBLE: &str = r##"version       = "0.1.0"
author        = "{user}"
description   = "{slug}"
license       = "MIT"
srcDir        = "src"
bin           = @["{pkg}"]

requires "nim >= 2.0.0"
"##;

const NIM_MAIN: &str = "echo \"Hello, world!\"\n";

const PERL_MAIN: &str = "#!/usr/bin/env perl\nuse strict;\nuse warnings;\n\nprint \"Hello, world!\\n\";\n";

const CLJ_DEPS: &str = "{:paths [\"src\"]\n :deps {org.clojure/clojure {:mvn/version \"1.12.0\"}}\n :aliases {:test {:extra-paths [\"test\"]}}}\n";

const CLJ_MAIN: &str = "(ns {pkg}.core)\n\n(defn -main [& _args]\n  (println \"Hello, world!\"))\n";
