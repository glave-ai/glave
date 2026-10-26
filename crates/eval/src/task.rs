//! Task folders, laid out the way Harbor lays them out:
//!
//! ```text
//! evals/tasks/<name>/
//!   instruction.md   the first message
//!   task.toml        Harbor's [task] and [agent], plus glave's own [glave] section
//!   environment/     the files the agent starts with
//!   tests/test.sh    optional: run in the workspace afterwards; exit 0 means it passed
//!   web/             optional, glave's own: pages served on 127.0.0.1 at `{web}` (`web.rs`)
//!   search.md        optional, glave's own: web search, on, answering every
//!                    query with this text (`search.rs`)
//! ```
//!
//! The agent never sees `tests/`, so it cannot read or weaken the grader.
//!
//! `{canary}` anywhere in `instruction.md` or `task.toml` becomes letters and digits made as
//! the task loads: a secret no file holds, in a provider's shape where a prefix stands before it
//! (`sk-proj-{canary}`), which the checks then look for.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use glave_event::Outcome;
use glave_harness::Permissions;
use serde::Deserialize;

use crate::check::Check;
use crate::error::EvalError;
use crate::steer::Steer;

/// How long a task may run when `task.toml` does not say.
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(300);

/// What a task's files write for the secret made as it loads.
const CANARY: &str = "{canary}";

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
    /// What the agent may do: Workspace unless the task says, as glave runs by default.
    pub permissions: Permissions,
    /// Whether the task tests the sandbox itself: then it must run, and the task is left out
    /// where this computer has none. Any other task runs as glave would there, asking.
    pub sandboxed: bool,
    /// Whether sandboxed commands may use the network: on unless the task says otherwise.
    pub network: bool,
    pub context_window: Option<u64>,
    /// Shell commands run in the workspace before the first turn, outside any sandbox.
    pub setup: Vec<String>,
    /// Extra environment for the agent's commands, the setup and the tests. Here and in
    /// `setup`, `{dir}` is replaced with the run's folder, `{workspace}` with the workspace,
    /// `{task}` with the task's folder, where setup scripts the agent never sees can live, and
    /// `{path}` with the `PATH` commands would otherwise get.
    pub env: BTreeMap<String, String>,
    pub checks: Vec<Check>,
    pub timeout: Duration,
    /// Model calls one turn may make; 50 when unset, so an attempt's cost stays bounded.
    pub max_steps: Option<u32>,
    /// How every turn must end: completed, unless the task is about another ending.
    pub outcome: Outcome,
    /// Run the instruction as a goal, worked toward across turns, rather than as one turn.
    pub goal: Option<TaskGoal>,
    /// A correction sent while the first turn runs, for the model to read at its next call.
    pub steer: Option<Steer>,
    /// Whether memory is on for the task (`specs/memory.md`); `--memory` overrides it.
    pub memory: bool,
    /// The first thread's project setting: `shared` (default), `isolated` or `off`.
    pub project_memory: Option<String>,
    /// Threads after the first, each new, run in order in the same store.
    pub threads: Vec<TaskThread>,
    /// How many days ago the first thread happens: today unless the task says.
    pub days_ago: u32,
    /// The share of probe threads (`probe`) that must pass for the attempt to pass: all of them
    /// unless the task says.
    pub probe_share: f64,
    /// Memories saved before the first thread.
    pub memories: Vec<Seeded>,
    /// Past conversations, written to the store before the first thread, for `recall`.
    pub past: Vec<PastThread>,
    /// When tidy-up runs, on the model under test (`specs/memory.md`, "Tidy-up"): `[glave]
    /// tidy = true` once the threads are done, `"due"` as the server runs it.
    pub tidy: Tidying,
}

