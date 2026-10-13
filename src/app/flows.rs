//! GitHub workflows (`N`): the list of the repository's workflows, the form that starts one by hand with
//! its inputs (`gh workflow run`), and, further down, the wizard that writes a new one.

use super::*;
use crate::workflow::{self, Input, InputKind, Workflow};

/// The workflows of one repository and what GitHub knows of their latest runs.
pub struct WorkflowsView {
    pub repo: PathBuf,
    pub name: String,
    /// `owner/repo` of the GitHub remote; without one workflows can be listed but not run.
    pub slug: Option<String>,
    /// The branch the repository is on, which the run form starts with.
    pub branch: String,
    pub items: Vec<Workflow>,
    pub state: ListState,
    /// The latest run of each workflow by name; None while loading, Err without `gh` or a GitHub remote.
    pub runs: Option<Result<HashMap<String, WorkflowRun>, String>>,
    pub(super) id: u64,
}

impl WorkflowsView {
    pub fn selected(&self) -> Option<&Workflow> {
        self.items.get(self.state.selected()?)
    }
}

/// One line of the run form.
pub struct Field {
    pub label: String,
    /// What the input is for (its `description`).
    pub help: String,
    pub required: bool,
    pub kind: InputKind,
    /// The text, `true` / `false` for a checkbox, or the chosen option.
    pub value: String,
    pub options: Vec<String>,
}

/// The form that starts a workflow: the branch to run on, then one line per input of the workflow.
pub struct RunForm {
    pub repo: PathBuf,
    pub slug: String,
    pub file: String,
    pub title: String,
    /// A warning shown above the fields (the file is not committed, there are no inputs ...).
    pub note: Option<String>,
    /// The first field is the branch or tag (`--ref`); the rest are the workflow's inputs, in order.
    pub fields: Vec<Field>,
    pub cursor: usize,
    pub error: Option<String>,
}

impl App {
    pub(super) fn open_workflows(&mut self) {
        let Some((repo, name, slug, branch)) = self.selected().map(|r| (r.path.clone(), r.name.clone(), r.slug.clone(), r.branch.clone())) else { return };
        let items = workflow::read_all(&repo);
        let state = ListState::default().with_selected(if items.is_empty() { None } else { Some(0) });
        self.workflows = Some(WorkflowsView { repo, name, slug, branch, items, state, runs: None, id: 0 });
        self.mode = Mode::Workflows;
        self.load_workflow_runs();
    }

    /// Reads the files again and asks GitHub for the latest runs in the background.
    pub(super) fn load_workflows(&mut self) {
        if let Some(v) = self.workflows.as_mut() {
            let keep = v.selected().map(|w| w.file.clone());
            v.items = workflow::read_all(&v.repo);
            let at = keep.and_then(|f| v.items.iter().position(|w| w.file == f)).or(if v.items.is_empty() { None } else { Some(0) });
            v.state.select(at);
        }
        self.load_workflow_runs();
    }

    fn load_workflow_runs(&mut self) {
        self.diff_id += 1;
        if let Some(v) = self.workflows.as_mut() {
            v.id = self.diff_id;
            match &v.slug {
                Some(slug) => {
                    v.runs = None;
                    worker::spawn_workflow_runs(slug.clone(), v.id, self.tx.clone());
                }
                None => v.runs = Some(Err("no github.com remote".to_string())),
            }
        }
    }

    pub(super) fn on_workflows_key(&mut self, key: KeyEvent) {
        let Some(v) = self.workflows.as_mut() else {
            self.mode = Mode::Normal;
            return;
        };
        let len = v.items.len();
        let picked = v.selected().cloned();
        let (repo, slug) = (v.repo.clone(), v.slug.clone());
        let delta = match key.code {
            KeyCode::Esc | KeyCode::Char('q' | 'N') => {
                self.workflows = None;
                self.mode = Mode::Normal;
                return;
            }
            KeyCode::Down | KeyCode::Char('j') => 1,
            KeyCode::Up | KeyCode::Char('k') => -1,
            KeyCode::Home | KeyCode::Char('g') => -(1 << 30),
            KeyCode::End | KeyCode::Char('G') => 1 << 30,
            KeyCode::Char('f') | KeyCode::F(5) => return self.load_workflows(),
            _ => {
                // Making a new one does not need a workflow to be highlighted.
                if key.code == KeyCode::Char('n') {
                    return self.open_wizard();
                }
                let Some(w) = picked else {
                    self.log(Level::Info, "no workflow highlighted (press n to create one)");
                    return;
                };
                match key.code {
                    KeyCode::Enter | KeyCode::Char('r') => self.open_run_form(w),
                    KeyCode::Char('n') => self.open_wizard(),
                    KeyCode::Char('o') => match slug {
                        Some(slug) => {
                            let args = vec!["workflow".to_string(), "view".into(), w.file.clone(), "-R".into(), slug, "--web".into()];
                            self.spawn_detached("gh", &args, &format!("{} opened in the browser", w.file));
                        }
                        None => self.log(Level::Warn, "opening a workflow needs a github.com remote"),
                    },
                    KeyCode::Char('e') => {
                        let target = repo.join(workflow::DIR).join(&w.file);
                        self.browse(repo, target);
                    }
                    _ => {}
                }
                return;
            }
        };
        v.state.select(step(v.state.selected(), len, delta));
    }

