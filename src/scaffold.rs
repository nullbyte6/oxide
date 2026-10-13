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
    /// Commands (like `cargo build`) that prove the finished project works, run one after another inside it by
    /// `oxide --check-setups`. They may use the same placeholders as steps. Empty: the setup itself is the check.
    pub check: &'static [&'static str],
}

impl Variant {
    /// This variant with the commands that check it.
    const fn check(mut self, commands: &'static [&'static str]) -> Variant {
        self.check = commands;
        self
    }
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
    /// The name Python code is imported by: one identifier, which becomes the folder under `src/`.
    Importable,
    /// A library name in lowercase_snake_case that names the files of `lib/` (Ruby, Dart).
    Snake,
    /// A name in PascalCase that is also the module name (Swift).
    Pascal,
    /// A Cabal package name: lowercase words joined by hyphens (Haskell).
    Hyphen,
    /// A Perl module name with `::` between parts (`My::App`), which becomes `lib/My/App.pm`.
    PerlModule,
    /// An R package name: letters, digits and dots.
    RPackage,
    /// A Clojure namespace: lowercase words joined by hyphens, parts joined by dots (`my-app.core`).
    Clojure,
    /// A PHP namespace, parts separated by backslashes (`Acme\\App`), mapped to `src/` by PSR-4.
    PhpNamespace,
}

impl PackageKind {
    /// Name of the menu page and of the field on it.
    pub fn noun(self) -> &'static str {
        match self {
            PackageKind::Packages | PackageKind::Importable | PackageKind::Snake | PackageKind::Pascal | PackageKind::Hyphen | PackageKind::RPackage => "package",
            PackageKind::Module => "module",
            PackageKind::Namespace | PackageKind::PhpNamespace | PackageKind::Clojure => "namespace",
            PackageKind::PerlModule => "module",
        }
    }

    /// Only one name is possible (everything but Java-style packages).
    fn single(self) -> bool {
        self != PackageKind::Packages
    }

    /// What a valid name is made of, for the error message.
    fn rule(self) -> &'static str {
        match self {
            PackageKind::Packages | PackageKind::Namespace => "letters, digits, _ and dots; no leading digit",
            PackageKind::Importable => "letters, digits and _; no leading digit",
            PackageKind::Snake => "lowercase letters, digits and _, starting with a letter",
            PackageKind::Pascal => "letters, digits and _, starting with a capital letter",
            PackageKind::Hyphen => "lowercase letters, digits and -, words joined by -, each with a letter",
            PackageKind::PerlModule => "letters, digits and _ between ::; no leading digit",
            PackageKind::RPackage => "letters, digits and dots; at least 2 characters, starting with a letter, not ending in a dot",
            PackageKind::Clojure => "lowercase letters, digits and - in parts joined by dots, each starting with a letter",
            PackageKind::PhpNamespace => "letters, digits and _ between \\; no leading digit",
            PackageKind::Module => "letters, digits and . _ ~ - between slashes",
        }
    }

    /// The line above the field.
    pub fn help(self) -> &'static str {
        match self {
            PackageKind::Packages => "Package names, separated by commas; the first one holds the main code.",
            PackageKind::Module => "Module path of go.mod, e.g. github.com/you/app.",
            PackageKind::Namespace => "Root namespace of the project, e.g. Company.App.",
            PackageKind::Importable => "Name the code is imported by, e.g. my_app; it becomes the folder under src/.",
            PackageKind::Snake => "lowercase_snake_case name, e.g. my_app; it names the main source file.",
            PackageKind::Pascal => "PascalCase name, e.g. MyApp; it names the module and its files.",
            PackageKind::Hyphen => "Cabal package name: lowercase words joined by hyphens, e.g. my-app.",
            PackageKind::PerlModule => "Module name with ::, e.g. My::App; it becomes lib/My/App.pm.",
            PackageKind::RPackage => "R package name, e.g. mypkg: letters, digits and dots.",
            PackageKind::Clojure => "Main namespace, e.g. my-app.core; it becomes src/my_app/core.clj.",
            PackageKind::PhpNamespace => "Root namespace with backslashes, e.g. Acme\\App (PSR-4, mapped to src/).",
        }
    }
}

