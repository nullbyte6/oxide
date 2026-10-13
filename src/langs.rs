//! Language statistics of a repository, fully local: a built-in table maps file extensions and a few file
//! names to a language and its GitHub colour, and the committed files' sizes (bytes) are summed per language.
//!
//! Like GitHub, only programming and markup languages count: prose (Markdown, text) and data formats
//! (JSON, YAML, TOML ...) are not in the table, and vendored or generated files are skipped.

use std::{collections::HashMap, sync::OnceLock};

pub struct Lang {
    pub name: &'static str,
    /// The colour GitHub shows for the language, as `0xRRGGBB`.
    pub color: u32,
    exts: &'static [&'static str],
    /// Exact file names that belong to the language without an extension.
    names: &'static [&'static str],
}

const fn l(name: &'static str, color: u32, exts: &'static [&'static str], names: &'static [&'static str]) -> Lang {
    Lang { name, color, exts, names }
}

#[rustfmt::skip]
const LANGS: &[Lang] = &[
    l("Rust", 0xdea584, &["rs"], &[]),
    l("Python", 0x3572a5, &["py", "pyw", "pyi", "pyx"], &[]),
    l("JavaScript", 0xf1e05a, &["js", "mjs", "cjs", "jsx"], &[]),
    l("TypeScript", 0x3178c6, &["ts", "tsx", "mts", "cts"], &[]),
    l("Go", 0x00add8, &["go"], &[]),
    l("C", 0x555555, &["c"], &[]),
    l("C++", 0xf34b7d, &["cpp", "cc", "cxx", "hpp", "hh", "hxx", "c++", "ipp", "tpp"], &[]),
    l("C#", 0x178600, &["cs", "csx"], &[]),
    l("Java", 0xb07219, &["java"], &[]),
    l("Kotlin", 0xa97bff, &["kt", "kts"], &[]),
    l("Swift", 0xf05138, &["swift"], &[]),
    l("Objective-C", 0x438eff, &["m", "mm"], &[]),
    l("Ruby", 0x701516, &["rb", "rake", "gemspec", "ru"], &["Rakefile", "Gemfile", "Vagrantfile", "Podfile"]),
    l("PHP", 0x4f5d95, &["php", "phtml"], &[]),
    l("Shell", 0x89e051, &["sh", "bash", "zsh", "fish", "ksh"], &[".bashrc", ".zshrc", ".profile", "PKGBUILD"]),
    l("Lua", 0x000080, &["lua"], &[]),
    l("Perl", 0x0298c3, &["pl", "pm", "t"], &[]),
    l("R", 0x198ce7, &["r"], &[]),
    l("Julia", 0xa270ba, &["jl"], &[]),
    l("Dart", 0x00b4ab, &["dart"], &[]),
    l("Scala", 0xc22d40, &["scala", "sc", "sbt"], &[]),
    l("Groovy", 0x4298b8, &["groovy", "gradle"], &[]),
    l("Clojure", 0xdb5855, &["clj", "cljs", "cljc", "edn"], &[]),
    l("Elixir", 0x6e4a7e, &["ex", "exs"], &[]),
    l("Erlang", 0xb83998, &["erl", "hrl"], &[]),
    l("Haskell", 0x5e5086, &["hs", "lhs"], &[]),
    l("OCaml", 0xef7a08, &["ml", "mli"], &[]),
    l("F#", 0xb845fc, &["fs", "fsi", "fsx"], &[]),
    l("Elm", 0x60b5cc, &["elm"], &[]),
    l("PureScript", 0x1d222d, &["purs"], &[]),
    l("Reason", 0xff5847, &["re", "rei"], &[]),
    l("Common Lisp", 0x3fb68b, &["lisp", "lsp", "cl"], &[]),
    l("Scheme", 0x1e4aec, &["scm", "ss"], &[]),
    l("Racket", 0x3c5caa, &["rkt"], &[]),
    l("Emacs Lisp", 0xc065db, &["el"], &[]),
    l("Vim Script", 0x199f4b, &["vim"], &[".vimrc"]),
    l("Zig", 0xec915c, &["zig"], &[]),
    l("Nim", 0xffc200, &["nim", "nims"], &[]),
    l("Crystal", 0x000100, &["cr"], &[]),
    l("D", 0xba595e, &["d"], &[]),
    l("Odin", 0x60affe, &["odin"], &[]),
    l("Gleam", 0xffaff3, &["gleam"], &[]),
    l("Mojo", 0xff4c1f, &["mojo"], &[]),
    l("Fortran", 0x4d41b1, &["f90", "f95", "f03", "f08", "f", "for"], &[]),
    l("Pascal", 0xe3f171, &["pas", "pp", "dpr"], &[]),
    l("Ada", 0x02f88c, &["adb", "ads"], &[]),
    l("Prolog", 0x74283c, &["prolog"], &[]),
    l("Tcl", 0xe4cc98, &["tcl"], &[]),
    l("AWK", 0xc30e9b, &["awk"], &[]),
    l("CoffeeScript", 0x244776, &["coffee"], &[]),
    l("Assembly", 0x6e4c13, &["asm", "s", "nasm"], &[]),
    l("WebAssembly", 0x04133b, &["wat", "wast"], &[]),
    l("Cuda", 0x3a4e3a, &["cu", "cuh"], &[]),
    l("GLSL", 0x5686a5, &["glsl", "vert", "frag", "geom", "comp"], &[]),
    l("HLSL", 0xaace60, &["hlsl"], &[]),
    l("Verilog", 0xb2b7f8, &["v", "sv", "svh", "vh"], &[]),
    l("VHDL", 0xadb2cb, &["vhd", "vhdl"], &[]),
    l("Solidity", 0xaa6746, &["sol"], &[]),
    l("GDScript", 0x355570, &["gd"], &[]),
    l("Hack", 0x878787, &["hh"], &[]),
    l("Visual Basic .NET", 0x945db7, &["vb"], &[]),
    l("PowerShell", 0x012456, &["ps1", "psm1", "psd1"], &[]),
    l("Batchfile", 0xc1f12e, &["bat", "cmd"], &[]),
    l("Nix", 0x7e7eff, &["nix"], &[]),
    l("Makefile", 0x427819, &["mk", "mak"], &["Makefile", "makefile", "GNUmakefile"]),
    l("CMake", 0xda3434, &["cmake"], &["CMakeLists.txt"]),
    l("Dockerfile", 0x384d54, &["dockerfile"], &["Dockerfile", "Containerfile"]),
    l("Starlark", 0x76d275, &["bzl", "star"], &["BUILD", "WORKSPACE", "BUILD.bazel"]),
    l("HCL", 0x844fba, &["tf", "tfvars", "hcl"], &[]),
    l("Puppet", 0x302b6d, &["pp"], &[]),
    l("SQL", 0xe38c00, &["sql"], &[]),
    l("GraphQL", 0xe10098, &["graphql", "gql"], &[]),
    l("TeX", 0x3d6117, &["tex", "sty"], &[]),
    l("Typst", 0x239dad, &["typ"], &[]),
    l("Jupyter Notebook", 0xda5b0b, &["ipynb"], &[]),
    l("HTML", 0xe34c26, &["html", "htm", "xhtml"], &[]),
    l("CSS", 0x663399, &["css"], &[]),
    l("SCSS", 0xc6538c, &["scss"], &[]),
    l("Sass", 0xa53b70, &["sass"], &[]),
    l("Less", 0x1d365d, &["less"], &[]),
    l("Stylus", 0xff6347, &["styl"], &[]),
    l("Vue", 0x41b883, &["vue"], &[]),
    l("Svelte", 0xff3e00, &["svelte"], &[]),
    l("Astro", 0xff5a03, &["astro"], &[]),
    l("Handlebars", 0xf7931e, &["hbs", "handlebars"], &[]),
    l("Pug", 0xa86454, &["pug", "jade"], &[]),
    l("Twig", 0xc1d026, &["twig"], &[]),
    l("Blade", 0xf7523f, &["blade.php"], &[]),
    l("Liquid", 0x67b8de, &["liquid"], &[]),
    l("Jinja", 0xa52a22, &["j2", "jinja", "jinja2"], &[]),
];

struct Index {
    by_ext: HashMap<&'static str, &'static Lang>,
    by_name: HashMap<&'static str, &'static Lang>,
}

fn index() -> &'static Index {
    static INDEX: OnceLock<Index> = OnceLock::new();
    INDEX.get_or_init(|| {
        let (mut by_ext, mut by_name) = (HashMap::new(), HashMap::new());
        for lang in LANGS {
            for ext in lang.exts {
                by_ext.entry(*ext).or_insert(lang); // an extension shared by two languages goes to the first listed
            }
            for name in lang.names {
                by_name.insert(*name, lang);
            }
        }
        Index { by_ext, by_name }
    })
}

fn named(name: &str) -> Option<&'static Lang> {
    LANGS.iter().find(|l| l.name == name)
}