    /// Opens the form that starts `w`: a branch, then its inputs. A workflow without a `workflow_dispatch`
    /// trigger cannot be started by hand, and running needs a GitHub remote.
    fn open_run_form(&mut self, w: Workflow) {
        let Some(v) = self.workflows.as_ref() else { return };
        let (repo, branch) = (v.repo.clone(), v.branch.clone());
        let Some(slug) = v.slug.clone() else {
            self.log(Level::Warn, "running a workflow needs a github.com remote");
            return;
        };
        if !w.dispatch {
            self.log(Level::Warn, format!("{} has no workflow_dispatch trigger, so GitHub cannot start it by hand (add `workflow_dispatch:` under `on:`)", w.file));
            return;
        }
        // GitHub runs the version on the remote, so a file that is new or changed here would not be what runs.
        let note = match git::path_state(&repo, &format!("{}/{}", workflow::DIR, w.file)) {
            git::PathState::Clean => None,
            git::PathState::Untracked => Some("this file is not committed yet: GitHub cannot run it until it is pushed".to_string()),
            git::PathState::Changed => Some("this file has uncommitted changes: GitHub runs the pushed version".to_string()),
        };
        let mut fields = vec![Field {
            label: "ref".to_string(),
            help: "branch or tag that holds the workflow".to_string(),
            required: true,
            kind: InputKind::Text,
            value: branch,
            options: Vec::new(),
        }];
        fields.extend(w.inputs.iter().map(|i: &Input| Field {
            label: i.name.clone(),
            help: i.description.clone(),
            required: i.required,
            kind: i.kind,
            value: i.default.clone(),
            options: i.options.clone(),
        }));
        self.run_form = Some(RunForm { repo, slug, file: w.file, title: w.name, note, fields, cursor: 0, error: None });
        self.mode = Mode::WorkflowRun;
    }

    pub(super) fn on_run_key(&mut self, key: KeyEvent) {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let Some(form) = self.run_form.as_mut() else {
            self.mode = Mode::Workflows;
            return;
        };
        let n = form.fields.len();
        match key.code {
            KeyCode::Esc => {
                self.run_form = None;
                self.mode = Mode::Workflows;
            }
            KeyCode::Down | KeyCode::Tab => form.cursor = (form.cursor + 1) % n,
            KeyCode::Up | KeyCode::BackTab => form.cursor = (form.cursor + n - 1) % n,
            KeyCode::Enter => self.submit_run_form(),
            _ => {
                form.error = None;
                let f = &mut form.fields[form.cursor];
                match f.kind {
                    InputKind::Text => match key.code {
                        KeyCode::Backspace => {
                            f.value.pop();
                        }
                        KeyCode::Char('u') if ctrl => f.value.clear(),
                        KeyCode::Char(c) if !ctrl => f.value.push(c),
                        _ => {}
                    },
                    InputKind::Bool => {
                        if matches!(key.code, KeyCode::Char(' ') | KeyCode::Left | KeyCode::Right) {
                            f.value = if f.value == "true" { "false" } else { "true" }.to_string();
                        }
                    }
                    InputKind::Choice => {
                        let at = f.options.iter().position(|o| *o == f.value).unwrap_or(0);
                        let to = match key.code {
                            KeyCode::Left => (at + f.options.len() - 1) % f.options.len(),
                            KeyCode::Right | KeyCode::Char(' ') => (at + 1) % f.options.len(),
                            _ => at,
                        };
                        f.value = f.options[to].clone();
                    }
                }
            }
        }
    }

