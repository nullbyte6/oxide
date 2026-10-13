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
pub struct Variant {
    pub name: &'static str,
    /// What it does in a few words, shown in the menu.
    pub note: &'static str,
    pub steps: &'static [Step],
}

pub struct Lang {
    pub name: &'static str,
    /// Selects the `.gitignore` template (see `ignore::gitignore`).
    pub key: &'static str,
    /// Never empty; the first one is the default.
    pub variants: &'static [Variant],
}

const fn v(name: &'static str, note: &'static str, steps: &'static [Step]) -> Variant {
    Variant { name, note, steps }
}

/// The languages in the menu, most popular first.
pub const LANGS: &[Lang] = &[
    Lang {
        name: "Rust",
        key: "rust",
        variants: &[
            v("Binary", "cargo init", &[Run("cargo init --name {pkg}")]),
            v("Library", "cargo init --lib", &[Run("cargo init --lib --name {pkg}")]),
        ],
    },
    Lang {
        name: "Python",
        key: "python",
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
        variants: &[
            v("npm", "npm init -y, src/", &[Run("npm init -y"), File("src/index.js", JS_MAIN)]),
            v("pnpm", "pnpm init, src/", &[Run("pnpm init"), File("src/index.js", JS_MAIN)]),
            v("Bun", "bun init -y", &[Run("bun init -y")]),
        ],
    },
    Lang {
        name: "TypeScript",
        key: "node",
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
        variants: &[v("Module", "go mod init, cmd/ and internal/", &[
            Run("go mod init {slug}"),
            File("cmd/{pkg}/main.go", GO_MAIN),
            Dir("internal"),
        ])],
    },
    Lang {
        name: "Java",
        key: "java",
        variants: &[
            v("Maven", "pom.xml, src/main/java, src/test/java", &[
                File("pom.xml", JAVA_POM),
                File("src/main/java/com/example/Main.java", JAVA_MAIN),
                Dir("src/test/java/com/example"),
            ]),
            v("Gradle (Groovy)", "build.gradle, settings.gradle", &[
                File("settings.gradle", "rootProject.name = '{slug}'\n"),
                File("build.gradle", JAVA_GRADLE),
                File("src/main/java/com/example/Main.java", JAVA_MAIN),
                Dir("src/test/java/com/example"),
            ]),
            v("Gradle (Kotlin DSL)", "build.gradle.kts, settings.gradle.kts", &[
                File("settings.gradle.kts", "rootProject.name = \"{slug}\"\n"),
                File("build.gradle.kts", JAVA_GRADLE_KTS),
                File("src/main/java/com/example/Main.java", JAVA_MAIN),
                Dir("src/test/java/com/example"),
            ]),
        ],
    },
    Lang {
        name: "Kotlin",
        key: "kotlin",
        variants: &[v("Gradle (Kotlin DSL)", "build.gradle.kts, src/main/kotlin", &[
            File("settings.gradle.kts", "rootProject.name = \"{slug}\"\n"),
            File("build.gradle.kts", KOTLIN_GRADLE),
            File("src/main/kotlin/Main.kt", KOTLIN_MAIN),
            Dir("src/test/kotlin"),
        ])],
    },
    Lang {
        name: "C",
        key: "c",
        variants: &[v("Makefile", "Makefile, src/, include/", &[
            File("Makefile", C_MAKEFILE),
            File("src/main.c", C_MAIN),
            Dir("include"),
        ])],
    },
    Lang {
        name: "C++",
        key: "cpp",
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
        variants: &[
            v("Console", "dotnet new console", &[Run("dotnet new console --name {slug} --output .")]),
            v("Class library", "dotnet new classlib", &[Run("dotnet new classlib --name {slug} --output .")]),
        ],
    },
    Lang {
        name: "PHP",
        key: "php",
        variants: &[v("Composer", "composer init, src/", &[
            Run("composer init --no-interaction --name={user}/{pkg}"),
            File("src/index.php", PHP_MAIN),
        ])],
    },
    Lang {
        name: "Ruby",
        key: "ruby",
        variants: &[v("Bundler", "bundle init, lib/", &[
            Run("bundle init"),
            File("lib/{pkg}.rb", RUBY_MAIN),
            Dir("spec"),
        ])],
    },
    Lang {
        name: "Swift",
        key: "swift",
        variants: &[
            v("Executable", "swift package init", &[Run("swift package init --type executable --name {pkg}")]),
            v("Library", "swift package init --type library", &[Run("swift package init --type library --name {pkg}")]),
        ],
    },
    Lang {
        name: "Dart",
        key: "dart",
        variants: &[v("Console", "dart create", &[Run("dart create --force --template console .")])],
    },
    Lang {
        name: "Lua",
        key: "lua",
        variants: &[v("Plain", "src/main.lua, tests/", &[File("src/main.lua", LUA_MAIN), Dir("tests")])],
    },
    Lang {
        name: "Zig",
        key: "zig",
        variants: &[v("zig init", "zig init", &[Run("zig init")])],
    },
    Lang {
        name: "Elixir",
        key: "elixir",
        variants: &[v("Mix", "mix new", &[Run("mix new . --app {pkg}")])],
    },
    Lang {
        name: "Haskell",
        key: "haskell",
        variants: &[v("Cabal", "cabal init, app/", &[Run("cabal init --non-interactive --exe --package-name={slug}")])],
    },
    Lang {
        name: "Scala",
        key: "scala",
        variants: &[v("sbt", "build.sbt, src/main/scala", &[
            File("build.sbt", SCALA_SBT),
            File("project/build.properties", "sbt.version=1.10.7\n"),
            File("src/main/scala/Main.scala", SCALA_MAIN),
            Dir("src/test/scala"),
        ])],
    },
    Lang {
        name: "R",
        key: "r",
        variants: &[v("Scripts", "R/, data/, main.R", &[File("main.R", R_MAIN), Dir("R"), Dir("data")])],
    },
    Lang {
        name: "Julia",
        key: "julia",
        variants: &[v("Package", "Project.toml, src/, test/", &[
            File("Project.toml", JULIA_PROJECT),
            File("src/{pkg}.jl", JULIA_MAIN),
            Dir("test"),
        ])],
    },
    Lang {
        name: "Shell",
        key: "shell",
        variants: &[v("Script", "bin/ script, lib/", &[
            File("bin/{slug}", SHELL_MAIN),
            Run("chmod +x bin/{slug}"),
            Dir("lib"),
        ])],
    },
    Lang {
        name: "Nim",
        key: "nim",
        variants: &[v("Nimble", "nimble file, src/", &[
            File("{slug}.nimble", NIM_NIMBLE),
            File("src/{pkg}.nim", NIM_MAIN),
            Dir("tests"),
        ])],
    },
    Lang {
        name: "Perl",
        key: "perl",
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
    pub fn build(root: &Path, lang: &Lang, variant: &Variant, license: Option<&License>, holder: &str, year: i64) -> Plan {
        let name = project_name(root);
        let (pkg, slug, user) = (ident(&name, '_', "app"), ident(&name, '-', "app"), ident(holder, '_', "app"));
        let fill = |s: &str| s.replace("{name}", &name).replace("{pkg}", &pkg).replace("{slug}", &slug).replace("{user}", &user);

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
                    // An empty directory would not exist in Git, so it gets a marker file, unless a file lands in it.
                    let path = fill(path);
                    let prefix = format!("{path}/");
                    if !variant.steps.iter().any(|s| matches!(s, File(f, _) if fill(f).starts_with(&prefix))) {
                        actions.push(Action::File { path: format!("{path}/.gitkeep"), text: String::new(), mode: Write::IfMissing });
                    }
                }
                File(path, text) => actions.push(Action::File { path: fill(path), text: fill(text), mode: Write::Always }),
            }
        }
        actions.push(Action::File { path: ".gitignore".to_string(), text: ignore::gitignore(lang.key), mode: Write::Merge });
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

/// A repository can be set up when nothing is in it but Git's own `.git` (a folder, or a file in worktrees).
pub fn is_empty(root: &Path) -> bool {
    std::fs::read_dir(root).is_ok_and(|mut entries| entries.all(|e| e.is_ok_and(|e| e.file_name() == ".git")))
}

/// The name of the project in `root`: the name of its directory.
fn project_name(root: &Path) -> String {
    root.file_name().map_or_else(|| "project".to_string(), |n| n.to_string_lossy().into_owned())
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

const JAVA_MAIN: &str = r##"package com.example;

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

  <groupId>com.example</groupId>
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
    mainClass = 'com.example.Main'
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
    mainClass = "com.example.Main"
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
    mainClass = "MainKt"
}

tasks.test {
    useJUnitPlatform()
}
"##;

const KOTLIN_MAIN: &str = "fun main() {\n    println(\"Hello, world!\")\n}\n";

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

const SCALA_MAIN: &str = "@main def main(): Unit =\n  println(\"Hello, world!\")\n";

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