/// The question a package-like language asks, with its suggested answer. The suggestion may use `{pkg}`,
/// `{slug}`, `{dotted}` (`my.app`), `{Pascal}` (`MyApp`), `{user}` and `{PascalUser}` (the owner's name in PascalCase).
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
    Variant { name, note, steps, check: &[] }
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
            v("Binary", "cargo init", &[Run("cargo init --name {pkg}")]).check(&["cargo build"]),
            v("Library", "cargo init --lib", &[Run("cargo init --lib --name {pkg}")]).check(&["cargo build"]),
        ],
    },
    Lang {
        name: "Python",
        key: "python",
        ignore: "",
        package: Some(Package { kind: PackageKind::Importable, suggest: "{pkg}" }),
        variants: &[
            v("venv + pip", "src/ layout, pyproject.toml, .venv", &[
                Run("python3 -m venv .venv"),
                File("pyproject.toml", PY_PROJECT),
                File("src/{package}/__init__.py", ""),
                File("src/{package}/main.py", PY_MAIN),
                Dir("tests"),
            ]).check(&["python3 -m compileall -q src"]),
            v("uv", "uv init --package", &[Run("uv init --package --name {package}"), Dir("tests")]).check(&["python3 -m compileall -q src"]),
        ],
    },
    Lang {
        name: "JavaScript",
        key: "node",
        ignore: "",
        package: None,
        variants: &[
            v("npm", "npm init -y, src/", &[Run("npm init -y"), File("src/index.js", JS_MAIN)]).check(&["node src/index.js"]),
            v("pnpm", "pnpm init, src/", &[Run("pnpm init"), File("src/index.js", JS_MAIN)]).check(&["node src/index.js"]),
            v("Bun", "bun init -y", &[Run("bun init -y")]).check(&["bun run index.ts"]),
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
            ]).check(&["npx tsc --noEmit"]),
            v("Bun", "bun init -y (TypeScript by default)", &[Run("bun init -y")]).check(&["bun run index.ts"]),
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
        ]).check(&["go build ./..."])],
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
            ]).check(&["mvn -q -B compile"]),
            v("Gradle (Groovy)", "build.gradle, settings.gradle", &[
                File("settings.gradle", "rootProject.name = '{slug}'\n"),
                File("build.gradle", JAVA_GRADLE),
                File("src/main/java/{package_path}/Main.java", JAVA_MAIN),
                Dir("src/main/java/{package_path}"),
                Dir("src/test/java/{package_path}"),
            ]).check(&["gradle -q --no-daemon compileJava"]),
            v("Gradle (Kotlin DSL)", "build.gradle.kts, settings.gradle.kts", &[
                File("settings.gradle.kts", "rootProject.name = \"{slug}\"\n"),
                File("build.gradle.kts", JAVA_GRADLE_KTS),
                File("src/main/java/{package_path}/Main.java", JAVA_MAIN),
                Dir("src/main/java/{package_path}"),
                Dir("src/test/java/{package_path}"),
            ]).check(&["gradle -q --no-daemon compileJava"]),
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
        ]).check(&["gradle -q --no-daemon compileKotlin"])],
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
        ]).check(&["make"])],
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
            ]).check(&["cmake -S . -B build", "cmake --build build"]),
            v("Makefile", "Makefile, src/, include/", &[
                File("Makefile", CPP_MAKEFILE),
                File("src/main.cpp", CPP_MAIN),
                Dir("include"),
            ]).check(&["make"]),
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
            ]).check(&["dotnet build"]),
            v("Class library", "{slug}.csproj, Class1.cs", &[
                File("{slug}.csproj", CSHARP_LIB),
                File("src/Class1.cs", CSHARP_CLASS),
            ]).check(&["dotnet build"]),
        ],
    },
    Lang {
        name: "PHP",
        key: "php",
        ignore: "",
        package: Some(Package { kind: PackageKind::PhpNamespace, suggest: "{PascalUser}\\{Pascal}" }),
        variants: &[v("Composer", "composer.json (PSR-4), src/, tests/", &[
            File("composer.json", PHP_COMPOSER),
            File("src/Greeter.php", PHP_CLASS),
            Dir("tests"),
        ]).check(&["php -l src/Greeter.php"])],
    },
    Lang {
        name: "Ruby",
        key: "ruby",
        ignore: "",
        package: Some(Package { kind: PackageKind::Snake, suggest: "{pkg}" }),
        variants: &[v("Bundler", "bundle init, lib/", &[
            Run("bundle init"),
            File("lib/{package}.rb", RUBY_MAIN),
            Dir("spec"),
        ]).check(&["ruby -c lib/{package}.rb"])],
    },
    Lang {
        name: "Swift",
        key: "swift",
        ignore: "",
        package: Some(Package { kind: PackageKind::Pascal, suggest: "{Pascal}" }),
        variants: &[
            v("Executable", "swift package init", &[Run("swift package init --type executable --name {package}")]).check(&["swift build"]),
            v("Library", "swift package init --type library", &[Run("swift package init --type library --name {package}")]).check(&["swift build"]),
        ],
    },
    Lang {
        name: "Dart",
        key: "dart",
        ignore: "",
        package: Some(Package { kind: PackageKind::Snake, suggest: "{pkg}" }),
        variants: &[v("Console", "pubspec.yaml, bin/, lib/, test/", &[
            File("pubspec.yaml", DART_PUBSPEC),
            File("bin/{package}.dart", DART_MAIN),
            File("lib/{package}.dart", DART_LIB),
            Dir("test"),
        ]).check(&["dart analyze"])],
    },
    Lang {
        name: "Lua",
        key: "lua",
        ignore: "",
        package: Some(Package { kind: PackageKind::Snake, suggest: "{pkg}" }),
        variants: &[v("Plain", "src/main.lua, src/<name>.lua, tests/", &[
            File("src/main.lua", LUA_MAIN),
            File("src/{package}.lua", LUA_MODULE),
            Dir("tests"),
        ]).check(&["lua src/main.lua"])],
    },
    Lang {
        name: "Zig",
        key: "zig",
        ignore: "",
        package: None,
        variants: &[v("zig init", "zig init", &[Run("zig init")]).check(&["zig build"])],
    },
    Lang {
        name: "Elixir",
        key: "elixir",
        ignore: "",
        package: Some(Package { kind: PackageKind::Snake, suggest: "{pkg}" }),
        variants: &[v("Mix", "mix new", &[Run("mix new . --app {package}")]).check(&["mix compile"])],
    },
    Lang {
        name: "Haskell",
        key: "haskell",
        ignore: "",
        package: Some(Package { kind: PackageKind::Hyphen, suggest: "{slug}" }),
        variants: &[v("Cabal", "cabal init, app/", &[Run("cabal init --non-interactive --exe --package-name={package}")]).check(&["cabal build"])],
    },
    Lang {
        name: "Scala",
        key: "scala",
        ignore: "",
        package: Some(Package { kind: PackageKind::Packages, suggest: "com.example.{pkg}" }),
        variants: &[v("sbt", "build.sbt, src/main/scala", &[
            File("build.sbt", SCALA_SBT),
            File("project/build.properties", "sbt.version={v_sbt}\n"),
            File("src/main/scala/{package_path}/Main.scala", SCALA_MAIN),
            Dir("src/main/scala/{package_path}"),
            Dir("src/test/scala/{package_path}"),
        ]).check(&["sbt -batch compile"])],
    },
    Lang {
        name: "R",
        key: "r",
        ignore: "",
        package: Some(Package { kind: PackageKind::RPackage, suggest: "{dotted}" }),
        variants: &[v("Package", "DESCRIPTION, NAMESPACE, R/", &[
            File("DESCRIPTION", R_DESCRIPTION),
            File("NAMESPACE", "exportPattern(\"^[[:alpha:]]+\")\n"),
            File("R/hello.R", R_MAIN),
        ]).check(&["Rscript R/hello.R"])],
    },
    Lang {
        name: "Julia",
        key: "julia",
        ignore: "",
        package: Some(Package { kind: PackageKind::Pascal, suggest: "{Pascal}" }),
        variants: &[v("Package", "Project.toml, src/, test/", &[
            File("Project.toml", JULIA_PROJECT),
            File("src/{package}.jl", JULIA_MAIN),
            Dir("test"),
        ]).check(&["julia --project src/{package}.jl"])],
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
        ]).check(&["bash bin/{slug}"])],
    },
    Lang {
        name: "Nim",
        key: "nim",
        ignore: "",
        package: Some(Package { kind: PackageKind::Snake, suggest: "{pkg}" }),
        variants: &[v("Nimble", "nimble file, src/", &[
            File("{package}.nimble", NIM_NIMBLE),
            File("src/{package}.nim", NIM_MAIN),
            Dir("tests"),
        ]).check(&["nim check src/{package}.nim"])],
    },
    Lang {
        name: "Perl",
        key: "perl",
        ignore: "",
        package: Some(Package { kind: PackageKind::PerlModule, suggest: "{Pascal}" }),
        variants: &[v("Plain", "script/, lib/, t/, cpanfile", &[
            File("script/main.pl", PERL_MAIN),
            File("lib/{package_path}.pm", PERL_MODULE),
            File("cpanfile", "# requires 'Some::Module';\n"),
            Dir("t"),
        ]).check(&["perl script/main.pl"])],
    },
    Lang {
        name: "Clojure",
        key: "clojure",
        ignore: "",
        package: Some(Package { kind: PackageKind::Clojure, suggest: "{slug}.core" }),
        variants: &[v("deps.edn", "deps.edn, src/, test/", &[
            File("deps.edn", CLJ_DEPS),
            File("src/{package_path}.clj", CLJ_MAIN),
            Dir("test"),
        ]).check(&["clojure -M:run"])],
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
        // `{package}` is the first package: the one that holds the main code. Directories that mention a
        // package are made for every package, so the others get their (empty) folders too.
        let fill_for = |s: &str, package: &str| fill(root, holder, package, s);
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
            .replace("{dotted}", &ident(&name, '.', "app"))
            .replace("{PascalUser}", &pascal(holder))
            .replace("{Pascal}", &pascal(&name))
            .replace("{user}", &ident(holder, '-', "user"))
    })
}