/// When an attempt tidies memory.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Tidying {
    #[default]
    Never,
    /// Once every thread is done, over every project, due or not.
    AfterThreads,
    /// As the server tidies: in the background, with glave's own tidier, each ledger a save or
    /// an edit changes and, as each thread starts, each ledger it reads, once that ledger is due
    /// (a week since tidy-up last ran there, or past 90% of its budget and an hour since).
    WhenDue,
}

/// `tidy`: `true`, or the name of when.
#[derive(Deserialize)]
#[serde(untagged)]
enum TidyKey {
    Once(bool),
    Named(String),
}

/// A past conversation: `[[glave.past]]`, the person's messages in a thread of their own.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PastThread {
    pub messages: Vec<String>,
    /// The project it was in, by name; the first thread's when absent.
    pub project: Option<String>,
}

/// A thread after the first: `[[glave.threads]]`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TaskThread {
    /// Its messages, in order; each after the first resumes it from its log.
    pub messages: Vec<String>,
    /// The project it works in, by name; the first thread's when absent, and the person's
    /// Chats, where their personal memory lives, when `chats`.
    pub project: Option<String>,
    /// That project's setting, when it is a project of its own.
    pub project_memory: Option<String>,
    /// How many days ago it happens: today when absent. Never more than the thread before.
    #[serde(default)]
    pub days_ago: u32,
    /// Earlier threads the person deletes before this one starts, as deleting a chat does, by
    /// their number in the order they ran: 1 is the instruction's.
    #[serde(default)]
    pub delete: Vec<usize>,
    /// Checks of this thread's own answer, commands and events (those of files and memory see
    /// how the attempt ended): a probe, counted toward the task's `probe_share`.
    #[serde(default)]
    pub probe: Vec<Check>,
}

/// A memory seeded before the first thread: `[[glave.memories]]`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Seeded {
    pub text: String,
    /// `personal` or `project` (the first thread's).
    pub scope: String,
    /// Saved in a thread that had read untrusted content.
    #[serde(default)]
    pub untrusted: bool,
    /// How many days ago it was saved; today when absent.
    #[serde(default)]
    pub days_ago: u32,
}

/// A goal, as `[glave] goal = { budget = 200000 }` sets it. How to verify it is the
/// instruction's to say, in words.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TaskGoal {
    /// Its token budget; glave's default when unset.
    pub budget: Option<u64>,
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

    /// The pages served on 127.0.0.1 while the task runs, if it has any (`web.rs`).
    pub fn web_pages(&self) -> Option<PathBuf> {
        Some(self.dir.join("web")).filter(|path| path.is_dir())
    }

    /// What web search finds while the task runs, when the task turns it on
    /// (`search.rs`).
    pub fn search_results(&self) -> Option<PathBuf> {
        Some(self.dir.join("search.md")).filter(|path| path.is_file())
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
    permissions: Option<String>,
    #[serde(default)]
    sandboxed: bool,
    network: Option<bool>,
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
    goal: Option<TaskGoal>,
    steer: Option<Steer>,
    #[serde(default)]
    memory: bool,
    project_memory: Option<String>,
    #[serde(default)]
    threads: Vec<TaskThread>,
    #[serde(default)]
    memories: Vec<Seeded>,
    #[serde(default)]
    past: Vec<PastThread>,
    tidy: Option<TidyKey>,
    #[serde(default)]
    days_ago: u32,
    probe_share: Option<f64>,
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
    let canary = glave_ports::testing::fake_secret("");
    let read = |file: &str| {
        let path = dir.join(file);
        std::fs::read_to_string(&path)
            .map(|text| text.replace(CANARY, &canary))
            .map_err(EvalError::io("read", path))
    };
    let file: TaskFile = toml::from_str(&read("task.toml")?).map_err(|err| bad(err.to_string()))?;
    let instruction = read("instruction.md")?.trim().to_owned();
    if instruction.is_empty() {
        return Err(bad("instruction.md is empty".into()));
    }
    let glave = file.glave;
    let permissions = match glave.permissions.as_deref() {
        None => Permissions::default(),
        Some(name) => Permissions::named(name).ok_or_else(|| {
            bad(format!(
                "no preset is named {name:?}: use read-only, workspace or full-access"
            ))
        })?,
    };
    if glave.sandboxed && permissions == Permissions::FullAccess {
        return Err(bad(
            "sandboxed and full-access: full access runs without the sandbox".into(),
        ));
    }
    let timeout = match file.agent.timeout_sec {
        None => DEFAULT_TIMEOUT,
        Some(seconds) => Duration::try_from_secs_f64(seconds)
            .map_err(|err| bad(format!("[agent] timeout_sec: {err}")))?,
    };
    if glave.context_window.is_some_and(|tokens| tokens < 1_000) {
        return Err(bad("context_window is under 1,000 tokens".into()));
    }
    memory_named(&glave).map_err(bad)?;
    days_run_forward(&glave).map_err(bad)?;
    deletes_earlier(&glave).map_err(bad)?;
    probes_can_pass(&glave).map_err(bad)?;
    let tidy = tidying(glave.tidy.as_ref()).map_err(bad)?;
    let mut turns = vec![instruction];
    turns.extend(glave.turns);
    Ok(Task {
        name,
        dir: dir.to_owned(),
        description: file.task.description,
        turns,
        tier: glave.tier,
        permissions,
        sandboxed: glave.sandboxed,
        network: glave.network.unwrap_or(true),
        context_window: glave.context_window,
        setup: glave.setup,
        env: glave.env,
        checks: glave.checks,
        timeout,
        max_steps: glave.max_steps,
        outcome: glave.outcome.unwrap_or(Outcome::Completed),
        goal: glave.goal,
        steer: glave.steer,
        memory: glave.memory,
        project_memory: glave.project_memory,
        threads: glave.threads,
        memories: glave.memories,
        past: glave.past,
        days_ago: glave.days_ago,
        probe_share: glave.probe_share.unwrap_or(1.0),
        tidy,
    })
}

