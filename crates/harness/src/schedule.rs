//! `schedule`: work that repeats, set up from a chat when the person asks for it ("do this every
//! weekday at 8"), listed, or removed. Setting one up always asks the person first, in every
//! mode, Full access included, with a card the server writes ("Repeat this every weekday at
//! 8:00 am?", when it first runs on their clock, and the chat its runs go to): answered "Repeat
//! it", the schedule and its own chat are made, and each run is a new turn there; "Not now"
//! makes nothing. Removing one asks as any change does, so the judge may clear it in Auto. It
//! is offered only where schedules run and someone can answer, and never to a subagent; a
//! schedule's own run, which is offered it so its prefix stays the chat's, only lists, never
//! setting up or removing what repeats. The schedules themselves are the session's, which keeps
//! them in its ledgers ([`Schedules`]); the harness only asks and passes on.

use std::sync::Arc;

use glave_event::{Arguments, Ask, CardLines, Caution, PrincipalId, Repeating, ThreadId};
use glave_ports::{
    BlobStore, BoxFuture, Effect, EventLog, ModelProvider, Scope, ToolCall, ToolOutput, ToolSpec,
};
use serde::Deserialize;
use serde_json::{Value, json};

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

/// What the model is told in a schedule's own run, asked to set one up or remove one.
const IN_A_RUN: &str = "Not done: this is a run of repeating work, which never sets up or \
     removes what repeats. Tell the person what you would change, and they can ask for it in a \
     chat.";

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
    /// The time zone its card showed the person, once it did: what it is made in, so a zone
    /// they set meanwhile never makes one the card didn't show.
    pub zone: Option<String>,
}

/// What is asked of the schedules.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Asked {
    /// Set it up, the person having said yes.
    Create(Repeat),
    /// The person's schedules.
    List,
    /// Remove the one with this id.
    Remove(String),
}

/// What the person is asked before something is done, in the server's words: the question,
/// what it would do in a line or two, and why it needs them, when something does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Confirm {
    pub ask: Ask,
    /// What it would do, recorded as the card's detail.
    pub detail: String,
    pub caution: Option<Caution>,
    /// For a schedule's card, what it was written from.
    pub repeating: Option<Repeating>,
    /// For a schedule's card, its lines, typed, as `detail` holds them.
    pub lines: Option<CardLines>,
}

/// What came of what was asked, for the model, and the schedule it was about, for the person's
/// client (`schedule` and its `chat`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Done {
    pub said: String,
    pub about: Option<Value>,
}

/// Where a person's schedules are kept: the session's ledgers, which the harness can't reach
/// itself. Every answer for the model is in English, as a tool's result is.
pub trait Schedules: Send + Sync {
    /// What setting up `repeat` would ask the person, or why it can't be set up: everything
    /// that setting it up would refuse is refused here, before anyone is asked.
    fn check<'a>(
        &'a self,
        from: &'a Asker,
        repeat: &'a Repeat,
    ) -> BoxFuture<'a, Result<Confirm, String>>;

    /// What removing the schedule `schedule` would ask the person, or why it can't be.
    fn check_removal<'a>(
        &'a self,
        from: &'a Asker,
        schedule: &'a str,
    ) -> BoxFuture<'a, Result<Confirm, String>>;

    /// Does what was asked.
    fn act<'a>(&'a self, from: &'a Asker, asked: Asked) -> BoxFuture<'a, Result<Done, String>>;
}

/// The model's arguments.
#[derive(Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
enum Called {
    Create {
        name: String,
        prompt: String,
        cron: String,
    },
    List,
    Remove {
        schedule: String,
    },
}

pub(crate) fn spec() -> ToolSpec {
    ToolSpec {
        name: SCHEDULE.to_owned(),
        description: "Set up work that repeats on its own, list what repeats in this project, \
                      or remove one of those, only when the person asks for it (\"do this every weekday at 8\", \"stop the \
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
            "schedule": {"type": "string", "description": "remove: its id, as list shows it"}
        }, "required": ["action"]}),
    }
}

/// What a call asks, or why it could not be read.
fn asked(arguments: &Arguments) -> Result<Asked, String> {
    let Arguments::Json(arguments) = arguments else {
        return Err("The arguments were not valid JSON.".to_owned());
    };
    let called: Called = serde_json::from_value(Value::Object(arguments.clone()))
        .map_err(|err| format!("The arguments were not understood: {err}."))?;
    Ok(match called {
        Called::Create { name, prompt, cron } => Asked::Create(Repeat {
            name,
            prompt,
            cron,
            zone: None,
        }),
        Called::List => Asked::List,
        Called::Remove { schedule } => Asked::Remove(schedule),
    })
}