/// What was typed on the package page, checked for `kind`: for packages, names separated by commas (the first
/// holds the main code); for a module or namespace, exactly one. The error says what is wrong.
pub fn parse_packages(text: &str, kind: PackageKind) -> Result<Vec<String>, String> {
    // Identifiers joined by `sep`: letters, digits and `_`, none starting with a digit.
    let joined = |name: &str, sep: Option<&str>| {
        let mut parts: Box<dyn Iterator<Item = &str>> = match sep {
            Some(c) => Box::new(name.split(c)),
            None => Box::new(std::iter::once(name)),
        };
        parts.all(|part| {
            let mut chars = part.chars();
            chars.next().is_some_and(|c| c.is_ascii_alphabetic() || c == '_') && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
        })
    };
    let names: Vec<&str> = text.split(',').map(str::trim).filter(|n| !n.is_empty()).collect();
    let mut out: Vec<String> = Vec::new();
    for name in &names {
        let ok = match kind {
            PackageKind::Packages | PackageKind::Namespace => joined(name, Some(".")),
            PackageKind::PerlModule => joined(name, Some("::")),
            PackageKind::Importable => joined(name, None),
            PackageKind::RPackage => {
                name.len() >= 2 && name.starts_with(|c: char| c.is_ascii_alphabetic()) && !name.ends_with('.') && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '.')
            }
            PackageKind::Clojure => name.split('.').all(|part| {
                part.starts_with(|c: char| c.is_ascii_lowercase()) && !part.ends_with('-') && part.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
            }),
            PackageKind::Pascal => name.starts_with(|c: char| c.is_ascii_uppercase()) && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_'),
            PackageKind::Hyphen => {
                name.split('-').all(|word| word.chars().any(|c| c.is_ascii_lowercase()) && word.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit()))
            }
            PackageKind::Snake => name.starts_with(|c: char| c.is_ascii_lowercase()) && name.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_'),
            PackageKind::PhpNamespace => joined(name, Some("\\")),
            // A module path is a URL-like path: no spaces, no empty parts.
            PackageKind::Module => name.split('/').all(|part| !part.is_empty() && part.chars().all(|c| c.is_ascii_alphanumeric() || "._~-".contains(c))),
        };
        if !ok {
            return Err(format!("'{name}' is not a valid {} ({})", kind.noun(), kind.rule()));
        }
        if !out.iter().any(|p| p == name) {
            out.push((*name).to_string());
        }
    }
    match out.len() {
        0 => Err(format!("write a {} name", kind.noun())),
        n if n > 1 && kind.single() => Err(format!("only one {} is possible", kind.noun())),
        _ => Ok(out),
    }
}