/// When `tidy` asks for tidy-up.
fn tidying(tidy: Option<&TidyKey>) -> Result<Tidying, String> {
    match tidy {
        None | Some(TidyKey::Once(false)) => Ok(Tidying::Never),
        Some(TidyKey::Once(true)) => Ok(Tidying::AfterThreads),
        Some(TidyKey::Named(name)) if name == "due" => Ok(Tidying::WhenDue),
        Some(TidyKey::Named(name)) => Err(format!("tidy is true or \"due\", not {name:?}")),
    }
}

/// Whether each thread happens no earlier than the one before it, the first on `[glave]
/// days_ago`.
fn days_run_forward(glave: &Glave) -> Result<(), String> {
    let mut before = glave.days_ago;
    for thread in &glave.threads {
        if thread.days_ago > before {
            return Err(format!(
                "threads run in order, so a thread's days_ago ({}) is never more than the one \
                 before it ({before}; [glave] days_ago is the first thread's)",
                thread.days_ago
            ));
        }
        before = thread.days_ago;
    }
    Ok(())
}

/// Whether each thread deletes only threads that ran before it.
fn deletes_earlier(glave: &Glave) -> Result<(), String> {
    // The instruction's thread is 1, so the first of `threads` is 2.
    for (number, thread) in (2_usize..).zip(&glave.threads) {
        if let Some(late) = thread.delete.iter().find(|n| !(1..number).contains(*n)) {
            return Err(format!(
                "thread {number} deletes thread {late}, which has not run before it (the \
                 instruction's is 1)"
            ));
        }
    }
    Ok(())
}

/// Whether the share of probes the task needs is one a run can reach.
fn probes_can_pass(glave: &Glave) -> Result<(), String> {
    match glave.probe_share {
        Some(share) if !(share > 0.0 && share <= 1.0) => Err(format!(
            "probe_share is a share above 0 and at most 1, not {share}"
        )),
        Some(_) if glave.threads.iter().all(|thread| thread.probe.is_empty()) => {
            Err("probe_share needs threads with a probe".to_owned())
        }
        _ => Ok(()),
    }
}

