//! `schedule`: work that repeats, set up from a chat when the person asks for it ("do this every
//! weekday at 8"), listed, or removed. Setting one up always asks the person first, in every
//! mode, Full access included, with a card whose sentence glave writes from when it repeats
//! ("Repeat this every weekday at 8:00 am?"): answered "Repeat it", the schedule and its own chat
//! are made, and each run is a new turn there; "Not now" makes nothing. It is offered only where
//! someone can answer, and never to a subagent. The schedules themselves are the session's,
//! which keeps them in its ledgers ([`Schedules`]); the harness only asks and passes on.

use std::sync::Arc;

use glave_event::{Arguments, Ask, PrincipalId, ThreadId};
use glave_ports::{
    BlobStore, BoxFuture, Effect, EventLog, ModelProvider, Scope, ToolCall, ToolOutput, ToolSpec,
};
use serde::Deserialize;
use serde_json::json;

use crate::approval::{ApprovalRequest, Approver};
use crate::cancel::Cancel;
use crate::harness::{Harness, HarnessError, Thread};
use crate::hooks::ToolHook;
use crate::turn::{Live, Step};

/// The tool's name.
pub(crate) const SCHEDULE: &str = "schedule";

/// What the model is told in plan mode.
const PLANNING: &str = "Not set up in plan mode: nothing repeats while you plan. Put it in the \
     plan instead.";

/// What the model is told where no one can confirm it.
const NO_ONE: &str = "Not set up: no one is here to confirm it. Tell the person what you would \
     repeat and when, and they can ask you again.";

/// What the model is told where the server keeps no schedules.
const NONE_HERE: &str = "Repeating work isn't available here.";

/// Who asks of the schedules, and from where: a chat of a person's.
#[derive(Debug, Clone)]
pub struct Asker {
    pub scope: Scope,
    /// The chat it is asked from, which a schedule it sets up remembers.
    pub thread: ThreadId,
    /// The person the chat is for.
    pub person: Option<PrincipalId>,
}

/// A schedule the model would set up.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Repeat {
    /// What it is called, which its chat is named.
    pub name: String,
    /// What each run does, as a request complete on its own.
    pub prompt: String,
    /// When it runs: a five-field cron expression on the person's own clock.
    pub cron: String,
    /// Tokens each run may use.
    pub budget: u64,
}

/// What is asked of the schedules.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Asked {
    /// Whether this one can be set up: answered with when it repeats, as a phrase that follows
    /// other words ("every weekday at 8:00 am"), or why not.
    Check(Repeat),
    /// Set it up, the person having said yes.
    Create(Repeat),
    /// The person's schedules.
    List,
    /// Remove the one with this id.
    Remove(String),
}

/// Where a person's schedules are kept: the session's ledgers, which the harness can't reach
/// itself. Every answer is for the model, in English, as a tool's result is.
pub trait Schedules: Send + Sync {
    fn act<'a>(&'a self, from: &'a Asker, asked: Asked) -> BoxFuture<'a, Result<String, String>>;
}

/// The model's arguments.
#[derive(Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
enum Called {
    Create {
        name: String,
        prompt: String,
        cron: String,
        budget: u64,
    },
    List,
    Remove {
        schedule: String,
    },
}

pub(crate) fn spec() -> ToolSpec {
    ToolSpec {
        name: SCHEDULE.to_owned(),
        description: "Set up work that repeats on its own, list what repeats, or remove one, \
                      only when the person asks for it (\"do this every weekday at 8\", \"stop the \
                      morning summary\"). Setting one up shows the person a card to confirm; once \
                      they do, it gets a chat of its own, and each run is a new turn there that \
                      starts with your prompt. Write the prompt as the person's request, complete \
                      on its own, since a run reads nothing of this chat. Write cron for the \
                      person's own clock, in their time zone: 0 8 * * 1-5 is every weekday at \
                      8:00 am. Never set one up unasked."
            .to_owned(),
        parameters: json!({"type": "object", "properties": {
            "action": {"type": "string", "enum": ["create", "list", "remove"]},
            "name": {"type": "string", "description": "create: a short name for it and its chat, such as Morning email summary"},
            "prompt": {"type": "string", "description": "create: what each run does"},
            "cron": {"type": "string", "description": "create: when it runs, five fields (minute hour day month weekday) on the person's clock"},
            "budget": {"type": "integer", "description": "create: tokens each run may use, in and out; 200000 suits most"},
            "schedule": {"type": "string", "description": "remove: its id, as list shows it"}
        }, "required": ["action"]}),
    }
}

/// What a call asks, or why it could not be read.
fn asked(arguments: &Arguments) -> Result<Asked, String> {
    let Arguments::Json(arguments) = arguments else {
        return Err("The arguments were not valid JSON.".to_owned());
    };
    let called: Called = serde_json::from_value(serde_json::Value::Object(arguments.clone()))
        .map_err(|err| format!("The arguments were not understood: {err}."))?;
    Ok(match called {
        Called::Create {
            name,
            prompt,
            cron,
            budget,
        } => Asked::Create(Repeat {
            name,
            prompt,
            cron,
            budget,
        }),
        Called::List => Asked::List,
        Called::Remove { schedule } => Asked::Remove(schedule),
    })
}

