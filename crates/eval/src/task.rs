//! Task folders, laid out the way Harbor lays them out:
//!
//! ```text
//! evals/tasks/<name>/
//!   instruction.md   the first message
//!   task.toml        Harbor's [task] and [agent], plus glave's own [glave] section
//!   environment/     the files the agent starts with
//!   tests/test.sh    optional: run in the workspace afterwards; exit 0 means it passed
//! ```
//!
//! The agent never sees `tests/`, so it cannot read or weaken the grader.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use glave_event::Outcome;
use glave_harness::{ApprovalMode, SandboxLevel};
use serde::Deserialize;

use crate::check::Check;
use crate::error::EvalError;

/// How long a task may run when `task.toml` does not say.
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(300);

/// One task, loaded and checked.
#[derive(Debug, Clone)]
pub struct Task {
    /// The folder's name.
    pub name: String,
    pub dir: PathBuf,
    pub description: String,
    /// The messages sent, in order; each one after the first resumes the thread from its log,
    /// the way a person returns to it later.
    pub turns: Vec<String>,
    pub tier: Tier,
    pub approval: ApprovalMode,
    pub sandbox: Option<SandboxLevel>,
    pub network: bool,
    pub context_window: Option<u64>,
    /// Shell commands run in the workspace before the first turn, outside any sandbox.
    pub setup: Vec<String>,
    /// Extra environment for the agent's commands, the setup and the tests. Here and in
    /// `setup`, `{dir}` is replaced with the run's folder, `{workspace}` with the workspace
    /// and `{task}` with the task's folder, where setup scripts the agent never sees can live.
    pub env: BTreeMap<String, String>,
    pub checks: Vec<Check>,
    pub timeout: Duration,
    /// Model calls one turn may make; the harness's default when unset.
    pub max_steps: Option<u32>,
    /// How every turn must end: completed, unless the task is about another ending.
    pub outcome: Outcome,
}

/// How hard a task is: the fast tier runs on every change the model feels; the hard tier
/// before releases and when comparing models.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Tier {
    #[default]
    Fast,
    Hard,
}

impl Task {
    /// The starting files, copied into each run's workspace.
    pub fn environment(&self) -> PathBuf {
        self.dir.join("environment")
    }

    /// The grader, if the task has one.
    pub fn test_script(&self) -> Option<PathBuf> {
        Some(self.dir.join("tests").join("test.sh")).filter(|path| path.is_file())
    }
}

#[derive(Deserialize)]
struct TaskFile {
    #[serde(default)]
    task: About,
    #[serde(default)]
    agent: Agent,
    #[serde(default)]
    glave: Glave,
}

/// Harbor's `[task]`; only the description is used.
#[derive(Default, Deserialize)]
struct About {
    #[serde(default)]
    description: String,
}

/// Harbor's `[agent]`.
#[derive(Default, Deserialize)]
struct Agent {
    timeout_sec: Option<f64>,
}

/// glave's own section. Harbor ignores it; typos here are errors.
#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct Glave {
    #[serde(default)]
    tier: Tier,
    approval: Option<String>,
    sandbox: Option<String>,
    #[serde(default)]
    network: bool,
    context_window: Option<u64>,
    #[serde(default)]
    setup: Vec<String>,
    #[serde(default)]
    env: BTreeMap<String, String>,
    /// Messages after the instruction.
    #[serde(default)]
    turns: Vec<String>,
    #[serde(default)]
    checks: Vec<Check>,
    max_steps: Option<u32>,
    outcome: Option<Outcome>,
}

/// Loads the task in `dir`. Its paths are absolute, since the grader runs from the workspace.
pub fn load(dir: &Path) -> Result<Task, EvalError> {
    let dir = &std::fs::canonicalize(dir).map_err(EvalError::io("find", dir))?;
    let bad = |message: String| EvalError::Task {
        dir: dir.to_owned(),
        message,
    };
    let name = dir
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| bad("the folder name is not text".into()))?
        .to_owned();
    let read = |file: &str| {
        let path = dir.join(file);
        std::fs::read_to_string(&path).map_err(EvalError::io("read", path))
    };
    let file: TaskFile = toml::from_str(&read("task.toml")?).map_err(|err| bad(err.to_string()))?;
    let instruction = read("instruction.md")?.trim().to_owned();
    if instruction.is_empty() {
        return Err(bad("instruction.md is empty".into()));
    }
    let glave = file.glave;
    let approval = match glave.approval.as_deref() {
        None => ApprovalMode::default(),
        Some(name) => ApprovalMode::named(name)
            .ok_or_else(|| bad(format!("no approval mode is named {name:?}")))?,
    };
    let sandbox = match glave.sandbox.as_deref() {
        None => None,
        Some(name) => Some(
            SandboxLevel::named(name)
                .ok_or_else(|| bad(format!("no sandbox is named {name:?}")))?,
        ),
    };
    let timeout = match file.agent.timeout_sec {
        None => DEFAULT_TIMEOUT,
        Some(seconds) => Duration::try_from_secs_f64(seconds)
            .map_err(|err| bad(format!("[agent] timeout_sec: {err}")))?,
    };
    if glave.context_window.is_some_and(|tokens| tokens < 1_000) {
        return Err(bad("context_window is under 1,000 tokens".into()));
    }
    let mut turns = vec![instruction];
    turns.extend(glave.turns);
    Ok(Task {
        name,
        dir: dir.to_owned(),
        description: file.task.description,
        turns,
        tier: glave.tier,
        approval,
        sandbox,
        network: glave.network,
        context_window: glave.context_window,
        setup: glave.setup,
        env: glave.env,
        checks: glave.checks,
        timeout,
        max_steps: glave.max_steps,
        outcome: glave.outcome.unwrap_or(Outcome::Completed),
    })
}