/// The versions the templates pin, as `(name, default, what it is)`. A template writes one as `{v_name}`, and
/// `versions` in `config.lua` replaces the default, so no template needs editing when a version goes stale.
pub const PINS: &[(&str, &str, &str)] = &[
    ("java", "21", "Java release of Maven and Gradle projects, also the Kotlin JVM target"),
    ("junit", "5.11.4", "JUnit Jupiter"),
    ("surefire", "3.5.2", "Maven Surefire plugin"),
    ("kotlin", "2.1.0", "Kotlin Gradle plugin"),
    ("scala", "3.3.4", "Scala"),
    ("sbt", "1.10.7", "sbt"),
    ("munit", "1.0.3", "MUnit"),
    ("clojure", "1.12.0", "Clojure"),
    ("dotnet", "net8.0", "C# target framework"),
    ("php", "8.1", "minimum PHP"),
    ("python", "3.10", "minimum Python"),
    ("dart", "3.0.0", "minimum Dart SDK"),
    ("nim", "2.0.0", "minimum Nim"),
    ("c_std", "11", "C standard"),
    ("cpp_std", "20", "C++ standard"),
    ("cmake", "3.16", "minimum CMake"),
    // Generated GitHub workflows (src/workflow.rs): the version of each action, as the part after `@`...
    ("gha_checkout", "v4", "actions/checkout"),
    ("gha_cache", "v4", "actions/cache"),
    ("gha_upload", "v4", "actions/upload-artifact"),
    ("gha_download", "v4", "actions/download-artifact"),
    ("gha_release", "v2", "softprops/action-gh-release"),
    ("gha_node", "v4", "actions/setup-node"),
    ("gha_pnpm", "v4", "pnpm/action-setup"),
    ("gha_python", "v5", "actions/setup-python"),
    ("gha_go", "v5", "actions/setup-go"),
    ("gha_java", "v4", "actions/setup-java"),
    ("gha_gradle", "v4", "gradle/actions/setup-gradle"),
    ("gha_dotnet", "v4", "actions/setup-dotnet"),
    ("gha_ruby", "v1", "ruby/setup-ruby"),
    ("gha_php", "v2", "shivammathur/setup-php"),
    ("gha_beam", "v1", "erlef/setup-beam (Elixir)"),
    ("gha_zig", "v1", "mlugg/setup-zig"),
    ("gha_swift", "v2", "swift-actions/setup-swift"),
    ("gha_dart", "v1", "dart-lang/setup-dart"),
    ("gha_rust_cache", "v2", "Swatinem/rust-cache"),
    ("gha_rust_toolchain", "stable", "dtolnay/rust-toolchain (its ref is the channel or version)"),
    // ...and the toolchain versions a generated workflow asks them for (Java and .NET use `java` and `dotnet`).
    ("ci_node", "22", "Node.js in generated workflows"),
    ("ci_python", "3.12", "Python in generated workflows"),
    ("ci_ruby", "3.3", "Ruby in generated workflows"),
    ("ci_php", "8.3", "PHP in generated workflows"),
    ("ci_elixir", "1.17", "Elixir in generated workflows"),
    ("ci_otp", "27", "Erlang/OTP in generated workflows"),
    ("ci_gradle", "current", "Gradle when the project has no wrapper"),
];

