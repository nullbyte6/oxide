//! `.gitignore` templates for new projects, one per language family, in the spirit of GitHub's
//! gitignore collection but kept short: build output, dependency folders and tool caches.

/// Ignored in every project: editor swap files and the operating system's folder metadata.
const COMMON: &str = "\
# Editors and operating system
.idea/
.vscode/
*.swp
*~
.DS_Store
";

const RUST: &str = "\
# Build output
/target

# Backup files made by rustfmt
**/*.rs.bk
";

const PYTHON: &str = "\
# Bytecode and caches
__pycache__/
*.py[cod]
.pytest_cache/
.mypy_cache/
.ruff_cache/

# Virtual environments
.venv/
venv/
env/

# Packaging
build/
dist/
*.egg-info/
.eggs/

# Environment files
.env
";

const NODE: &str = "\
# Dependencies
node_modules/

# Build output
dist/
build/
out/
*.tsbuildinfo

# Logs and caches
npm-debug.log*
yarn-debug.log*
yarn-error.log*
.npm/
.eslintcache
.cache/

# Environment files
.env
.env.local
";

const GO: &str = "\
# Binaries
/bin/
*.exe
*.test
*.out

# Workspace and vendoring
go.work
vendor/

# Coverage
*.coverprofile
";

const JAVA: &str = "\
# Build output
target/
build/
out/
*.class
*.jar
*.war

# Gradle
.gradle/
!gradle/wrapper/gradle-wrapper.jar

# Logs
*.log
hs_err_pid*
";

const KOTLIN: &str = "\
# Build output
build/
out/
*.class
*.jar

# Gradle
.gradle/
!gradle/wrapper/gradle-wrapper.jar

# Kotlin
.kotlin/

# Logs
*.log
";

const C: &str = "\
# Object files and libraries
*.o
*.obj
*.a
*.so
*.so.*
*.dylib
*.dll
*.lib

# Executables and build output
*.exe
*.out
/build/
/bin/

# Dependency files and debug data
*.d
*.dSYM/
core
";

const CPP: &str = "\
# Object files and libraries
*.o
*.obj
*.a
*.so
*.so.*
*.dylib
*.dll
*.lib

# Executables and build output
*.exe
*.out
/build/
/bin/
/out/

# CMake
CMakeCache.txt
CMakeFiles/
cmake_install.cmake
compile_commands.json

# Dependency files and debug data
*.d
*.dSYM/
core
";

const CSHARP: &str = "\
# Build output
bin/
obj/
out/
[Dd]ebug/
[Rr]elease/

# NuGet
*.nupkg
packages/

# Visual Studio
.vs/
*.user
*.suo
";

const PHP: &str = "\
# Dependencies
/vendor/

# Environment files
.env
.env.*

# Caches and logs
.phpunit.result.cache
.php-cs-fixer.cache
*.log
";

const RUBY: &str = "\
# Gems and bundler
.bundle/
vendor/bundle/
*.gem

# Build and docs
/pkg/
/doc/
/.yardoc/

# Coverage and logs
/coverage/
*.log

# Environment files
.env
";

const SWIFT: &str = "\
# Swift Package Manager
.build/
.swiftpm/
Packages/
*.xcodeproj/xcuserdata/
xcuserdata/
DerivedData/
";

const DART: &str = "\
# Dart and Pub
.dart_tool/
.packages
pubspec.lock
build/
doc/api/

# Coverage
coverage/
";

const LUA: &str = "\
# Compiled chunks and libraries
*.luac
*.o
*.so
*.dll

# LuaRocks
/lua_modules/
/.luarocks/
*.rock
";

const ZIG: &str = "\
# Build output and cache
.zig-cache/
zig-cache/
zig-out/
";

const ELIXIR: &str = "\
# Build output and dependencies
/_build/
/deps/
/cover/
/doc/

# Compiled and generated files
*.beam
*.ez
erl_crash.dump
/tmp/

# Environment files
.env
";

const HASKELL: &str = "\
# Cabal and Stack
dist/
dist-newstyle/
.stack-work/
cabal.project.local*
.ghc.environment.*

# Compiled files
*.o
*.hi
*.dyn_o
*.dyn_hi
*.prof
*.hp
";

const SCALA: &str = "\
# sbt and build output
target/
project/target/
project/project/
.bsp/

# Metals and Bloop
.bloop/
.metals/
metals.sbt

# Compiled files
*.class
*.log
";

const R: &str = "\
# R session data and history
.Rhistory
.Rapp.history
.RData
.Rproj.user/
.Ruserdata

# Knitr and R Markdown output
*_cache/
*_files/
/renv/library/
/renv/staging/
";

const JULIA: &str = "\
# Compiled files and dependency resolution
*.jl.cov
*.jl.*.cov
*.jl.mem
Manifest.toml

# Documentation build
/docs/build/
/docs/site/
";

const SHELL: &str = "\
# Logs and temporary files
*.log
*.tmp
*.bak

# Environment files
.env
";

const NIM: &str = "\
# Compiled output and cache
nimcache/
nimblecache/
htmldocs/
/bin/

# Executables
*.exe
*.out
";

const PERL: &str = "\
# Build output
blib/
_build/
Build
Build.bat
Makefile
Makefile.old
pm_to_blib
MYMETA.*

# Dependencies and distribution
local/
/cover_db/
*.tar.gz
*.bak
";

const CLOJURE: &str = "\
# Build output
/target/
/classes/
/checkouts/
*.jar
*.class

# Tooling caches
.cpcache/
.clj-kondo/.cache/
.lsp/.cache/
.nrepl-port
.lein-*
pom.xml.asc
";

/// The `.gitignore` for the language with this `key` (see `scaffold::LANGS`), followed by the common part.
/// Unknown keys get only the common part.
pub fn gitignore(key: &str) -> String {
    let body = match key {
        "rust" => RUST,
        "python" => PYTHON,
        "node" => NODE,
        "go" => GO,
        "java" => JAVA,
        "kotlin" => KOTLIN,
        "c" => C,
        "cpp" => CPP,
        "csharp" => CSHARP,
        "php" => PHP,
        "ruby" => RUBY,
        "swift" => SWIFT,
        "dart" => DART,
        "lua" => LUA,
        "zig" => ZIG,
        "elixir" => ELIXIR,
        "haskell" => HASKELL,
        "scala" => SCALA,
        "r" => R,
        "julia" => JULIA,
        "shell" => SHELL,
        "nim" => NIM,
        "perl" => PERL,
        "clojure" => CLOJURE,
        _ => "",
    };
    if body.is_empty() { COMMON.to_string() } else { format!("{body}\n{COMMON}") }
}