/// A call in a line, for the person: `schedule: Morning email summary`.
pub(crate) fn describe(arguments: &Arguments) -> String {
    match asked(arguments) {
        Ok(Asked::Create(repeat)) => format!("{SCHEDULE}: {}", repeat.name),
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
    /// Keeps threads' schedules in `schedules`, which offers `schedule` in their chats: none
    /// where no server fires schedules.
    #[must_use]
    pub fn with_schedules(mut self, schedules: Option<Arc<dyn Schedules>>) -> Self {
        self.schedules = schedules;
        self
    }

    /// Whether `thread` is offered `schedule`: a chat someone can answer in, where schedules
    /// run.
    pub(crate) fn offers_schedule(&self, thread: &Thread) -> bool {
        self.schedules.is_some() && !thread.delegated() && self.approver.can_answer(thread.id)
    }

    /// Runs a `schedule` call, its hooks first whatever it does: setting one up asks the
    /// person, whatever the mode, and waits for them as any approval does; removing one asks
    /// as any change does; listing asks nothing. A schedule's own run only lists.
    pub(crate) async fn call_schedule(
        &self,
        thread: &mut Thread,
        call: &ToolCall,
        step: &mut Step,
        cancel: &Cancel,
        live: &mut (dyn FnMut(Live) + Send),
    ) -> Result<ToolOutput, HarnessError> {
        let Some(schedules) = self.schedules.clone() else {
            return Ok(ToolOutput::text(
                true,
                "Repeating work isn't available here.",
            ));
        };
        let mut asked = match asked(&call.arguments) {
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
        if let Some(answered) = step.waited(&call.call_id) {
            // Its hooks ran and it asked before the turn stopped: the person's answer decides.
            shown(&mut asked, answered.asked.repeating.as_ref());
            if let Some(refused) = self.answered(thread, answered, cancel, live).await? {
                return Ok(refused);
            }
        } else {
            let hook = self.stopped_by_hook(thread, &hooked, &summary, cancel, live);
            if let Some(stopped) = hook.await? {
                return Ok(stopped);
            }
            if step.scheduled && asked != Asked::List {
                return Ok(ToolOutput::text(true, IN_A_RUN));
            }
            let refused = match &mut asked {
                Asked::Create(repeat) => {
                    let checked = schedules.check(&from, repeat).await;
                    let card = checked.as_ref().ok().and_then(|it| it.repeating.as_ref());
                    repeat.zone = card.map(|card| card.zone.clone());
                    self.confirmed(thread, call, checked, cancel, live).await?
                }
                Asked::Remove(schedule) => {
                    let request = match schedules.check_removal(&from, schedule).await {
                        Ok(confirm) => request(thread, call, confirm),
                        Err(why) => return Ok(ToolOutput::text(true, why)),
                    };
                    let turn = &mut step.summary;
                    self.approve(thread, &request, turn, cancel, live).await?
                }
                Asked::List => None,
            };
            if let Some(refused) = refused {
                return Ok(refused);
            }
        }
        self.record_started(thread, &call.call_id).await?;
        live(Live::ToolStarted {
            summary: summary.clone(),
        });
        let output = match schedules.act(&from, asked).await {
            Ok(done) => ToolOutput {
                structured: done.about,
                ..ToolOutput::text(false, done.said)
            },
            Err(why) => ToolOutput::text(true, why),
        };
        live(Live::ToolFinished {
            summary,
            is_error: output.is_error,
        });
        self.after_tool(thread, &hooked, output, cancel).await
    }

    /// Puts a schedule's card to the person, whatever the mode: `None` once they said yes;
    /// otherwise what the model is told instead, as when `checked` refused it, in plan mode, or
    /// with no one to ask.
    async fn confirmed(
        &self,
        thread: &mut Thread,
        call: &ToolCall,
        checked: Result<Confirm, String>,
        cancel: &Cancel,
        live: &mut (dyn FnMut(Live) + Send),
    ) -> Result<Option<ToolOutput>, HarnessError> {
        let confirm = match checked {
            Ok(confirm) => confirm,
            Err(why) => return Ok(Some(ToolOutput::text(true, why))),
        };
        if self.planning(thread) {
            return Ok(Some(ToolOutput::text(true, PLANNING)));
        }
        if !self.approver.can_answer(thread.id) {
            return Ok(Some(ToolOutput::text(true, NO_ONE)));
        }
        let request = request(thread, call, confirm);
        self.put_to_person(thread, &request, cancel, live).await
    }
}

/// Keeps in `asked`, when it sets up a schedule, the zone of the card the person was shown.
fn shown(asked: &mut Asked, card: Option<&Repeating>) {
    if let Asked::Create(repeat) = asked {
        repeat.zone = card.map(|card| card.zone.clone());
    }
}

/// What `call` asks the person, as `confirm` words it: a change, with the card's detail as
/// what would run.
fn request(thread: &Thread, call: &ToolCall, confirm: Confirm) -> ApprovalRequest {
    ApprovalRequest {
        thread: thread.id,
        call_id: call.call_id.clone(),
        tool: SCHEDULE.to_owned(),
        effect: Effect::Changes,
        summary: format!("{SCHEDULE}: {}", confirm.detail),
        ask: confirm.ask,
        secret: false,
        caution: confirm.caution,
        outside_sandbox: None,
        repeating: confirm.repeating,
        lines: confirm.lines,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn arguments(value: Value) -> Arguments {
        match value {
            Value::Object(map) => Arguments::Json(map),
            other => panic!("not an object: {other}"),
        }
    }

    #[test]
    fn a_call_reads_as_what_it_asks_and_says_what_it_could_not_read() {
        let create = arguments(json!({
            "action": "create", "name": "Morning email summary",
            "prompt": "Summarize my email", "cron": "0 8 * * 1-5"
        }));
        let repeat = Repeat {
            name: "Morning email summary".into(),
            prompt: "Summarize my email".into(),
            cron: "0 8 * * 1-5".into(),
            zone: None,
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
