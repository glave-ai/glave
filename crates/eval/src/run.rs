//! One attempt: a fresh copy of the task's files, the harness wired in-process to a real model,
//! every turn sent, then the grading. Everything it leaves behind stays in the attempt's folder.

use std::ffi::OsString;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use glave_computer::LocalComputers;
use glave_harness::{
    ApprovalMode, Cancel, DEFAULT_BUDGET, Goal, Harness, Live, NoOneToAsk, Parts, Settings, Skills,
    Todo, Workspace, agents_md, find_skills, skill_folders,
};
use glave_memory::Tidier;
use glave_ports::computer::Sandbox;
use glave_ports::settings::{Domain, WebPolicy};
use glave_ports::{Caller, EventLog, OsEntropy, Scope, SystemClock, ToolSource};
use glave_provider_openai::{Config, OpenAiCompatible};
use glave_store_sqlite::{SqliteBlobs, SqliteLog};
use glave_web::pages::Pages;
use glave_web::{Limits, Web, WebSettings};
use serde::{Deserialize, Serialize};

use crate::calendar::Calendar;
use crate::check::{Context, ended};
use crate::error::EvalError;
use crate::memory::{MemoryReport, MemoryUse, Remembering};
use crate::prepare::{Environment, Prepared, last_lines, prepare, script, tidy};
use crate::probe::{self, Probes};
use crate::steer::{Steer, Steering};
use crate::task::{Task, Tidying};
use crate::trace::{self, Trace};

/// Model calls one turn of an attempt may make, unless its task says otherwise.
const MAX_STEPS: u32 = 50;

/// The harness as an attempt wires it.
type Wired = Harness<OpenAiCompatible, SqliteLog, SqliteBlobs, NoOneToAsk>;

/// What every attempt runs with.
pub struct Setup {
    /// The model endpoint. Its key is never printed; it is only searched for in the events.
    pub endpoint: Config,
    /// The environment commands start from, before the task's own and minus secrets.
    pub environment: Vec<(OsString, OsString)>,
    /// Whether the model is offered the todo list, to measure what it does.
    pub todo: bool,
    /// Whether the model may hand work to child threads with `task`, to measure what it does.
    pub subagents: bool,
    /// Memory on or off for every task, to measure what it does; each task's own when `None`.
    pub memory: Option<bool>,
}

/// How one attempt went.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Attempt {
    pub task: String,
    pub model: String,
    /// 1 for the first run of this task on this model.
    pub number: u32,
    /// Not run, because the cost cap was reached first.
    pub skipped: bool,
    /// Empty when the attempt passed.
    pub failures: Vec<String>,
    pub model_calls: u32,
    pub tool_calls: u32,
    pub tool_errors: u32,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cached_tokens: u64,
    /// Micros of the reported currency; `None` when any call reported no cost.
    pub cost_micros: Option<i64>,
    pub seconds: f64,
    /// How the task's probes went, when it has them.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub probes: Option<Probes>,
    /// What memory held and cost, when it was on.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub memory: Option<MemoryUse>,
}

impl Attempt {
    pub fn passed(&self) -> bool {
        !self.skipped && self.failures.is_empty()
    }

    /// An attempt that never ran, or could not.
    pub fn unrun(task: &str, model: &str, number: u32, skipped: bool, failure: String) -> Self {
        let mut attempt = Self::graded(task, model, number, &Trace::default(), 0.0);
        attempt.cost_micros = None;
        attempt.skipped = skipped;
        attempt.failures.push(failure);
        attempt
    }

    fn graded(task: &str, model: &str, number: u32, trace: &Trace, seconds: f64) -> Self {
        Self {
            task: task.to_owned(),
            model: model.to_owned(),
            number,
            skipped: false,
            failures: Vec::new(),
            model_calls: trace.model_calls,
            tool_calls: trace.tool_calls,
            tool_errors: trace.tool_errors,
            input_tokens: trace.input_tokens,
            output_tokens: trace.output_tokens,
            cached_tokens: trace.cached_tokens,
            cost_micros: trace.cost_micros,
            seconds,
            probes: None,
            memory: None,
        }
    }
}