    /// Checks the required fields and asks for a yes before starting the run.
    fn submit_run_form(&mut self) {
        let Some(form) = self.run_form.as_mut() else { return };
        if let Some(f) = form.fields.iter().find(|f| f.required && f.value.trim().is_empty()) {
            form.error = Some(format!("{} is required", f.label));
            return;
        }
        let git_ref = form.fields[0].value.trim().to_string();
        let inputs: Vec<(String, String)> = form.fields[1..].iter().filter(|f| !f.value.is_empty() || f.required).map(|f| (f.label.clone(), f.value.trim().to_string())).collect();
        let shown = if inputs.is_empty() { String::new() } else { format!(" with {}", inputs.iter().map(|(k, v)| format!("{k}={v}")).collect::<Vec<_>>().join(", ")) };
        let prompt = format!("run {} on {git_ref}{shown}? this uses GitHub Actions minutes", form.file);
        let job = (form.repo.clone(), Op::Dispatch { slug: form.slug.clone(), file: form.file.clone(), git_ref, inputs });
        self.ask(prompt, vec![job]);
    }
}

// ---- the wizard that writes a new workflow ----

use crate::workflow::{Facts, Kind, Spec, TOOLCHAINS, Tc};

/// The pages of the wizard.
#[derive(Clone, Copy, PartialEq)]
pub enum WStep {
    Kind,
    Toolchain,
    Triggers,
    Options,
    Details,
    Preview,
}

/// A box to check.
pub struct Check {
    pub label: String,
    pub hint: String,
    pub on: bool,
}

/// A line of text on the details page.
pub struct Detail {
    pub label: &'static str,
    pub help: String,
    pub value: String,
}

/// The wizard: kind, toolchain, triggers, options, details, then the compiled file to write.
pub struct Wizard {
    pub repo: PathBuf,
    pub name: String,
    pub slug: Option<String>,
    pub step: WStep,
    pub kind_state: ListState,
    pub tool_state: ListState,
    /// The toolchain the repository looks like it uses, an index into `TOOLCHAINS`.
    pub detected: Option<usize>,
    pub spec: Spec,
    /// The boxes of the triggers or options page.
    pub checks: Vec<Check>,
    pub check_state: ListState,
    pub details: Vec<Detail>,
    pub cursor: usize,
    pub error: Option<String>,
    /// File name inside `.github/workflows`, chosen on the details page.
    pub file: String,
    /// The file already exists and would be replaced.
    pub exists: bool,
    pub yaml: String,
    pub scroll: usize,
    /// Rows the preview shows, set when it is drawn.
    pub height: usize,
}

fn custom_toolchain() -> usize {
    TOOLCHAINS.iter().position(|(t, _, _)| *t == Tc::Custom).unwrap_or(0)
}

fn trigger_checks(spec: &Spec) -> Vec<Check> {
    let c = |label: &str, hint: &str, on: bool| Check { label: label.to_string(), hint: hint.to_string(), on };
    vec![
        c("Push to a branch", "the branches are on the details page", spec.push),
        c("Pull request", "against those branches", spec.pull_request),
        c("Version tag (v*)", "a push of a tag such as v1.2.0", spec.tags),
        c("Schedule", "a cron time, on the details page", spec.schedule),
        c("Run by hand", "workflow_dispatch: start it from GitHub or from Oxide (N)", spec.dispatch),
    ]
}

fn option_checks(spec: &Spec) -> Vec<Check> {
    let c = |label: &str, hint: &str, on: bool| Check { label: label.to_string(), hint: hint.to_string(), on };
    let mut v = vec![
        c("Linux", "ubuntu-latest", spec.linux),
        c("macOS", "macos-latest", spec.macos),
        c("Windows", "windows-latest, commands run in bash", spec.windows),
        c("Cache dependencies", "faster runs", spec.cache),
        c("Keep the build output", "uploaded as a run artifact", spec.artifacts),
        c("Cancel obsolete runs", "a newer push stops the older run", spec.concurrency),
    ];
    if spec.kind == Kind::Release {
        v.push(c("Publish a GitHub release", "with generated notes and the build output", spec.release));
    }
    v
}

fn detail_fields(spec: &Spec, file: &str) -> Vec<Detail> {
    let mut v = vec![
        Detail { label: "Name", help: "shown on GitHub".to_string(), value: spec.name.clone() },
        Detail { label: "File", help: format!("in {}", workflow::DIR), value: file.to_string() },
    ];
    if spec.push || spec.pull_request {
        v.push(Detail { label: "Branches", help: "separated by commas".to_string(), value: spec.branches.join(", ") });
    }
    if spec.schedule {
        v.push(Detail { label: "Cron", help: "minute hour day month weekday, in UTC".to_string(), value: spec.cron.clone() });
    }
    if spec.kind == Kind::Manual || spec.tc() == Tc::Custom {
        let help = if spec.kind == Kind::Manual { "the default of the input you are asked for" } else { "separated by ;" };
        v.push(Detail { label: "Command", help: help.to_string(), value: spec.command.clone() });
    }
    v
}