/// Versions of `config.lua` that replace the defaults of `PINS`.
static PIN_OVERRIDES: OnceLock<Vec<(String, String)>> = OnceLock::new();

/// Makes the `versions` of `config.lua` count; call it once, before the first plan is made.
pub fn set_pins(overrides: Vec<(String, String)>) {
    let _ = PIN_OVERRIDES.set(overrides);
}

/// The version for pin `name`: the one from `config.lua`, else the default.
fn pin(name: &str, default: &str) -> String {
    PIN_OVERRIDES.get().and_then(|o| o.iter().find(|(k, _)| k == name)).map_or_else(|| default.to_string(), |(_, v)| v.clone())
}

/// The version of the pin `name` (see `PINS`) after `config.lua`, for code outside the language table.
pub fn pin_value(name: &str) -> String {
    PINS.iter().find(|(k, _, _)| *k == name).map_or_else(String::new, |(k, d, _)| pin(k, d))
}

/// Every placeholder a step may use, for `fill` and for checking the language table.
pub const PLACEHOLDERS: &[&str] =
    &["name", "slug", "pkg", "user", "Pascal", "package_path", "PackagePascal", "package_json", "package"];

/// `s` with the placeholders of a project in `root` owned by `holder` filled in; `package` is what `{package}` is.
pub fn fill(root: &Path, holder: &str, package: &str, s: &str) -> String {
    let name = project_name(root);
    let mut s = s.to_string();
    for (key, default, _) in PINS {
        let token = format!("{{v_{key}}}");
        if s.contains(&token) {
            s = s.replace(&token, &pin(key, default));
        }
    }
    s.replace("{name}", &name)
        .replace("{pkg}", &ident(&name, '_', "app"))
        .replace("{slug}", &ident(&name, '-', "app"))
        .replace("{user}", &ident(holder, '_', "app"))
        .replace("{Pascal}", &pascal(&name))
        .replace("{package_path}", &package.replace("::", "/").replace(['.', '\\'], "/").replace('-', "_"))
        .replace("{PackagePascal}", &pascal(package))
        .replace("{package_json}", &package.replace('\\', "\\\\"))
        .replace("{package}", package)
}