/// Loads every task folder under `root`, sorted by name.
pub fn load_all(root: &Path) -> Result<Vec<Task>, EvalError> {
    let entries = std::fs::read_dir(root).map_err(EvalError::io("list", root))?;
    let mut tasks = Vec::new();
    for entry in entries {
        let path = entry.map_err(EvalError::io("list", root))?.path();
        if path.join("task.toml").is_file() {
            tasks.push(load(&path)?);
        }
    }
    tasks.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(tasks)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn folder(name: &str, toml: &str, instruction: &str) -> PathBuf {
        let dir = std::env::temp_dir()
            .join(format!("glave-eval-task-{}", std::process::id()))
            .join(name);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("task.toml"), toml).unwrap();
        std::fs::write(dir.join("instruction.md"), instruction).unwrap();
        dir
    }

    #[test]
    fn a_full_task_loads() {
        let dir = folder(
            "full",
            r#"
            schema_version = "1.3"
            [task]
            name = "glave/full"
            description = "Everything set."
            [agent]
            timeout_sec = 90
            [glave]
            tier = "hard"
            approval = "auto"
            sandbox = "project"
            network = true
            context_window = 6000
            setup = ["git init -q"]
            env = { HOME = "{dir}/home" }
            turns = ["And again."]
            checks = ["one_thread", { missing = "x.txt" }]
            max_steps = 3
            outcome = "limit_reached"
            "#,
            "Do it.\n",
        );
        let task = load(&dir).unwrap();
        assert_eq!(task.name, "full");
        assert_eq!(task.turns, ["Do it.", "And again."]);
        assert_eq!(task.tier, Tier::Hard);
        assert_eq!(task.approval, ApprovalMode::Auto);
        assert_eq!(task.sandbox, Some(SandboxLevel::Project));
        assert!(task.network);
        assert_eq!(task.context_window, Some(6000));
        assert_eq!(task.timeout, Duration::from_secs(90));
        assert_eq!(task.env.get("HOME").map(String::as_str), Some("{dir}/home"));
        assert_eq!(task.checks.len(), 2);
        assert_eq!(task.max_steps, Some(3));
        assert_eq!(task.outcome, Outcome::LimitReached);
    }

    #[test]
    fn a_bare_task_takes_the_defaults() {
        let task = load(&folder("bare", "", "Hello.")).unwrap();
        assert_eq!(task.tier, Tier::Fast);
        assert_eq!(task.approval, ApprovalMode::Edits);
        assert_eq!(task.sandbox, None);
        assert_eq!(task.timeout, DEFAULT_TIMEOUT);
        assert_eq!(task.checks, []);
        assert_eq!(task.outcome, Outcome::Completed);
    }

    #[test]
    fn mistakes_are_named() {
        let error = |toml: &str| load(&folder("bad", toml, "Hi.")).unwrap_err().to_string();
        assert!(error("[glave]\napproval = \"yolo\"").contains("no approval mode is named"));
        assert!(error("[glave]\nsandbox = \"all\"").contains("no sandbox is named"));
        assert!(error("[glave]\ncheckz = []").contains("unknown field"));
        assert!(error("[glave]\ncontext_window = 10").contains("under 1,000"));
        assert!(
            load(&folder("empty", "", "  \n"))
                .unwrap_err()
                .to_string()
                .contains("instruction.md is empty")
        );
    }

    /// Every task in the repository loads and grades something beyond finishing, so a broken
    /// task fails CI, not an eval run.
    #[test]
    fn the_repository_tasks_load() {
        // Relative, as `cargo xtask eval` names it; tests run from the crate's folder.
        let tasks = load_all(Path::new("../../evals/tasks")).unwrap();
        assert!(!tasks.is_empty());
        for task in &tasks {
            assert!(task.dir.is_absolute(), "{}", task.dir.display());
            assert!(
                !task.checks.is_empty() || task.test_script().is_some(),
                "{} has no checks and no tests/test.sh",
                task.name
            );
        }
    }
}
