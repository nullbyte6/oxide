//! A small, dependency-free syntax highlighter for the diff viewer.
//!
//! It tokenizes one line at a time. Languages are described by a `Lang` table (keywords, comment
//! markers, string quotes and a few flags); the only state carried between lines is whether a block
//! comment or a multi-line string is open. It is a lexer, not a parser: good enough to color keywords,
//! types, strings, numbers, comments and calls the way an editor would, but it never rejects input.

/// Kind of a highlighted piece of text; `ui.rs` maps each kind to a style.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Tok {
    Plain,
    Keyword,
    Type,
    Func,
    Str,
    Number,
    Comment,
    /// `true`, `null`, `self`, ...
    Const,
    /// Macros, attributes, preprocessor directives, lifetimes and shell variables.
    Macro,
    /// Object keys and property names.
    Key,
}

/// What is still open at the end of a line.
#[derive(Clone, Copy, PartialEq, Default)]
pub enum State {
    #[default]
    Normal,
    /// Inside a block comment.
    Block,
    /// Inside a string that continues on the next line, closed by this quote character.
    Multi(char),
    /// Inside a triple-quoted string (`"""` or `'''`).
    Triple(char),
}

/// Language description.
pub struct Lang {
    keywords: &'static [&'static str],
    line_comments: &'static [&'static str],
    block: Option<(&'static str, &'static str)>,
    /// Characters that open a string.
    quotes: &'static str,
    /// Quotes whose strings may continue on the next line.
    multiline: &'static str,
    /// Python-style triple quotes.
    triple: bool,
    /// Keywords match regardless of case (SQL).
    ci: bool,
    /// `-` is part of identifiers (CSS, YAML, TOML, HTML).
    dash: bool,
    /// A leading name or string followed by this character is a key.
    key_sep: Option<char>,
    /// `$name` is a variable (shell).
    dollar: bool,
    /// Rust: `#[attribute]`, lifetimes and `macro!`.
    rust: bool,
    /// C family: `#include`-style directives.
    preproc: bool,
    /// HTML and XML tags.
    markup: bool,
    /// `[section]` lines (TOML, INI).
    section: bool,
    /// Markdown headings and code.
    markdown: bool,
}

const BASE: Lang = Lang {
    keywords: &[],
    line_comments: &[],
    block: None,
    quotes: "\"'",
    multiline: "",
    triple: false,
    ci: false,
    dash: false,
    key_sep: None,
    dollar: false,
    rust: false,
    preproc: false,
    markup: false,
    section: false,
    markdown: false,
};

const RUST: Lang = Lang {
    keywords: &[
        "as", "async", "await", "break", "const", "continue", "crate", "dyn", "else", "enum", "extern", "fn", "for", "if", "impl", "in",
        "let", "loop", "match", "mod", "move", "mut", "pub", "ref", "return", "static", "struct", "super", "trait", "type", "union",
        "unsafe", "use", "where", "while",
    ],
    line_comments: &["//"],
    block: Some(("/*", "*/")),
    rust: true,
    ..BASE
};

const PYTHON: Lang = Lang {
    keywords: &[
        "and", "as", "assert", "async", "await", "break", "case", "class", "continue", "def", "del", "elif", "else", "except", "finally",
        "for", "from", "global", "if", "import", "in", "is", "lambda", "match", "nonlocal", "not", "or", "pass", "raise", "return", "try",
        "while", "with", "yield",
    ],
    line_comments: &["#"],
    triple: true,
    ..BASE
};

const JS: Lang = Lang {
    keywords: &[
        "abstract", "as", "async", "await", "break", "case", "catch", "class", "const", "continue", "debugger", "declare", "default",
        "delete", "do", "else", "enum", "export", "extends", "finally", "for", "from", "function", "if", "implements", "import", "in",
        "instanceof", "interface", "let", "namespace", "new", "of", "private", "protected", "public", "readonly", "return", "static",
        "super", "switch", "throw", "try", "type", "typeof", "var", "void", "while", "with", "yield",
    ],
    line_comments: &["//"],
    block: Some(("/*", "*/")),
    quotes: "\"'`",
    multiline: "`",
    ..BASE
};