/// Edits `value` with a key press: characters, Backspace and Ctrl+U.
fn edit_text(value: &mut String, key: KeyEvent) {
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    match key.code {
        KeyCode::Backspace => {
            value.pop();
        }
        KeyCode::Char('u') if ctrl => value.clear(),
        KeyCode::Char(c) if !ctrl => value.push(c),
        _ => {}
    }
}

impl App {
    /// Opens the wizard for the repository of the workflows view.
    fn open_wizard(&mut self) {
        let Some(v) = self.workflows.as_ref() else { return };
        let (repo, name, slug, branch) = (v.repo.clone(), v.name.clone(), v.slug.clone(), v.branch.clone());
        let (detected, facts): (Option<usize>, Facts) = workflow::detect(&repo);
        let spec = Spec::new(Kind::Ci, detected.unwrap_or_else(custom_toolchain), facts, &branch);
        self.wizard = Some(Wizard {
            repo,
            name,
            slug,
            step: WStep::Kind,
            kind_state: ListState::default().with_selected(Some(0)),
            tool_state: ListState::default().with_selected(Some(detected.unwrap_or(0))),
            detected,
            spec,
            checks: Vec::new(),
            check_state: ListState::default().with_selected(Some(0)),
            details: Vec::new(),
            cursor: 0,
            error: None,
            file: String::new(),
            exists: false,
            yaml: String::new(),
            scroll: 0,
            height: 20,
        });
        self.mode = Mode::WorkflowNew;
    }

    pub(super) fn on_wizard_key(&mut self, key: KeyEvent) {
        let Some(w) = self.wizard.as_mut() else {
            self.mode = Mode::Workflows;
            return;
        };
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let delta = match key.code {
            KeyCode::Down => 1,
            KeyCode::Up => -1,
            KeyCode::Char('n') if ctrl => 1,
            KeyCode::Char('p') if ctrl => -1,
            KeyCode::PageDown => 10,
            KeyCode::PageUp => -10,
            _ => 0,
        };
        w.error = None;
        match w.step {
            WStep::Kind => match key.code {
                KeyCode::Esc => {
                    self.wizard = None;
                    self.mode = Mode::Workflows;
                }
                KeyCode::Enter => {
                    let kind = Kind::ALL[w.kind_state.selected().unwrap_or(0)];
                    let branch = w.spec.branches.first().cloned().unwrap_or_else(|| "main".to_string());
                    w.spec = Spec::new(kind, w.spec.toolchain, w.spec.facts.clone(), &branch);
                    w.step = WStep::Toolchain;
                }
                _ => w.kind_state.select(step(w.kind_state.selected(), Kind::ALL.len(), delta)),
            },
            WStep::Toolchain => match key.code {
                KeyCode::Esc => w.step = WStep::Kind,
                KeyCode::Enter => {
                    w.spec.toolchain = w.tool_state.selected().unwrap_or(0);
                    w.checks = trigger_checks(&w.spec);
                    w.check_state.select(Some(0));
                    w.step = WStep::Triggers;
                }
                _ => w.tool_state.select(step(w.tool_state.selected(), TOOLCHAINS.len(), delta)),
            },
            WStep::Triggers | WStep::Options => match key.code {
                KeyCode::Esc => {
                    if w.step == WStep::Triggers {
                        w.step = WStep::Toolchain;
                    } else {
                        w.checks = trigger_checks(&w.spec);
                        w.step = WStep::Triggers;
                    }
                    w.check_state.select(Some(0));
                }
                KeyCode::Char(' ') | KeyCode::Char('x') => {
                    if let Some(c) = w.check_state.selected().and_then(|i| w.checks.get_mut(i)) {
                        c.on = !c.on;
                    }
                }
                KeyCode::Enter => {
                    let on: Vec<bool> = w.checks.iter().map(|c| c.on).collect();
                    if w.step == WStep::Triggers {
                        (w.spec.push, w.spec.pull_request, w.spec.tags, w.spec.schedule, w.spec.dispatch) = (on[0], on[1], on[2], on[3], on[4]);
                        if !on.iter().any(|b| *b) {
                            w.error = Some("pick at least one trigger".to_string());
                            return;
                        }
                        w.checks = option_checks(&w.spec);
                        w.step = WStep::Options;
                    } else {
                        (w.spec.linux, w.spec.macos, w.spec.windows, w.spec.cache, w.spec.artifacts, w.spec.concurrency) = (on[0], on[1], on[2], on[3], on[4], on[5]);
                        if let Some(r) = on.get(6) {
                            w.spec.release = *r;
                        }
                        if !(w.spec.linux || w.spec.macos || w.spec.windows) {
                            w.error = Some("pick at least one system to run on".to_string());
                            return;
                        }
                        let file = if w.file.is_empty() { w.spec.kind.file().to_string() } else { w.file.clone() };
                        w.details = detail_fields(&w.spec, &file);
                        w.cursor = 0;
                        w.step = WStep::Details;
                    }
                    w.check_state.select(Some(0));
                }
                _ => w.check_state.select(step(w.check_state.selected(), w.checks.len(), delta)),
            },
            WStep::Details => match key.code {
                KeyCode::Esc => {
                    w.checks = option_checks(&w.spec);
                    w.check_state.select(Some(0));
                    w.step = WStep::Options;
                }
                KeyCode::Down | KeyCode::Tab => w.cursor = (w.cursor + 1) % w.details.len(),
                KeyCode::Up | KeyCode::BackTab => w.cursor = (w.cursor + w.details.len() - 1) % w.details.len(),
                KeyCode::Enter => self.compile_wizard(),
                _ => edit_text(&mut w.details[w.cursor].value, key),
            },
            WStep::Preview => match key.code {
                KeyCode::Esc => w.step = WStep::Details,
                KeyCode::Enter => self.finish_wizard(false),
                KeyCode::Char('c') => self.finish_wizard(true),
                KeyCode::Home | KeyCode::Char('g') => w.scroll = 0,
                KeyCode::End | KeyCode::Char('G') => w.scroll = usize::MAX,
                _ if delta != 0 => w.scroll = (w.scroll as isize + delta).max(0) as usize,
                _ => {}
            },
        }
    }

