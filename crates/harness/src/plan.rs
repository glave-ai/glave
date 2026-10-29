//! The thread's plan (`specs/approvals.md`, "Plan mode"): the steps of its work, each to do,
//! doing or done, kept by the model with `update_plan`, as Codex's tool of that name and
//! Claude Code's todo list are, and by the person editing a proposal. Every change is a
//! `plan.updated` (`events.md`), the latest the plan, so a resumed thread, another server and
//! every client read the same one. In plan mode the model proposes it, and its turn ends
//! there; "Build it" approves it and builds from it (`build.rs`).

mod build;

use std::fmt::Write as _;

use glave_event::{
    Arguments, Body, CallId, Caution, Event, Part, Payload as _, PlanApproved, PlanStatus,
    PlanStep, PlanUpdated, ThreadId, decode,
};
use glave_ports::{BlobStore, EventLog, ModelProvider, Scope, ToolCall, ToolOutput, ToolSpec};
use serde::Deserialize;
use serde_json::json;

use crate::approval::{ApprovalRequest, Approver};
use crate::calls::NOT_RUN;
use crate::cancel::Cancel;
use crate::harness::{Harness, HarnessError, Thread};
use crate::turn::{Live, Step};

/// The tool the model keeps the plan with, as Codex names it.
pub(crate) const UPDATE_PLAN: &str = "update_plan";

/// The most steps a plan holds.
pub const MAX_STEPS: usize = 64;

/// The longest step, in characters.
pub const MAX_STEP: usize = 1_000;

/// Closes the system prompt of a turn in plan mode: what the mode is for, and how the turn
/// ends.
pub(crate) const PLAN_MODE: &str = "\n\nPlan mode: the person wants a plan before anything \
     changes. Look around with tools and commands that only read: nothing of the project may \
     change, so a command that would write is blocked and any tool that would change something \
     is refused. Ask with question when the request is unclear. Once you understand the work, \
     propose the plan with update_plan: every step to do, in order, each pending and a short \
     line the person can read and edit. Your turn ends there, and the person builds it or asks \
     you to change it. Do not write the steps out in your reply too.";