/// Runs `task` on `model` in `dir`, keeping the workspace, events, transcript and test output
/// there. An `Err` means the attempt could not be set up, not that the model failed.
pub async fn attempt(
    setup: &Setup,
    task: &Task,
    model: &str,
    number: u32,
    dir: &Path,
) -> Result<Attempt, EvalError> {
    let Prepared {
        workspace,
        environment,
    } = prepare(&setup.environment, task, dir).await?;
    // The task's pages, served until the attempt is graded.
    let pages = match task.web_pages() {
        Some(pages) => Some(crate::web::serve(&pages).await?),
        None => None,
    };
    let web = pages.as_ref().map(Pages::base);
    let start = Instant::now();
    let Conversed {
        threads,
        transcript,
        mut failures,
        memory,
    } = converse(
        setup,
        task,
        model,
        &environment,
        (&workspace, web.as_deref()),
    )
    .await?;
    let seconds = start.elapsed().as_secs_f64();
    let events = threads.concat();
    write(dir.join("events.jsonl"), &events).await?;
    write(dir.join("transcript.txt"), &transcript).await?;
    write(dir.join("memory.jsonl"), &memory.ledger).await?;
    failures.extend(leaked(setup, &events));
    let trace = trace::parse(&events).unwrap_or_else(|err| {
        failures.push(err);
        Trace::default()
    });
    // A goal's turns may end at the step limit and go on; its own checks grade how it ended.
    if task.goal.is_none() {
        failures.extend(ended(task.outcome, &trace.outcomes).err());
    }
    failures.extend(tested(task, (&workspace, &environment), dir).await?);
    let last = (threads.last())
        .and_then(|thread| trace::parse(thread).ok())
        .unwrap_or_default();
    let cx = Context {
        trace: &trace,
        last: &last,
        events: &events,
        workspace: &workspace,
        original: &task.environment(),
        model,
        memory: &memory,
        requested: &pages.as_ref().map(Pages::requested).unwrap_or_default(),
    };
    failures.extend(
        task.checks
            .iter()
            .filter_map(|check| check.judge(&cx).err()),
    );
    let probes = probe::grade(task, &threads, &cx);
    failures.extend((probes.as_ref()).and_then(|probes| probes.short_of(task.probe_share)));
    tidy(dir, task).await;
    Ok(Attempt {
        failures,
        probes,
        memory: memory.usage,
        ..Attempt::graded(&task.name, model, number, &trace, seconds)
    })
}

/// The failure when the endpoint's API key is in `events`: it reached the model.
fn leaked(setup: &Setup, events: &str) -> Option<String> {
    let key = (setup.endpoint.auth.fixed_key()).map(glave_ports::auth::Secret::expose)?;
    (key.len() >= 8 && events.contains(key))
        .then(|| "the endpoint's API key reached the model: rotate it now".to_owned())
}

/// Runs the task's grader, if it has one, in the workspace, keeping its output in `dir`; the
/// failure, when it failed.
async fn tested(
    task: &Task,
    (workspace, environment): (&Path, &Environment),
    dir: &Path,
) -> Result<Option<String>, EvalError> {
    let Some(test) = task.test_script() else {
        return Ok(None);
    };
    let test = test.display().to_string();
    let output = script("bash", &[test.as_str()], workspace, environment).await;
    tidy(dir, task).await;
    let output = output?;
    write(dir.join("test.log"), &output.text).await?;
    Ok((!output.success).then(|| format!("tests/test.sh failed: {}", last_lines(&output.text))))
}

/// The workspace in `root`, in the sandbox the task runs in, and the approval mode it runs
/// with. As glave does: where the default sandbox cannot run, commands run without it and ask
/// (with no one to ask, they are not run); a sandbox the task names never gives way.
async fn workspace(
    task: &Task,
    environment: &Environment,
    root: &Path,
) -> Result<(Workspace, ApprovalMode), EvalError> {
    let open = || async {
        let (computers, info) = LocalComputers::here(root, environment.clone())
            .await
            .map_err(|error| EvalError::io("open", root)(std::io::Error::other(error)))?;
        Ok::<_, EvalError>(Workspace::new(computers, info))
    };
    let Some(level) = task.sandbox else {
        return Ok((open().await?, task.approval));
    };
    let sandbox = Sandbox {
        level,
        network: task.network,
        home: environment.get(&OsString::from("HOME")).map(PathBuf::from),
        glave_home: None,
    };
    match open().await?.with_sandbox(&sandbox).await {
        Ok(workspace) => Ok((workspace, task.approval)),
        Err(_) if !task.sandbox_named => Ok((open().await?, ApprovalMode::Ask)),
        Err(error) => Err(EvalError::io("sandbox", root)(std::io::Error::other(error))),
    }
}