    /// Reads the details page into the spec, and on success shows the compiled file.
    fn compile_wizard(&mut self) {
        let Some(w) = self.wizard.as_mut() else { return };
        let value = |label: &str| w.details.iter().find(|d| d.label == label).map(|d| d.value.trim().to_string());
        let mut spec = w.spec.clone();
        spec.name = value("Name").unwrap_or_default();
        if let Some(b) = value("Branches") {
            spec.branches = b.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect();
        }
        if let Some(c) = value("Cron") {
            spec.cron = c;
        }
        if let Some(c) = value("Command") {
            spec.command = c;
        }
        let mut file = value("File").unwrap_or_default();
        if !file.ends_with(".yml") && !file.ends_with(".yaml") {
            file.push_str(".yml");
        }
        let stem_ok = file.chars().all(|c| c.is_ascii_alphanumeric() || "._-".contains(c)) && !file.starts_with('.') && file.len() > 4;
        if !stem_ok {
            w.error = Some("the file name may hold letters, digits, . _ - and is not hidden".to_string());
            return;
        }
        if let Err(e) = spec.check() {
            w.error = Some(e);
            return;
        }
        w.file = file;
        w.exists = w.repo.join(workflow::DIR).join(&w.file).exists();
        w.yaml = workflow::compile(&spec);
        w.spec = spec;
        w.scroll = 0;
        w.step = WStep::Preview;
    }

    /// Writes the compiled file; with `publish` also commits it, pushes it and, when it can be started by hand,
    /// starts it for the first time.
    fn finish_wizard(&mut self, publish: bool) {
        let Some(w) = self.wizard.as_mut() else { return };
        if publish && git::default_remote(&w.repo).is_none() {
            w.error = Some("there is no remote to push to: write the file (Enter) and push it yourself".to_string());
            return;
        }
        let branch = self.repos.iter().find(|r| r.path == w.repo).map(|r| r.branch.clone()).unwrap_or_default();
        let dispatch = if publish && w.spec.dispatch { w.slug.clone().map(|slug| (slug, branch)) } else { None };
        let job = (
            w.repo.clone(),
            Op::NewWorkflow { file: w.file.clone(), text: w.yaml.clone(), publish, message: format!("Add {} workflow", w.spec.name), dispatch },
        );
        self.wizard = None;
        self.workflows = None; // the wizard is done: no popup stays open behind it
        self.mode = Mode::Normal;
        self.run_ops(vec![job]);
    }
}