/// A call in a line, for the person: `schedule: Morning email summary`.
pub(crate) fn describe(arguments: &Arguments) -> String {
    match asked(arguments) {
        Ok(Asked::Create(repeat) | Asked::Check(repeat)) => format!("{SCHEDULE}: {}", repeat.name),
        Ok(Asked::List) => format!("{SCHEDULE}: list"),
        Ok(Asked::Remove(schedule)) => format!("{SCHEDULE}: remove {schedule}"),
        Err(_) => SCHEDULE.to_owned(),
    }
}

impl<P, L, B, A> Harness<P, L, B, A>
where
    P: ModelProvider,
    L: EventLog,
    B: BlobStore,
    A: Approver,
{
    /// Keeps threads' schedules in `schedules`, which offers `schedule` in their chats.
    #[must_use]
    pub fn with_schedules(mut self, schedules: Option<Arc<dyn Schedules>>) -> Self {
        self.schedules = schedules;
        self
    }

    /// Whether `thread` is offered `schedule`: a chat someone can answer in, where schedules
    /// are kept.
    pub(crate) fn offers_schedule(&self, thread: &Thread) -> bool {
        self.schedules.is_some() && !thread.delegated() && self.approver.can_answer(thread.id)
    }

    /// Runs a `schedule` call: one that sets a schedule up asks the person first, whatever the
    /// mode, and waits for them as any approval does.
    pub(crate) async fn call_schedule(
        &self,
        thread: &mut Thread,
        call: &ToolCall,
        step: &mut Step,
        cancel: &Cancel,
        live: &mut (dyn FnMut(Live) + Send),
    ) -> Result<ToolOutput, HarnessError> {
        let Some(schedules) = self.schedules.clone() else {
            return Ok(ToolOutput::text(true, NONE_HERE));
        };
        let asked = match asked(&call.arguments) {
            Ok(asked) => asked,
            Err(why) => return Ok(ToolOutput::text(true, why)),
        };
        let from = Asker {
            scope: thread.scope,
            thread: thread.id,
            person: thread.for_person(),
        };
        let summary = describe(&call.arguments);
        let hooked = ToolHook::of(call, SCHEDULE);
        if let Asked::Create(repeat) = &asked {
            if let Some(answered) = step.waited(&call.call_id) {
                // It asked before the turn stopped: the person's answer decides.
                if let Some(refused) = self.answered(thread, answered, cancel, live).await? {
                    return Ok(refused);
                }
            } else {
                let hook = self.stopped_by_hook(thread, &hooked, &summary, cancel, live);
                if let Some(stopped) = hook.await? {
                    return Ok(stopped);
                }
                let when = match schedules.act(&from, Asked::Check(repeat.clone())).await {
                    Ok(when) => when,
                    Err(why) => return Ok(ToolOutput::text(true, why)),
                };
                if self.planning(thread) {
                    return Ok(ToolOutput::text(true, PLANNING));
                }
                if !self.approver.can_answer(thread.id) {
                    return Ok(ToolOutput::text(true, NO_ONE));
                }
                let request = ApprovalRequest {
                    thread: thread.id,
                    call_id: call.call_id.clone(),
                    tool: SCHEDULE.to_owned(),
                    effect: Effect::Runs,
                    // What each run does, shown apart from the sentence.
                    summary: format!("{SCHEDULE}: {}", repeat.prompt),
                    ask: Ask::Repeat { when },
                    secret: false,
                    caution: None,
                    outside_sandbox: None,
                };
                if let Some(refused) = self.put_to_person(thread, &request, cancel, live).await? {
                    return Ok(refused);
                }
            }
        }
        self.record_started(thread, &call.call_id).await?;
        live(Live::ToolStarted {
            summary: summary.clone(),
        });
        let output = match schedules.act(&from, asked).await {
            Ok(said) => ToolOutput::text(false, said),
            Err(why) => ToolOutput::text(true, why),
        };
        live(Live::ToolFinished {
            summary,
            is_error: output.is_error,
        });
        self.after_tool(thread, &hooked, output, cancel).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn arguments(value: serde_json::Value) -> Arguments {
        match value {
            serde_json::Value::Object(map) => Arguments::Json(map),
            other => panic!("not an object: {other}"),
        }
    }

    #[test]
    fn a_call_reads_as_what_it_asks_and_says_what_it_could_not_read() {
        let create = arguments(json!({
            "action": "create", "name": "Morning email summary",
            "prompt": "Summarize my email", "cron": "0 8 * * 1-5", "budget": 200_000
        }));
        let repeat = Repeat {
            name: "Morning email summary".into(),
            prompt: "Summarize my email".into(),
            cron: "0 8 * * 1-5".into(),
            budget: 200_000,
        };
        assert_eq!(asked(&create), Ok(Asked::Create(repeat)));
        assert_eq!(describe(&create), "schedule: Morning email summary");
        assert_eq!(
            asked(&arguments(json!({"action": "list"}))),
            Ok(Asked::List)
        );
        let remove = arguments(json!({"action": "remove", "schedule": "0192"}));
        assert_eq!(asked(&remove), Ok(Asked::Remove("0192".into())));
        let half = arguments(json!({"action": "create", "name": "x"}));
        assert!(asked(&half).unwrap_err().contains("missing field"));
        assert!(asked(&arguments(json!({"action": "edit"}))).is_err());
    }
}
