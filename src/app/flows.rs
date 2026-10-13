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
                let Some(w) = picked else {
                    self.log(Level::Info, "no workflow highlighted");
                    return;
                };
                match key.code {
                    KeyCode::Enter | KeyCode::Char('r') => self.open_run_form(w),
                    KeyCode::Char('o') => match slug {
                        Some(slug) => {
                            let args = vec!["workflow".to_string(), "view".into(), w.file.clone(), "-R".into(), slug, "--web".into()];
                            self.spawn_detached("gh", &args, &format!("{} opened in the browser", w.file));
                        }
                        None => self.log(Level::Warn, "opening a workflow needs a github.com remote"),
                    },
                    KeyCode::Char('e') => {
                        let target = repo.join(workflow::DIR).join(&w.file);
                        self.launch = Some(Launch::Browse { repo, target });
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