/// Folders whose contents are somebody else's code.
const VENDORED: &[&str] = &["node_modules", "vendor", "third_party", "thirdparty", "bower_components", "Pods", ".yarn", "__pycache__", "venv", "site-packages"];

/// Name patterns of generated or minified files.
const GENERATED: &[&str] = &[".min.js", ".min.css", ".pb.go", ".pb.cc", ".pb.h", "_pb2.py", ".generated.", ".designer.cs", ".g.dart", ".bundle.js"];

/// True for files GitHub would not count: vendored folders and generated or minified files.
fn skipped(path: &str) -> bool {
    path.split('/').any(|part| VENDORED.contains(&part)) || GENERATED.iter().any(|g| path.contains(g))
}

fn ext_of(path: &str) -> Option<String> {
    let file = path.rsplit('/').next()?;
    file.rsplit_once('.').map(|(_, e)| e.to_lowercase())
}

fn classify(path: &str, has_cpp: bool) -> Option<&'static Lang> {
    let idx = index();
    let file = path.rsplit('/').next()?;
    if let Some(l) = idx.by_name.get(file) {
        return Some(l);
    }
    if file.ends_with(".blade.php") {
        return named("Blade");
    }
    let ext = ext_of(path)?;
    // A header is C++ when the repository has C++ sources, else C.
    if ext == "h" {
        return named(if has_cpp { "C++" } else { "C" });
    }
    idx.by_ext.get(ext.as_str()).copied()
}

