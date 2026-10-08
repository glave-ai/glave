//! One attempt: a fresh copy of the task's files, the harness wired in-process to a real model,
//! every turn sent, then the grading. Everything it leaves behind stays in the attempt's folder.

use std::ffi::OsString;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use glave_harness::{
    Cancel, Harness, Live, NoOneToAsk, OsEntropy, Parts, Sandbox, Settings, SystemClock, Todo,
    Workspace, agents_md,
};
use glave_ports::{EventLog, MemoryBlobs, MemoryLog, OrgId, Scope, SpaceId};
use glave_provider_openai::{Config, OpenAiCompatible};
use serde::{Deserialize, Serialize};

use crate::check::{Context, ended};
use crate::error::EvalError;
use crate::prepare::{Environment, Prepared, last_lines, prepare, script, tidy};
use crate::task::Task;
use crate::trace::{self, Trace};

/// What every attempt runs with.
pub struct Setup {
    /// The model endpoint. Its key is never printed; it is only searched for in the events.
    pub endpoint: Config,
    /// The environment commands start from, before the task's own and minus secrets.
    pub environment: Vec<(OsString, OsString)>,
    /// Whether the model is offered the todo list, to measure what it does.
    pub todo: bool,
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
    let start = Instant::now();
    let (events, transcript, mut failures) =
        converse(setup, task, model, &environment, &workspace).await?;
    let seconds = start.elapsed().as_secs_f64();
    write(dir.join("events.jsonl"), &events).await?;
    write(dir.join("transcript.txt"), &transcript).await?;
    if let Some(key) = &setup.endpoint.api_key
        && key.len() >= 8
        && events.contains(key.as_str())
    {
        failures.push("the endpoint's API key reached the model: rotate it now".to_owned());
    }
    let trace = trace::parse(&events).unwrap_or_else(|err| {
        failures.push(err);
        Trace::default()
    });
    failures.extend(ended(task.outcome, &trace.outcomes).err());
    if let Some(test) = task.test_script() {
        let test = test.display().to_string();
        let output = script("bash", &[test.as_str()], &workspace, &environment).await;
        tidy(dir).await;
        let output = output?;
        write(dir.join("test.log"), &output.text).await?;
        if !output.success {
            failures.push(format!(
                "tests/test.sh failed: {}",
                last_lines(&output.text)
            ));
        }
    }
    let cx = Context {
        trace: &trace,
        events: &events,
        workspace: &workspace,
        original: &task.environment(),
        model,
    };
    failures.extend(
        task.checks
            .iter()
            .filter_map(|check| check.judge(&cx).err()),
    );
    tidy(dir).await;
    let mut attempt = Attempt::graded(&task.name, model, number, &trace, seconds);
    attempt.failures = failures;
    Ok(attempt)
}

/// The harness for one attempt, wired in-process: a real provider, an in-memory log, the
/// workspace (sandboxed when the task asks) and no one to approve anything.
async fn wire(
    setup: &Setup,
    task: &Task,
    model: &str,
    environment: &Environment,
    root: &Path,
) -> Result<Harness<OpenAiCompatible, MemoryLog, MemoryBlobs, NoOneToAsk>, EvalError> {
    let mut workspace = Workspace::open(root, environment.clone())
        .await
        .map_err(EvalError::io("open", root))?;
    if let Some(level) = task.sandbox {
        let sandbox = Sandbox {
            level,
            network: task.network,
            home: environment.get(&OsString::from("HOME")).map(PathBuf::from),
            glave_home: None,
        };
        workspace = workspace
            .with_sandbox(&sandbox)
            .await
            .map_err(EvalError::io("sandbox", root))?;
    }
    let provider = OpenAiCompatible::new(setup.endpoint.clone())
        .map_err(|err| EvalError::Setup(err.to_string()))?;
    let mut settings = Settings::new(model, setup.endpoint.provider.clone());
    settings.instructions = agents_md(root, None)
        .await
        .map_err(EvalError::io("read the instructions in", root))?
        .map(|loaded| loaded.text);
    settings.approval = task.approval;
    if let Some(steps) = task.max_steps {
        settings.max_steps = steps;
    }
    if let Some(tokens) = task.context_window {
        settings.compaction.window_tokens = tokens;
    }
    let parts = Parts {
        provider,
        log: MemoryLog::new(),
        blobs: MemoryBlobs::new(),
        approver: NoOneToAsk,
        clock: Box::new(SystemClock),
        entropy: Box::new(OsEntropy),
        workspace,
    };
    let harness = Harness::new(parts, settings);
    Ok(if setup.todo {
        harness.with_tools(vec![Arc::new(Todo)])
    } else {
        harness
    })
}

/// Sends every turn, each after the first to a thread resumed from the log, and returns the
/// events as JSON Lines, a readable transcript, and anything that stopped the conversation.
async fn converse(
    setup: &Setup,
    task: &Task,
    model: &str,
    environment: &Environment,
    root: &Path,
) -> Result<(String, String, Vec<String>), EvalError> {
    let harness = wire(setup, task, model, environment, root).await?;
    let scope = Scope {
        org: OrgId::from_uuid(uuid::Uuid::from_u128(1)),
        space: SpaceId::from_uuid(uuid::Uuid::from_u128(2)),
    };
    let fail = |err: glave_harness::HarnessError| EvalError::Setup(err.to_string());
    let mut thread = harness.start_thread(scope).await.map_err(fail)?;
    let deadline = Instant::now() + task.timeout;
    let mut transcript = String::new();
    let mut failures = Vec::new();
    for (index, prompt) in task.turns.iter().enumerate() {
        if index > 0 {
            thread = harness
                .resume_thread(scope, thread.id)
                .await
                .map_err(fail)?;
        }
        let _ = write!(transcript, "\n> {prompt}\n");
        // Past the deadline the turn is cancelled, so it still ends cleanly in the log.
        let cancel = Cancel::new();
        let _deadline = Deadline::cancels(&cancel, deadline);
        let mut show = |live| note(&mut transcript, live);
        match harness
            .run_turn(&mut thread, prompt, &cancel, &mut show)
            .await
        {
            Ok(_) if cancel.is_cancelled() => {
                failures.push(format!("took longer than {}s", task.timeout.as_secs()));
                break;
            }
            Ok(_) => {}
            Err(err) => {
                failures.push(format!("turn {} failed: {err}", index + 1));
                break;
            }
        }
    }
    let events = harness
        .log()
        .read(&scope, thread.id, None, usize::MAX)
        .await
        .map_err(|err| EvalError::Setup(err.to_string()))?;
    let mut lines = String::new();
    for event in &events {
        lines.push_str(
            &event
                .to_json()
                .map_err(|err| EvalError::Setup(err.to_string()))?,
        );
        lines.push('\n');
    }
    Ok((lines, transcript, failures))
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
        Live::ToolDeclined { summary, .. } => writeln!(transcript, "[declined] {summary}"),
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
        Live::Asked { questions, .. } => {
            let asked: Vec<&str> = questions.iter().map(|q| q.question.as_str()).collect();
            writeln!(transcript, "[asked] {}", asked.join(" / "))
        }
        Live::Reasoning(_) | Live::ToolFinished { .. } | Live::Waiting => Ok(()),
    };
}

async fn write(path: PathBuf, text: &str) -> Result<(), EvalError> {
    tokio::fs::write(&path, text)
        .await
        .map_err(EvalError::io("write", path))
}