const GO: Lang = Lang {
    keywords: &[
        "break", "case", "chan", "const", "continue", "default", "defer", "else", "fallthrough", "for", "func", "go", "goto", "if",
        "import", "interface", "map", "package", "range", "return", "select", "struct", "switch", "type", "var",
    ],
    line_comments: &["//"],
    block: Some(("/*", "*/")),
    quotes: "\"'`",
    multiline: "`",
    ..BASE
};

const C: Lang = Lang {
    keywords: &[
        "auto", "break", "case", "catch", "class", "const", "constexpr", "continue", "default", "delete", "do", "else", "enum", "explicit",
        "extern", "for", "friend", "goto", "if", "inline", "namespace", "new", "operator", "private", "protected", "public", "register",
        "return", "sizeof", "static", "struct", "switch", "template", "throw", "try", "typedef", "typename", "union", "using", "virtual",
        "volatile", "while",
    ],
    line_comments: &["//"],
    block: Some(("/*", "*/")),
    preproc: true,
    ..BASE
};

const JAVA: Lang = Lang {
    keywords: &[
        "abstract", "as", "break", "case", "catch", "class", "const", "continue", "default", "do", "else", "enum", "extends", "final",
        "finally", "for", "fun", "if", "implements", "import", "in", "instanceof", "interface", "is", "namespace", "new", "object",
        "override", "package", "private", "protected", "public", "return", "static", "super", "switch", "synchronized", "throw",
        "throws", "try", "using", "val", "var", "when", "while",
    ],
    line_comments: &["//"],
    block: Some(("/*", "*/")),
    ..BASE
};

const SHELL: Lang = Lang {
    keywords: &[
        "alias", "break", "case", "continue", "declare", "do", "done", "elif", "else", "esac", "exit", "export", "fi", "for",
        "function", "if", "in", "local", "readonly", "return", "select", "source", "then", "unset", "until", "while",
    ],
    line_comments: &["#"],
    quotes: "\"'`",
    dollar: true,
    ..BASE
};

const TOML: Lang = Lang { line_comments: &["#"], dash: true, key_sep: Some('='), section: true, ..BASE };

const JSON: Lang = Lang { quotes: "\"", key_sep: Some(':'), ..BASE };

const YAML: Lang = Lang { line_comments: &["#"], dash: true, key_sep: Some(':'), ..BASE };

const MARKDOWN: Lang = Lang { quotes: "`", markdown: true, ..BASE };

const HTML: Lang = Lang { block: Some(("<!--", "-->")), dash: true, markup: true, ..BASE };

const CSS: Lang = Lang { block: Some(("/*", "*/")), dash: true, key_sep: Some(':'), ..BASE };

const SQL: Lang = Lang {
    keywords: &[
        "add", "all", "alter", "and", "as", "asc", "begin", "by", "commit", "constraint", "create", "database", "default", "delete",
        "desc", "distinct", "drop", "foreign", "from", "group", "having", "in", "index", "inner", "insert", "into", "is", "join", "key",
        "left", "like", "limit", "not", "null", "offset", "on", "or", "order", "outer", "primary", "references", "right", "rollback",
        "select", "set", "table", "union", "unique", "update", "values", "view", "where",
    ],
    line_comments: &["--"],
    block: Some(("/*", "*/")),
    ci: true,
    ..BASE
};

const LUA: Lang = Lang {
    keywords: &[
        "and", "break", "do", "else", "elseif", "end", "for", "function", "goto", "if", "in", "local", "not", "or", "repeat", "return",
        "then", "until", "while",
    ],
    line_comments: &["--"],
    block: Some(("--[[", "]]")),
    ..BASE
};

const RUBY: Lang = Lang {
    keywords: &[
        "alias", "and", "begin", "break", "case", "class", "def", "defined", "do", "else", "elsif", "end", "ensure", "for", "if", "in",
        "module", "next", "not", "or", "redo", "rescue", "retry", "return", "then", "undef", "unless", "until", "when", "while", "yield",
    ],
    line_comments: &["#"],
    ..BASE
};