/// The names of the pins as placeholders (`v_java`).
pub fn pin_words() -> Vec<String> {
    PINS.iter().map(|(k, _, _)| format!("v_{k}")).collect()
}

/// The words that sit alone between braces in `text` (`{slug}`, not `{ a }` or `{:paths`).
pub fn braced_words(text: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut rest = text;
    while let Some(open) = rest.find('{') {
        rest = &rest[open + 1..];
        if let Some(close) = rest.find('}')
            && close > 0
            && rest[..close].chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
        {
            out.push(&rest[..close]);
        }
    }
    out
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
pub fn on_path(program: &str) -> bool {
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
requires-python = ">={v_python}"
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
    <maven.compiler.release>{v_java}</maven.compiler.release>
    <project.build.sourceEncoding>UTF-8</project.build.sourceEncoding>
  </properties>

  <dependencies>
    <dependency>
      <groupId>org.junit.jupiter</groupId>
      <artifactId>junit-jupiter</artifactId>
      <version>{v_junit}</version>
      <scope>test</scope>
    </dependency>
  </dependencies>

  <build>
    <plugins>
      <plugin>
        <groupId>org.apache.maven.plugins</groupId>
        <artifactId>maven-surefire-plugin</artifactId>
        <version>{v_surefire}</version>
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
    testImplementation 'org.junit.jupiter:junit-jupiter:{v_junit}'
    testRuntimeOnly 'org.junit.platform:junit-platform-launcher'
}

java {
    toolchain {
        languageVersion = JavaLanguageVersion.of({v_java})
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
    testImplementation("org.junit.jupiter:junit-jupiter:{v_junit}")
    testRuntimeOnly("org.junit.platform:junit-platform-launcher")
}

java {
    toolchain {
        languageVersion = JavaLanguageVersion.of({v_java})
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
    kotlin("jvm") version "{v_kotlin}"
    application
}

repositories {
    mavenCentral()
}

dependencies {
    testImplementation(kotlin("test"))
}

kotlin {
    jvmToolchain({v_java})
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
    <TargetFramework>{v_dotnet}</TargetFramework>
    <RootNamespace>{package}</RootNamespace>
    <AssemblyName>{slug}</AssemblyName>
    <ImplicitUsings>enable</ImplicitUsings>
    <Nullable>enable</Nullable>
  </PropertyGroup>

</Project>
"##;

const CSHARP_LIB: &str = r##"<Project Sdk="Microsoft.NET.Sdk">

  <PropertyGroup>
    <TargetFramework>{v_dotnet}</TargetFramework>
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
CFLAGS  ?= -Wall -Wextra -std=c{v_c_std} -O2
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

const CPP_CMAKE: &str = r##"cmake_minimum_required(VERSION {v_cmake})
project({slug} LANGUAGES CXX)

set(CMAKE_CXX_STANDARD {v_cpp_std})
set(CMAKE_CXX_STANDARD_REQUIRED ON)
set(CMAKE_EXPORT_COMPILE_COMMANDS ON)

add_executable({slug} src/main.cpp)
target_include_directories({slug} PRIVATE include)
"##;

const CPP_MAKEFILE: &str = "\
CXX      ?= c++
CXXFLAGS ?= -Wall -Wextra -std=c++{v_cpp_std} -O2
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

const PHP_COMPOSER: &str = r##"{
    "name": "{user}/{slug}",
    "description": "{slug}",
    "type": "project",
    "require": {
        "php": ">={v_php}"
    },
    "autoload": {
        "psr-4": {
            "{package_json}\\": "src/"
        }
    }
}
"##;

const PHP_CLASS: &str = "<?php\n\ndeclare(strict_types=1);\n\nnamespace {package};\n\nfinal class Greeter\n{\n    public function greet(): string\n    {\n        return 'Hello, world!';\n    }\n}\n";

const RUBY_MAIN: &str = "# frozen_string_literal: true\n\nmodule {PackagePascal}\n  def self.greet\n    'Hello, world!'\n  end\nend\n";

const DART_PUBSPEC: &str = "name: {package}\ndescription: {slug}\nversion: 0.1.0\n\nenvironment:\n  sdk: '>={v_dart} <5.0.0'\n";

const DART_MAIN: &str = "import 'package:{package}/{package}.dart';\n\nvoid main() {\n  print(greet());\n}\n";

const DART_LIB: &str = "String greet() => 'Hello, world!';\n";

const LUA_MAIN: &str = "-- Find the modules next to this file, wherever the script is run from.\nlocal dir = arg and arg[0] and arg[0]:match(\"^(.*)/[^/]*$\") or \".\"\npackage.path = dir .. \"/?.lua;\" .. package.path\n\nlocal {package} = require(\"{package}\")\n\nprint({package}.greet())\n";

const LUA_MODULE: &str = "local M = {}\n\nfunction M.greet()\n  return \"Hello, world!\"\nend\n\nreturn M\n";

const SCALA_SBT: &str = r##"ThisBuild / scalaVersion := "{v_scala}"
ThisBuild / version      := "0.1.0"

lazy val root = (project in file("."))
  .settings(
    name := "{slug}",
    libraryDependencies += "org.scalameta" %% "munit" % "{v_munit}" % Test
  )
"##;

const SCALA_MAIN: &str = "package {package}\n\n@main def main(): Unit =\n  println(\"Hello, world!\")\n";

const R_MAIN: &str = "hello <- function() {\n  \"Hello, world!\"\n}\n";

const R_DESCRIPTION: &str = "Package: {package}\nType: Package\nTitle: What the Package Does (Title Case)\nVersion: 0.1.0\nDescription: One paragraph describing what the package does.\nLicense: file LICENSE\nEncoding: UTF-8\n";

const JULIA_PROJECT: &str = "name = \"{package}\"\nversion = \"0.1.0\"\n";

const JULIA_MAIN: &str = "module {package}\n\ngreet() = println(\"Hello, world!\")\n\nend # module\n";

const SHELL_MAIN: &str = "#!/usr/bin/env bash\nset -euo pipefail\n\necho \"Hello, world!\"\n";

const NIM_NIMBLE: &str = r##"version       = "0.1.0"
author        = "{user}"
description   = "{slug}"
license       = "MIT"
srcDir        = "src"
bin           = @["{package}"]

requires "nim >= {v_nim}"
"##;

const NIM_MAIN: &str = "echo \"Hello, world!\"\n";

const PERL_MAIN: &str = "#!/usr/bin/env perl\nuse strict;\nuse warnings;\nuse FindBin qw($RealBin);\nuse lib \"$RealBin/../lib\";\nuse {package};\n\nprint {package}->greet, \"\\n\";\n";

const PERL_MODULE: &str = "package {package};\n\nuse strict;\nuse warnings;\n\nour $VERSION = '0.01';\n\nsub greet {\n    return 'Hello, world!';\n}\n\n1;\n";

const CLJ_DEPS: &str = "{:paths [\"src\"]\n :deps {org.clojure/clojure {:mvn/version \"{v_clojure}\"}}\n :aliases {:run {:main-opts [\"-m\" \"{package}\"]}\n           :test {:extra-paths [\"test\"]}}}\n";

const CLJ_MAIN: &str = "(ns {package})\n\n(defn -main [& _args]\n  (println \"Hello, world!\"))\n";