/// The harness for one attempt, wired in-process: a real provider, an in-memory log, the
/// workspace (sandboxed when the task asks), the attempt's calendar and no one to approve
/// anything.
async fn wire(
    setup: &Setup,
    task: &Task,
    model: &str,
    environment: &Environment,
    (root, web): (&Path, Option<&str>),
    (remembering, calendar): (&Remembering, &Calendar),
) -> Result<Wired, EvalError> {
    let (workspace, approval) = workspace(task, environment, root).await?;
    let provider = OpenAiCompatible::new(setup.endpoint.clone())
        .map_err(|err| EvalError::Setup(err.to_string()))?;
    let mut settings = Settings::new(model, setup.endpoint.provider.clone());
    settings.instructions = agents_md(root, None)
        .await
        .map_err(EvalError::io("read the instructions in", root))?
        .map(|loaded| loaded.text);
    settings.approval = approval;
    settings.subagents = setup.subagents;
    settings.person = Some(crate::memory::person()?);
    // As the CLI runs it: the model under test also judges flagged actions in auto mode.
    settings.judge = glave_ports::settings::Judge::Main;
    // The harness has no cap of its own, so an attempt gets one and its cost stays bounded.
    settings.max_steps = Some(task.max_steps.unwrap_or(MAX_STEPS));
    if let Some(tokens) = task.context_window {
        settings.compaction.window_tokens = tokens;
    }
    let parts = Parts {
        provider,
        log: remembering.store.log(),
        blobs: remembering.store.blobs(),
        approver: NoOneToAsk,
        clock: Box::new(calendar.clone()),
        entropy: Box::new(OsEntropy),
        workspace,
    };
    // web_fetch, as the CLI offers it; a task's own pages are on 127.0.0.1, named exactly.
    let mut policy = WebPolicy::default();
    if web.is_some() {
        policy.allow = ["127.0.0.1", "*"]
            .iter()
            .filter_map(|pattern| Domain::parse(pattern).ok())
            .collect();
    }
    let web = Web::new(WebSettings {
        policy,
        limits: Limits::default(),
    })
    .map_err(|err| EvalError::Setup(err.to_string()))?;
    let mut tools: Vec<Arc<dyn ToolSource>> = vec![Arc::new(web)];
    // The project's skills, as the CLI finds them; the person's own are not part of a task.
    let skills = find_skills(&skill_folders(root, &[])).await;
    if let Some(warning) = skills.warnings.first() {
        return Err(EvalError::Setup(format!("{}: skill {warning}", task.name)));
    }
    if let Some(skills) = Skills::offer(skills.skills) {
        tools.push(Arc::new(skills));
    }
    if setup.todo {
        tools.push(Arc::new(Todo));
    }
    let memory = remembering
        .memory
        .clone()
        .map(|memory| memory as Arc<dyn glave_ports::memories::Memory>);
    Ok(Harness::new(parts, settings)
        .with_tools(tools)
        .with_memory_if(memory))
}

/// What one attempt left: each thread's events as JSON Lines, in the order they ran, a readable
/// transcript, anything that stopped the conversation, and what memory holds.
struct Conversed {
    threads: Vec<String>,
    transcript: String,
    failures: Vec<String>,
    memory: MemoryReport,
}

/// One thread to run: in which project, how many days ago, and its messages.
struct Planned {
    project: Option<String>,
    days_ago: u32,
    turns: Vec<String>,
}

/// Every thread of `task`, the instruction's first, with `web`'s address in their messages.
fn planned(task: &Task, web: Option<&str>) -> Vec<Planned> {
    let fill =
        |turn: &String| web.map_or_else(|| turn.clone(), |base| crate::web::fill(turn, base));
    let first = Planned {
        project: None,
        days_ago: task.days_ago,
        turns: task.turns.iter().map(fill).collect(),
    };
    let later = task.threads.iter().map(|thread| Planned {
        project: thread.project.clone(),
        days_ago: thread.days_ago,
        turns: thread.messages.iter().map(fill).collect(),
    });
    std::iter::once(first).chain(later).collect()
}