/// Words that are constants in most languages.
const CONSTS: &[&str] = &["true", "false", "null", "nil", "None", "True", "False", "NULL", "undefined", "self", "this", "Self"];

/// Built-in type names shared by many languages.
const TYPES: &[&str] = &[
    "i8", "i16", "i32", "i64", "i128", "isize", "u8", "u16", "u32", "u64", "u128", "usize", "f32", "f64", "bool", "str", "char", "int",
    "float", "double", "long", "short", "void", "string", "number", "boolean", "byte", "uint", "size_t", "any", "never", "object",
];

/// The language for a file path, by extension or well-known file name.
pub fn lang_for(path: &str) -> Option<&'static Lang> {
    let name = path.rsplit('/').next().unwrap_or(path);
    match name {
        "Makefile" | "makefile" | "Dockerfile" | ".gitignore" | ".env" => return Some(&SHELL),
        "Cargo.lock" | "Pipfile" => return Some(&TOML),
        _ => {}
    }
    let ext = name.rsplit_once('.')?.1.to_ascii_lowercase();
    Some(match ext.as_str() {
        "rs" => &RUST,
        "py" | "pyi" => &PYTHON,
        "js" | "jsx" | "mjs" | "cjs" | "ts" | "tsx" => &JS,
        "go" => &GO,
        "c" | "h" | "cc" | "cpp" | "cxx" | "hpp" | "hh" | "ino" => &C,
        "java" | "kt" | "kts" | "cs" | "scala" | "swift" | "dart" => &JAVA,
        "sh" | "bash" | "zsh" | "fish" | "mk" => &SHELL,
        "toml" | "ini" | "cfg" | "conf" | "service" => &TOML,
        "json" | "jsonc" | "lock" => &JSON,
        "yml" | "yaml" => &YAML,
        "md" | "markdown" => &MARKDOWN,
        "html" | "htm" | "xml" | "svg" | "xhtml" | "vue" => &HTML,
        "css" | "scss" | "less" => &CSS,
        "sql" => &SQL,
        "lua" => &LUA,
        "rb" => &RUBY,
        _ => return None,
    })
}

fn starts(chars: &[char], i: usize, pat: &str) -> bool {
    let mut k = i;
    for p in pat.chars() {
        if chars.get(k) != Some(&p) {
            return false;
        }
        k += 1;
    }
    true
}

/// Index just after the next occurrence of `pat` at or after `from`, if any.
fn find_end(chars: &[char], from: usize, pat: &str) -> Option<usize> {
    let n = pat.chars().count();
    (from..chars.len()).find(|&k| starts(chars, k, pat)).map(|k| k + n)
}

fn is_ident(c: char, lang: &Lang) -> bool {
    c.is_alphanumeric() || c == '_' || (lang.dash && c == '-')
}

/// The first non-space character at or after `from`.
fn next_non_space(chars: &[char], from: usize) -> Option<char> {
    chars.get(from..)?.iter().copied().find(|c| !c.is_whitespace())
}

struct Out(Vec<(Tok, String)>);

impl Out {
    fn push(&mut self, tok: Tok, text: impl AsRef<str>) {
        let text = text.as_ref();
        if text.is_empty() {
            return;
        }
        match self.0.last_mut() {
            Some((t, s)) if *t == tok => s.push_str(text),
            _ => self.0.push((tok, text.to_string())),
        }
    }
}

