//! New-project scaffolding: the languages the project menu offers and what setting each one up involves.
//!
//! A language has one or more variants (build tools, usually); a variant is a list of steps: commands to
//! run, directories to create and files to write. Anything a step says may use the placeholders
//! `{name}` (the directory name), `{pkg}` (the name as a lowercase identifier: `my-app` -> `my_app`) and
//! `{user}` (the Git user name as a lowercase identifier). Commands are split on spaces and run without a shell.

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
            Run("go mod init {name}"),
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
                File("settings.gradle", "rootProject.name = '{name}'\n"),
                File("build.gradle", JAVA_GRADLE),
                File("src/main/java/com/example/Main.java", JAVA_MAIN),
                Dir("src/test/java/com/example"),
            ]),
            v("Gradle (Kotlin DSL)", "build.gradle.kts, settings.gradle.kts", &[
                File("settings.gradle.kts", "rootProject.name = \"{name}\"\n"),
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
            File("settings.gradle.kts", "rootProject.name = \"{name}\"\n"),
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
            v("Console", "dotnet new console", &[Run("dotnet new console --name {name} --output .")]),
            v("Class library", "dotnet new classlib", &[Run("dotnet new classlib --name {name} --output .")]),
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
        variants: &[v("Cabal", "cabal init, app/", &[Run("cabal init --non-interactive --exe --package-name={name}")])],
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
        variants: &[v("Script", "bin/{name}, lib/", &[
            File("bin/{name}", SHELL_MAIN),
            Run("chmod +x bin/{name}"),
            Dir("lib"),
        ])],
    },
    Lang {
        name: "Nim",
        key: "nim",
        variants: &[v("Nimble", "{name}.nimble, src/", &[
            File("{name}.nimble", NIM_NIMBLE),
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

// ---- file contents ----

const PY_PROJECT: &str = r##"[build-system]
requires = ["setuptools>=68"]
build-backend = "setuptools.build_meta"

[project]
name = "{name}"
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
  <artifactId>{name}</artifactId>
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
TARGET  := {name}
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
project({name} LANGUAGES CXX)

set(CMAKE_CXX_STANDARD 20)
set(CMAKE_CXX_STANDARD_REQUIRED ON)
set(CMAKE_EXPORT_COMPILE_COMMANDS ON)

add_executable({name} src/main.cpp)
target_include_directories({name} PRIVATE include)
"##;

const CPP_MAKEFILE: &str = "\
CXX      ?= c++
CXXFLAGS ?= -Wall -Wextra -std=c++20 -O2
TARGET   := {name}
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
    name := "{name}",
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
description   = "{name}"
license       = "MIT"
srcDir        = "src"
bin           = @["{pkg}"]

requires "nim >= 2.0.0"
"##;

const NIM_MAIN: &str = "echo \"Hello, world!\"\n";

const PERL_MAIN: &str = "#!/usr/bin/env perl\nuse strict;\nuse warnings;\n\nprint \"Hello, world!\\n\";\n";

const CLJ_DEPS: &str = "{:paths [\"src\"]\n :deps {org.clojure/clojure {:mvn/version \"1.12.0\"}}\n :aliases {:test {:extra-paths [\"test\"]}}}\n";

const CLJ_MAIN: &str = "(ns {pkg}.core)\n\n(defn -main [& _args]\n  (println \"Hello, world!\"))\n";