/// A thread's plan: its latest `plan.updated`, and whether it waits for the person.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Plan {
    /// The steps, in order.
    pub steps: Vec<PlanStep>,
    /// Proposed in plan mode and not approved yet: the person builds it, edits it, or asks
    /// for changes.
    pub proposed: bool,
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
            match &event.body {
                Body::PlanUpdated(updated) => planning.recorded(updated),
                Body::PlanApproved(_) => planning.approved(),
                _ => {}
            }
        }
        planning
    }

    /// The plan once `updated` is recorded.
    pub(crate) fn recorded(&mut self, updated: &PlanUpdated) {
        self.plan = Some(Plan {
            steps: updated.steps.clone(),
            proposed: updated.proposed,
        });
    }

    /// The plan once it is approved: no longer a proposal.
    pub(crate) fn approved(&mut self) {
        if let Some(plan) = self.plan.as_mut() {
            plan.proposed = false;
        }
    }

    /// The plan proposed and waiting for the person, if there is one.
    pub(crate) fn proposed(&self) -> Option<&Plan> {
        self.plan.as_ref().filter(|plan| plan.proposed)
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
                      tasks. When the person approved a plan, build it and keep it current."
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
pub(crate) fn checked(
    steps: impl IntoIterator<Item = (String, PlanStatus)>,
) -> Result<Vec<PlanStep>, String> {
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
pub(crate) fn progress(steps: &[PlanStep]) -> String {
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
pub(crate) fn describe(arguments: &Arguments) -> String {
    match called(arguments) {
        Ok(steps) => format!("{UPDATE_PLAN} {}", progress(&steps)),
        Err(_) => glave_ports::describe_call(UPDATE_PLAN, arguments),
    }
}

/// Runs an `update_plan` call, call `call_id`: the thread's plan from now on, staged to be
/// recorded with the call's result, which makes it the thread's, and proposed when the thread is
/// in plan mode (`proposing`). The model's result, and whether it proposed a plan.
pub(crate) fn update(
    thread: &mut Thread,
    call_id: &CallId,
    arguments: &Arguments,
    proposing: bool,
) -> (ToolOutput, bool) {
    let steps = match called(arguments) {
        Ok(steps) => steps,
        Err(err) => return (ToolOutput::text(true, err), false),
    };
    let text = if proposing {
        format!(
            "Plan proposed, {} steps: the person builds it or asks you to change it.",
            steps.len()
        )
    } else {
        format!("Plan updated: {}.", progress(&steps))
    };
    thread.plan.staged = Some(PlanUpdated {
        steps,
        call_id: Some(call_id.clone()),
        proposed: proposing,
        by: None,
    });
    (ToolOutput::text(false, text), proposing)
}

/// `steps` as the model reads them: one numbered line each, its status in brackets.
pub(crate) fn listed(steps: &[PlanStep]) -> String {
    let mut text = String::new();
    for (number, step) in (1_usize..).zip(steps) {
        let status = match step.status {
            PlanStatus::Pending => "pending",
            PlanStatus::InProgress => "in_progress",
            PlanStatus::Completed => "completed",
        };
        let _ = writeln!(text, "{number}. [{status}] {}", step.step);
    }
    text
}

/// What the model is told of `updated` where it was recorded, in the conversation: the
/// person's edit, as a note of glave's; nothing of its own call's update, whose call it sees.
pub(crate) fn told(updated: &PlanUpdated) -> Option<Vec<Part>> {
    updated.call_id.is_none().then(|| {
        let plan = listed(&updated.steps);
        let text = format!(
            "[The person changed the plan. It is now:\n{plan}Work from this plan, not the one before.]"
        );
        vec![Part::Text { text }]
    })
}

/// What the model is told instead of a call plan mode refused: why it would have asked in
/// Ask first, and what to do instead.
pub(crate) fn refused(request: &ApprovalRequest) -> String {
    let because = request.caution.as_ref().and_then(Caution::because);
    let why = match (because, request.effect) {
        (Some(caution), _) => format!(", and {caution}"),
        (None, glave_ports::Effect::Changes) => ", and this changes files".to_owned(),
        (None, _) => ", and this may change things".to_owned(),
    };
    format!(
        "Not run: plan mode changes nothing until the person approves the plan{why}. Look \
         around with tools and commands that only read; what needs changing goes in the plan."
    )
}

impl<P, L, B, A> Harness<P, L, B, A>
where
    P: ModelProvider,
    L: EventLog,
    B: BlobStore,
    A: Approver,
{
    /// Thread `id`'s plan, read from the log: its latest `plan.updated`, approved if a
    /// `plan.approved` came after it.
    pub async fn plan(&self, scope: Scope, id: ThreadId) -> Result<Option<Plan>, HarnessError> {
        if self.log.read(&scope, id, None, 1).await?.is_empty() {
            return Err(HarnessError::NotFound(id));
        }
        let kinds = [PlanUpdated::TYPE, PlanApproved::TYPE];
        let Some(latest) = self.log.latest_of(&scope, id, &kinds).await? else {
            return Ok(None);
        };
        let approved = latest.kind.as_str() == PlanApproved::TYPE;
        let updated = if approved {
            self.log.latest_of(&scope, id, &[PlanUpdated::TYPE]).await?
        } else {
            Some(latest)
        };
        let Some(raw) = updated else {
            return Ok(None);
        };
        let event = decode(raw).map_err(|e| HarnessError::Event(e.to_string()))?;
        let mut planning = Planning::default();
        if let Body::PlanUpdated(updated) = &event.body {
            planning.recorded(updated);
        }
        if approved {
            planning.approved();
        }
        Ok(planning.plan)
    }

    /// Runs the model's `update_plan` call: the thread's plan from now on, recorded with the
    /// call's result, and in plan mode a proposal the turn ends with.
    pub(crate) fn update_plan(
        &self,
        thread: &mut Thread,
        call: &ToolCall,
        step: &mut Step,
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
        let proposing = self.planning(thread);
        let (output, proposed) = update(thread, &call.call_id, &call.arguments, proposing);
        step.proposed |= proposed;
        live(Live::ToolFinished {
            summary,
            is_error: output.is_error,
        });
        output
    }
}

#[cfg(test)]
pub(crate) mod tests;