/// One language's share of a repository.
pub struct Stat {
    pub name: &'static str,
    pub color: u32,
    pub bytes: u64,
    pub files: u32,
}

/// Languages of a repository, biggest first.
pub struct Breakdown {
    pub langs: Vec<Stat>,
    pub total: u64,
}

impl Breakdown {
    /// Share of the counted bytes, 0.0 to 100.0.
    pub fn percent(&self, stat: &Stat) -> f64 {
        if self.total == 0 { 0.0 } else { stat.bytes as f64 * 100.0 / self.total as f64 }
    }
}

/// Sums the sizes of `files` (`(path, bytes)`) per language.
pub fn analyze(files: Vec<(String, u64)>) -> Breakdown {
    let has_cpp = files.iter().any(|(p, _)| matches!(ext_of(p).as_deref(), Some("cpp" | "cc" | "cxx" | "hpp" | "hxx" | "hh")));
    let mut sums: HashMap<&'static str, Stat> = HashMap::new();
    for (path, size) in files {
        if size == 0 || skipped(&path) {
            continue;
        }
        let Some(lang) = classify(&path, has_cpp) else { continue };
        let s = sums.entry(lang.name).or_insert(Stat { name: lang.name, color: lang.color, bytes: 0, files: 0 });
        s.bytes += size;
        s.files += 1;
    }
    let mut langs: Vec<Stat> = sums.into_values().collect();
    langs.sort_by(|a, b| b.bytes.cmp(&a.bytes).then(a.name.cmp(b.name)));
    let total = langs.iter().map(|s| s.bytes).sum();
    Breakdown { langs, total }
}
