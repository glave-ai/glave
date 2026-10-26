//! The thread's plan (`specs/session.md`, "Plans"): the steps of its work, each to do, doing or
//! done, kept by the model with `update_plan`, as Codex's tool of that name and Claude Code's
//! todo list are. Every change is a `plan.updated` (`events.md`), the latest the plan, so a
//! resumed thread, another server and every client read the same one.

use glave_event::{
    Arguments, Body, CallId, Event, Payload as _, PlanStatus, PlanStep, PlanUpdated, ThreadId,
    decode,
};
use glave_ports::{BlobStore, EventLog, ModelProvider, Scope, ToolCall, ToolOutput, ToolSpec};
use serde::Deserialize;
use serde_json::json;

use crate::approval::Approver;
use crate::calls::NOT_RUN;
use crate::cancel::Cancel;
use crate::harness::{Harness, HarnessError, Thread};
use crate::turn::Live;

/// The tool the model keeps the plan with, as Codex names it.
pub(crate) const UPDATE_PLAN: &str = "update_plan";

/// The most steps a plan holds.
const MAX_STEPS: usize = 64;

/// The longest step, in characters.
const MAX_STEP: usize = 1_000;

/// A thread's plan: its latest `plan.updated`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Plan {
    /// The steps, in order.
    pub steps: Vec<PlanStep>,
}

/// A thread's plan as the harness holds it: the latest recorded, and what the model's call set
/// that is still to be recorded with the call's result.
#[derive(Debug, Clone, Default)]
pub(crate) struct Planning {
    /// The thread's plan, if it has one.
    pub(crate) plan: Option<Plan>,
    /// What the model's call set, recorded with the call's result.
    pub(crate) staged: Option<PlanUpdated>,
}

impl Planning {
    /// The plan as `events` leave it.
    pub(crate) fn of(events: &[Event]) -> Self {
        let mut planning = Self::default();
        for event in events {
            if let Body::PlanUpdated(updated) = &event.body {
                planning.recorded(updated);
            }
        }
        planning
    }

    /// The plan once `updated` is recorded.
    pub(crate) fn recorded(&mut self, updated: &PlanUpdated) {
        self.plan = Some(Plan {
            steps: updated.steps.clone(),
        });
    }
}

/// The plan `update_plan` was called with.
#[derive(Deserialize)]
struct Called {
    plan: Vec<CalledStep>,
}

#[derive(Deserialize)]
struct CalledStep {
    step: String,
    status: PlanStatus,
}

pub(crate) fn spec() -> ToolSpec {
    ToolSpec {
        name: UPDATE_PLAN.to_owned(),
        description: "Keep the plan for work with several steps, so you and the person can see \
                      it and how far along it is. Send the whole plan each time: every step, \
                      in order, with its status. Keep one step in_progress while you work on \
                      it, and mark each completed as soon as it is done. Skip it for simple \
                      tasks."
            .to_owned(),
        parameters: json!({"type": "object", "properties": {
            "plan": {"type": "array", "items": {"type": "object", "properties": {
                "step": {"type": "string", "description": "What to do, in a short line"},
                "status": {"type": "string", "enum": ["pending", "in_progress", "completed"]}
            }, "required": ["step", "status"]}}
        }, "required": ["plan"]}),
    }
}

/// The steps an `update_plan` call sets, or why they are refused, for the model.
fn called(arguments: &Arguments) -> Result<Vec<PlanStep>, String> {
    let Arguments::Json(arguments) = arguments else {
        return Err("The arguments were not valid JSON.".to_owned());
    };
    let called: Called = serde_json::from_value(serde_json::Value::Object(arguments.clone()))
        .map_err(|err| format!("The plan was not understood: {err}."))?;
    let steps = called.plan.into_iter().map(|step| (step.step, step.status));
    checked(steps)
}

/// `steps` as a plan holds them, each trimmed, or why they are refused: a plan has 1 to 64
/// steps, none blank and none longer than 1,000 characters.
fn checked(steps: impl IntoIterator<Item = (String, PlanStatus)>) -> Result<Vec<PlanStep>, String> {
    let mut checked = Vec::new();
    for (step, status) in steps {
        let step = step.trim();
        if step.is_empty() {
            return Err("A step is blank: say what to do in each.".to_owned());
        }
        if step.chars().count() > MAX_STEP {
            return Err(format!(
                "A step is longer than {MAX_STEP} characters: keep each to a short line."
            ));
        }
        checked.push(PlanStep {
            step: step.to_owned(),
            status,
        });
    }
    if checked.is_empty() || checked.len() > MAX_STEPS {
        return Err(format!("A plan has 1 to {MAX_STEPS} steps."));
    }
    Ok(checked)
}

/// How far along `steps` are: `2/5 done, now: run the tests`.
fn progress(steps: &[PlanStep]) -> String {
    let done = steps
        .iter()
        .filter(|step| step.status == PlanStatus::Completed)
        .count();
    let total = steps.len();
    match steps
        .iter()
        .find(|step| step.status == PlanStatus::InProgress)
    {
        Some(step) => format!("{done}/{total} done, now: {}", step.step),
        None => format!("{done}/{total} done"),
    }
}

/// What an `update_plan` call shows while it runs: `update_plan 2/5 done, now: run the tests`.
fn describe(arguments: &Arguments) -> String {
    match called(arguments) {
        Ok(steps) => format!("{UPDATE_PLAN} {}", progress(&steps)),
        Err(_) => glave_ports::describe_call(UPDATE_PLAN, arguments),
    }
}

/// Runs the model's `update_plan` call: the thread's plan from now on, recorded with the
/// call's result.
pub(crate) fn update_plan(
    thread: &mut Thread,
    call: &ToolCall,
    cancel: &Cancel,
    live: &mut (dyn FnMut(Live) + Send),
) -> ToolOutput {
    if cancel.is_cancelled() {
        return ToolOutput::text(true, NOT_RUN);
    }
    let summary = describe(&call.arguments);
    live(Live::ToolStarted {
        summary: summary.clone(),
    });
    let output = update(thread, &call.call_id, &call.arguments);
    live(Live::ToolFinished {
        summary,
        is_error: output.is_error,
    });
    output
}

/// Runs an `update_plan` call, call `call_id`: the thread's plan from now on, staged to be
/// recorded with the call's result, which makes it the thread's. The model's result.
fn update(thread: &mut Thread, call_id: &CallId, arguments: &Arguments) -> ToolOutput {
    let steps = match called(arguments) {
        Ok(steps) => steps,
        Err(err) => return ToolOutput::text(true, err),
    };
    let text = format!("Plan updated: {}.", progress(&steps));
    thread.plan.staged = Some(PlanUpdated {
        steps,
        call_id: Some(call_id.clone()),
    });
    ToolOutput::text(false, text)
}

impl<P, L, B, A> Harness<P, L, B, A>
where
    P: ModelProvider,
    L: EventLog,
    B: BlobStore,
    A: Approver,
{
    /// Thread `id`'s plan, read from the log: its latest `plan.updated`.
    pub async fn plan(&self, scope: Scope, id: ThreadId) -> Result<Option<Plan>, HarnessError> {
        if self.log.read(&scope, id, None, 1).await?.is_empty() {
            return Err(HarnessError::NotFound(id));
        }
        let latest = self.log.latest_of(&scope, id, &[PlanUpdated::TYPE]).await?;
        let Some(raw) = latest else {
            return Ok(None);
        };
        let event = decode(raw).map_err(|e| HarnessError::Event(e.to_string()))?;
        Ok(Planning::of(&[event]).plan)
    }
}

#[cfg(test)]
mod tests;