/// Sends every turn of every thread, in order, each on its day, each turn after a thread's
/// first to the thread resumed from the log, all in one store; tidy-up runs when the task says.
async fn converse(
    setup: &Setup,
    task: &Task,
    model: &str,
    environment: &Environment,
    (root, web): (&Path, Option<&str>),
) -> Result<Conversed, EvalError> {
    let calendar = Calendar::new(Arc::new(SystemClock));
    let on = setup.memory.unwrap_or(task.memory);
    let remembering = Remembering::open(task, on, &calendar).await?;
    let wiring = (&remembering, &calendar);
    let harness = wire(setup, task, model, environment, (root, web), wiring).await?;
    let tidier = match task.tidy {
        Tidying::Never => None,
        Tidying::AfterThreads | Tidying::WhenDue => Some(Tidier {
            provider: OpenAiCompatible::new(setup.endpoint.clone())
                .map_err(|err| EvalError::Setup(err.to_string()))?,
            model: model.to_owned(),
        }),
    };
    let mut conversation = Talk {
        harness: &harness,
        task,
        deadline: Instant::now() + task.timeout,
        transcript: String::new(),
        failures: Vec::new(),
    };
    let (mut threads, mut in_view) = (Vec::new(), Vec::new());
    for (index, plan) in planned(task, web).iter().enumerate() {
        let scope = crate::memory::project(plan.project.as_deref());
        calendar.set_days_ago(plan.days_ago);
        if !task.threads.is_empty() {
            let place = plan.project.as_deref().unwrap_or("project");
            let _ = write!(
                conversation.transcript,
                "\n[thread {}: {place}, days_ago {}]\n",
                index.saturating_add(1),
                plan.days_ago
            );
        }
        // As the server does when a session starts: each ledger the thread reads, when due.
        if let Some(tidier) = tidier.as_ref().filter(|_| task.tidy == Tidying::WhenDue) {
            let ledgers = remembering.ledgers(scope);
            conversation.tidied(remembering.tidy(tidier, &ledgers, true).await);
        }
        in_view.extend(remembering.in_view(scope).await?);
        let goal = task.goal.as_ref().filter(|_| index == 0);
        let steer = task.steer.as_ref().filter(|_| index == 0);
        let Some(thread) = conversation.thread(scope, &plan.turns, goal, steer).await? else {
            break;
        };
        threads.push(events_of(&harness, scope, thread).await?);
        if !conversation.failures.is_empty() {
            break;
        }
    }
    if let Some(tidier) = tidier
        .as_ref()
        .filter(|_| task.tidy == Tidying::AfterThreads)
        && conversation.failures.is_empty()
    {
        let ledgers = remembering.every_ledger();
        conversation.tidied(remembering.tidy(tidier, &ledgers, false).await);
    }
    let mut memory = remembering.report().await?;
    if let Some(usage) = &mut memory.usage {
        usage.in_view = in_view;
    }
    Ok(Conversed {
        threads,
        transcript: conversation.transcript,
        failures: conversation.failures,
        memory,
    })
}

/// `thread`'s events in `scope`, as JSON Lines.
async fn events_of(
    harness: &Wired,
    scope: Scope,
    thread: glave_event::ThreadId,
) -> Result<String, EvalError> {
    let fail = |err: &dyn std::fmt::Display| EvalError::Setup(err.to_string());
    let events = (harness.log())
        .read(&scope, thread, None, usize::MAX)
        .await
        .map_err(|err| fail(&err))?;
    let mut lines = String::new();
    for event in &events {
        lines.push_str(&event.to_json().map_err(|err| fail(&err))?);
        lines.push('\n');
    }
    Ok(lines)
}

/// One attempt's conversation so far.
struct Talk<'a> {
    harness: &'a Wired,
    task: &'a Task,
    deadline: Instant,
    transcript: String,
    failures: Vec<String>,
}