/// Highlights one line of source; `state` carries open comments and strings from line to line.
pub fn highlight(lang: Option<&Lang>, line: &str, state: &mut State) -> Vec<(Tok, String)> {
    let Some(lang) = lang else { return vec![(Tok::Plain, line.to_string())] };
    let chars: Vec<char> = line.chars().collect();
    let mut out = Out(Vec::new());

    if lang.markdown && *state == State::Normal {
        let t = line.trim_start();
        if t.starts_with('#') {
            out.push(Tok::Keyword, line);
            return out.0;
        }
        if t.starts_with("```") {
            out.push(Tok::Str, line);
            return out.0;
        }
    }
    if lang.section && *state == State::Normal && line.trim_start().starts_with('[') {
        out.push(Tok::Type, line);
        return out.0;
    }

    let (mut i, mut seen_token) = (0, false);
    while i < chars.len() {
        // Continue whatever the previous line left open.
        match *state {
            State::Block => {
                let end = lang.block.map_or("", |b| b.1);
                match find_end(&chars, i, end) {
                    Some(e) => {
                        out.push(Tok::Comment, chars[i..e].iter().collect::<String>());
                        *state = State::Normal;
                        i = e;
                    }
                    None => {
                        out.push(Tok::Comment, chars[i..].iter().collect::<String>());
                        return out.0;
                    }
                }
                continue;
            }
            State::Multi(q) | State::Triple(q) => {
                let triple = matches!(*state, State::Triple(_));
                let end = scan_string(&chars, i, q, if triple { 3 } else { 1 });
                out.push(Tok::Str, chars[i..end.unwrap_or(chars.len())].iter().collect::<String>());
                match end {
                    Some(e) => {
                        *state = State::Normal;
                        i = e;
                        continue;
                    }
                    None => return out.0,
                }
            }
            State::Normal => {}
        }

        let c = chars[i];
        if c.is_whitespace() {
            out.push(Tok::Plain, c.to_string());
            i += 1;
            continue;
        }
        if let Some((open, _)) = lang.block
            && starts(&chars, i, open)
        {
            out.push(Tok::Comment, open);
            *state = State::Block;
            i += open.chars().count();
            seen_token = true;
            continue;
        }
        if lang.line_comments.iter().any(|lc| starts(&chars, i, lc) && (*lc != "#" || i == 0 || chars[i - 1].is_whitespace())) {
            out.push(Tok::Comment, chars[i..].iter().collect::<String>());
            return out.0;
        }
        if lang.rust && c == '#' && matches!(chars.get(i + 1), Some('[' | '!')) {
            let end = chars[i..].iter().position(|&x| x == ']').map_or(chars.len(), |p| i + p + 1);
            out.push(Tok::Macro, chars[i..end].iter().collect::<String>());
            i = end;
            seen_token = true;
            continue;
        }
        if lang.preproc && c == '#' && !seen_token {
            let mut e = i + 1;
            while chars.get(e).is_some_and(|x| x.is_whitespace()) {
                e += 1;
            }
            while chars.get(e).is_some_and(|x| x.is_alphanumeric() || *x == '_') {
                e += 1;
            }
            out.push(Tok::Macro, chars[i..e].iter().collect::<String>());
            i = e;
            seen_token = true;
            continue;
        }
        if lang.markup && c == '<' {
            let mut e = i + 1;
            if matches!(chars.get(e), Some('/' | '!' | '?')) {
                e += 1;
            }
            while chars.get(e).is_some_and(|x| is_ident(*x, lang) || *x == ':') {
                e += 1;
            }
            out.push(Tok::Keyword, chars[i..e].iter().collect::<String>());
            i = e;
            seen_token = true;
            continue;
        }
        if lang.markup && (c == '>' || starts(&chars, i, "/>")) {
            let n = if c == '>' { 1 } else { 2 };
            out.push(Tok::Keyword, chars[i..i + n].iter().collect::<String>());
            i += n;
            continue;
        }
        if lang.quotes.contains(c) {
            seen_token = true;
            // Rust: `'a` is a lifetime, `'x'` and `'\n'` are characters.
            if lang.rust && c == '\'' {
                let is_char = chars.get(i + 1) == Some(&'\\') || chars.get(i + 2) == Some(&'\'');
                if !is_char {
                    let mut e = i + 1;
                    while chars.get(e).is_some_and(|x| x.is_alphanumeric() || *x == '_') {
                        e += 1;
                    }
                    out.push(Tok::Macro, chars[i..e].iter().collect::<String>());
                    i = e;
                    continue;
                }
            }
            let triple = lang.triple && starts(&chars, i, &format!("{c}{c}{c}"));
            let end = scan_string(&chars, i + if triple { 3 } else { 1 }, c, if triple { 3 } else { 1 });
            let stop = end.unwrap_or(chars.len());
            let text: String = chars[i..stop].iter().collect();
            if end.is_none() && (triple || lang.multiline.contains(c)) {
                out.push(Tok::Str, text);
                *state = if triple { State::Triple(c) } else { State::Multi(c) };
                return out.0;
            }
            let is_key = lang.key_sep.is_some() && end.is_some() && next_non_space(&chars, stop) == lang.key_sep;
            out.push(if is_key { Tok::Key } else { Tok::Str }, text);
            i = stop;
            continue;
        }
        if c.is_ascii_digit() {
            let mut e = i + 1;
            while e < chars.len() && (chars[e].is_alphanumeric() || chars[e] == '_' || (chars[e] == '.' && chars.get(e + 1) != Some(&'.'))) {
                e += 1;
            }
            out.push(Tok::Number, chars[i..e].iter().collect::<String>());
            i = e;
            seen_token = true;
            continue;
        }
        if lang.dollar && c == '$' {
            let mut e = i + 1;
            if chars.get(e) == Some(&'{') {
                e = chars[e..].iter().position(|&x| x == '}').map_or(chars.len(), |p| e + p + 1);
            } else {
                while chars.get(e).is_some_and(|x| x.is_alphanumeric() || matches!(*x, '_' | '@' | '*' | '#' | '?' | '$')) {
                    e += 1;
                }
            }
            out.push(Tok::Macro, chars[i..e].iter().collect::<String>());
            i = e;
            seen_token = true;
            continue;
        }
        if c.is_alphabetic() || c == '_' {
            let mut e = i + 1;
            while e < chars.len() && is_ident(chars[e], lang) {
                e += 1;
            }
            let word: String = chars[i..e].iter().collect();
            let next = next_non_space(&chars, e);
            let tok = if CONSTS.contains(&word.as_str()) {
                Tok::Const
            } else if lang.keywords.iter().any(|k| if lang.ci { k.eq_ignore_ascii_case(&word) } else { *k == word }) {
                Tok::Keyword
            } else if lang.rust && chars.get(e) == Some(&'!') && chars.get(e + 1) != Some(&'=') {
                e += 1; // the `!` belongs to the macro name
                Tok::Macro
            } else if lang.key_sep.is_some() && !seen_token && next == lang.key_sep {
                Tok::Key
            } else if lang.markup && chars.get(e) == Some(&'=') {
                Tok::Type // attribute name
            } else if TYPES.contains(&word.as_str()) || word.starts_with(|x: char| x.is_uppercase()) {
                Tok::Type
            } else if chars.get(e) == Some(&'(') || (next == Some('(') && !lang.markup) {
                Tok::Func
            } else {
                Tok::Plain
            };
            out.push(tok, chars[i..e].iter().collect::<String>());
            i = e;
            seen_token = true;
            continue;
        }
        // Punctuation. A YAML list dash does not count as the first token of the line.
        out.push(Tok::Plain, c.to_string());
        if !(c == '-' && !seen_token && chars.get(i + 1) == Some(&' ')) {
            seen_token = true;
        }
        i += 1;
    }
    out.0
}

/// Finds where the string opened before `from` ends. `quote` is the closing character, repeated
/// `n` times for triple quotes. Returns the index just after the closing quote(s), if the string closes.
fn scan_string(chars: &[char], from: usize, quote: char, n: usize) -> Option<usize> {
    let mut j = from;
    while j < chars.len() {
        if chars[j] == '\\' {
            j += 2;
            continue;
        }
        if chars[j] == quote && (1..n).all(|k| chars.get(j + k) == Some(&quote)) {
            return Some(j + n);
        }
        j += 1;
    }
    None
}