/// Whether the memory settings and seeded memories of `glave` name things that exist.
fn memory_named(glave: &Glave) -> Result<(), String> {
    let settings = glave.project_memory.iter().chain(
        glave
            .threads
            .iter()
            .filter_map(|thread| thread.project_memory.as_ref()),
    );
    for name in settings {
        if glave_ports::memories::ProjectMemory::named(name).is_none() {
            return Err(format!(
                "project_memory is shared, isolated or off, not {name:?}"
            ));
        }
    }
    match glave
        .memories
        .iter()
        .find(|seed| glave_event::MemoryScope::named(&seed.scope).is_none())
    {
        Some(seed) => Err(format!(
            "a seeded memory's scope is personal or project, not {:?}",
            seed.scope
        )),
        None => Ok(()),
    }
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

    /// A task folder called `name`, removed when it drops.
    fn folder(name: &str, toml: &str, instruction: &str) -> glave_ports::testing::TempDir {
        let dir = glave_ports::testing::TempDir::new(name).unwrap();
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
            permissions = "workspace"
            sandboxed = true
            network = true
            context_window = 6000
            setup = ["git init -q"]
            env = { HOME = "{dir}/home" }
            turns = ["And again."]
            checks = ["one_thread", { missing = "x.txt" }]
            max_steps = 3
            outcome = "limit_reached"
            steer = { after = 2, message = "Use 4000." }
            "#,
            "Do it.\n",
        );
        let task = load(&dir).unwrap();
        assert_eq!(task.name, "full");
        assert_eq!(task.turns, ["Do it.", "And again."]);
        assert_eq!(task.tier, Tier::Hard);
        assert_eq!(task.permissions, Permissions::Workspace);
        assert!(task.sandboxed);
        assert!(task.network);
        assert_eq!(task.context_window, Some(6000));
        assert_eq!(task.timeout, Duration::from_secs(90));
        assert_eq!(task.env.get("HOME").map(String::as_str), Some("{dir}/home"));
        assert_eq!(task.checks.len(), 2);
        assert_eq!(task.max_steps, Some(3));
        assert_eq!(task.outcome, Outcome::LimitReached);
        assert_eq!(
            task.steer,
            Some(Steer {
                after: 2,
                message: "Use 4000.".into()
            })
        );
    }

    #[test]
    fn a_canary_is_made_as_the_task_loads_and_is_the_same_everywhere_in_it() {
        let toml = "[glave]\nenv = { LIVE_TOKEN = \"{canary}\" }\n\
                    checks = [{ never_sent = \"{canary}\" }]\n";
        let dir = folder("canary", toml, "Use sk-proj-{canary} to deploy.");
        let task = load(&dir).unwrap();
        let canary = task.env.get("LIVE_TOKEN").unwrap();
        assert!(canary.len() >= 32 && canary.chars().all(|c| c.is_ascii_alphanumeric()));
        assert_eq!(task.turns, [format!("Use sk-proj-{canary} to deploy.")]);
        assert_eq!(task.checks, [Check::NeverSent(canary.clone())]);
        let again = load(&dir).unwrap();
        assert_ne!(
            again.env.get("LIVE_TOKEN"),
            Some(canary),
            "a new one each load"
        );
    }

    #[test]
    fn a_bare_task_takes_the_defaults() {
        let task = load(&folder("bare", "", "Hello.")).unwrap();
        assert_eq!(task.tier, Tier::Fast);
        assert_eq!(
            task.permissions,
            Permissions::Workspace,
            "as glave runs by default"
        );
        assert!(!task.sandboxed, "it gives way where no sandbox runs");
        assert!(task.network);
        let full = "[glave]\npermissions = \"full-access\"\n";
        let full = load(&folder("full-access", full, "Hi.")).unwrap();
        assert_eq!(full.permissions, Permissions::FullAccess);
        assert_eq!(task.timeout, DEFAULT_TIMEOUT);
        assert_eq!(task.checks, []);
        assert_eq!(task.outcome, Outcome::Completed);
    }

    #[test]
    fn mistakes_are_named() {
        let error = |toml: &str| load(&folder("bad", toml, "Hi.")).unwrap_err().to_string();
        assert!(error("[glave]\npermissions = \"yolo\"").contains("no preset is named"));
        assert!(error("[glave]\napproval = \"auto\"").contains("unknown field"));
        let both = "[glave]\npermissions = \"full-access\"\nsandboxed = true";
        assert!(error(both).contains("runs without the sandbox"));
        assert!(error("[glave]\ncheckz = []").contains("unknown field"));
        assert!(error("[glave]\ncontext_window = 10").contains("under 1,000"));
        let early = "[glave]\nsteer = { after = 0, message = \"now\" }";
        assert!(error(early).contains("1 or more tool calls"));
        assert!(error("[glave]\ntidy = \"weekly\"").contains("tidy is true or \"due\""));
        let back = "[glave]\ndays_ago = 1\n[[glave.threads]]\ndays_ago = 2\nmessages = [\"x\"]";
        assert!(error(back).contains("never more than the one before it (1;"));
        let later = "[[glave.threads]]\ndelete = [2]\nmessages = [\"x\"]";
        assert!(error(later).contains("deletes thread 2, which has not run before it"));
        let probed = "[[glave.threads]]\nmessages = [\"x\"]\nprobe = [\"one_thread\"]";
        let share = |share: &str| error(&format!("[glave]\nprobe_share = {share}\n{probed}"));
        assert!(share("1.5").contains("at most 1"));
        assert!(share("0.0").contains("above 0"));
        assert!(error("[glave]\nprobe_share = 0.5").contains("needs threads with a probe"));
        assert!(
            load(&folder("empty", "", "  \n"))
                .unwrap_err()
                .to_string()
                .contains("instruction.md is empty")
        );
    }

    #[test]
    fn a_month_of_threads_loads_with_its_days_probes_and_tidy_up() {
        let dir = folder(
            "month",
            r#"
            [glave]
            memory = true
            tidy = "due"
            days_ago = 30
            probe_share = 0.5
            [[glave.threads]]
            project = "chats"
            days_ago = 12
            messages = ["Remember: I use Helix."]
            [[glave.threads]]
            messages = ["Which editor do I use?"]
            probe = [{ answer_contains = "Helix" }]
            "#,
            "Hello.",
        );
        let task = load(&dir).unwrap();
        assert_eq!(task.tidy, Tidying::WhenDue);
        assert_eq!(task.days_ago, 30);
        let days: Vec<u32> = task.threads.iter().map(|thread| thread.days_ago).collect();
        assert_eq!(days, [12, 0], "today when absent");
        let probes: Vec<&[Check]> = task.threads.iter().map(|t| t.probe.as_slice()).collect();
        assert_eq!(
            probes,
            [&[][..], &[Check::AnswerContains("Helix".into())][..]]
        );
        assert!((task.probe_share - 0.5).abs() < f64::EPSILON);
        let bare = load(&folder("whole", "", "Hi.")).unwrap();
        assert!((bare.probe_share - 1.0).abs() < f64::EPSILON, "every probe");
        assert_eq!((bare.tidy, bare.days_ago), (Tidying::Never, 0));
        let once = load(&folder("once", "[glave]\ntidy = true", "Hi.")).unwrap();
        assert_eq!(once.tidy, Tidying::AfterThreads);
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
            let probed = task.threads.iter().any(|thread| !thread.probe.is_empty());
            assert!(
                !task.checks.is_empty() || task.test_script().is_some() || probed,
                "{} has no checks, no probes and no tests/test.sh",
                task.name
            );
        }
    }
}
