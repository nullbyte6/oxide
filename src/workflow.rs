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