impl Talk<'_> {
    /// Notes what tidy-up did. One that fails changes nothing; the checks then grade what is
    /// left.
    fn tidied(&mut self, runs: Result<Vec<String>, EvalError>) {
        let _ = match runs {
            Ok(runs) => runs
                .iter()
                .try_for_each(|run| writeln!(self.transcript, "[tidy] {run}")),
            Err(err) => writeln!(self.transcript, "[tidy failed] {err}"),
        };
    }

    /// Runs `turns` in a new thread in `scope`, the first as `goal` when there is one, with
    /// `steer` sent while it runs, and gives the thread's id; `None` when it could not start.
    async fn thread(
        &mut self,
        scope: Scope,
        turns: &[String],
        goal: Option<&crate::task::TaskGoal>,
        steer: Option<&Steer>,
    ) -> Result<Option<glave_event::ThreadId>, EvalError> {
        let fail = |err: glave_harness::HarnessError| EvalError::Setup(err.to_string());
        let harness = self.harness;
        let mut thread = harness.start_thread(scope).await.map_err(fail)?;
        for (index, prompt) in turns.iter().enumerate() {
            if index > 0 {
                thread = harness
                    .resume_thread(scope, thread.id)
                    .await
                    .map_err(fail)?;
            }
            let _ = write!(self.transcript, "\n> {prompt}\n");
            // Past the deadline the turn is cancelled, so it still ends cleanly in the log.
            let cancel = Cancel::new();
            let _deadline = Deadline::cancels(&cancel, self.deadline);
            let transcript = &mut self.transcript;
            let mut steering = steer
                .filter(|_| index == 0)
                .map(|steer| Steering::new(steer, &thread));
            let mut show = |live: Live| {
                if let Some(sent) = steering
                    .as_mut()
                    .and_then(|steering| steering.see(&live, harness.clock(), harness.entropy()))
                {
                    let _ = write!(transcript, "\n> (while it works) {sent}\n");
                }
                note(transcript, live);
            };
            let ran = match (goal, index) {
                (Some(goal), 0) => {
                    let goal = Goal {
                        objective: prompt.clone(),
                        token_budget: goal.budget.unwrap_or(DEFAULT_BUDGET),
                    };
                    harness
                        .start_goal(&mut thread, goal, &Caller::default(), &cancel, &mut show)
                        .await
                }
                _ => {
                    harness
                        .run_turn(&mut thread, prompt, &cancel, &mut show)
                        .await
                }
            };
            // What was sent after the model's last call starts a turn of its own.
            let ran = harness
                .run_queued(&mut thread, ran, &cancel, &mut show)
                .await;
            self.failures
                .extend(steering.as_ref().and_then(Steering::unsent));
            match ran {
                Ok(_) if cancel.is_cancelled() => {
                    let seconds = self.task.timeout.as_secs();
                    self.failures.push(format!("took longer than {seconds}s"));
                    break;
                }
                Ok(_) => {}
                Err(err) => {
                    self.failures
                        .push(format!("turn {} failed: {err}", index + 1));
                    break;
                }
            }
        }
        Ok(Some(thread.id))
    }
}

/// Cancels a [`Cancel`] at a deadline, for as long as this lives.
struct Deadline(tokio::task::JoinHandle<()>);

impl Deadline {
    fn cancels(cancel: &Cancel, at: Instant) -> Self {
        let cancel = cancel.clone();
        Self(tokio::spawn(async move {
            tokio::time::sleep_until(at.into()).await;
            cancel.cancel();
        }))
    }
}

impl Drop for Deadline {
    fn drop(&mut self) {
        self.0.abort();
    }
}

/// Appends what happened to the readable transcript.
fn note(transcript: &mut String, live: Live) {
    let _ = match live {
        Live::Text(text) => write!(transcript, "{text}"),
        Live::ToolStarted { summary } => write!(transcript, "\n[tool] {summary}\n"),
        Live::ToolFinished {
            summary,
            is_error: true,
        } => writeln!(transcript, "[tool failed] {summary}"),
        Live::ToolDeclined {
            summary, caution, ..
        } => match caution {
            Some(caution) => writeln!(transcript, "[declined] {summary}\n[why it asked] {caution}"),
            None => writeln!(transcript, "[declined] {summary}"),
        },
        Live::Compacted {
            before_tokens,
            after_tokens,
        } => writeln!(
            transcript,
            "[compacted {before_tokens} → {after_tokens} tokens]"
        ),
        Live::CompactionFailed(reason) => writeln!(transcript, "[compaction failed] {reason}"),
        Live::Retrying {
            retry, of, error, ..
        } => writeln!(transcript, "[retry {retry} of {of}] {error}"),
        Live::SubagentStarted { description, .. } => writeln!(
            transcript,
            "
[agent] {description}"
        ),
        Live::Child { update, .. } => {
            transcript.push_str("  ");
            note(transcript, *update);
            Ok(())
        }
        Live::SubagentEnded { ok, .. } => {
            writeln!(transcript, "[agent {}]", if ok { "done" } else { "failed" })
        }
        Live::Memory { verb, text, .. } => writeln!(
            transcript,
            "[memory] {} {}",
            verb.as_str(),
            text.unwrap_or_default()
        ),
        Live::Asked { questions, .. } => {
            let asked: Vec<&str> = questions.iter().map(|q| q.question.as_str()).collect();
            writeln!(transcript, "[asked] {}", asked.join(" / "))
        }
        Live::Unread { reason, .. } => {
            writeln!(transcript, "[not read] {}", reason.unwrap_or_default())
        }
        Live::Reasoning(_)
        | Live::ToolFinished { .. }
        | Live::ApprovalPending { .. }
        | Live::Waiting => Ok(()),
    };
}

async fn write(path: PathBuf, text: &str) -> Result<(), EvalError> {
    tokio::fs::write(&path, text)
        .await
        .map_err(EvalError::io("write", path))
}
