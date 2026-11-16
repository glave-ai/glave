//! The schedules a chat's `schedule` tool asks of ([`Schedules`]): the session's, for the
//! person the chat is for, in the project it is in. Each answer is for the model, in English, as
//! a tool's result is.

use std::sync::Weak;

use glave_event::{ScheduleId, When};
use glave_harness::{Asked, Asker, Repeat, Schedules};
use glave_ports::{BlobStore, BoxFuture, Caller, EventLog, ModelProvider};

use super::NewSchedule;
use crate::cron::Cron;
use crate::protocol::{ApiError, ScheduleInfo, code};
use crate::repeats::repeats;
use crate::server::{Inner, Server};
use crate::words::{Language, Sentence, say};
use crate::zone::time_zone;

/// The server's schedules, as its chats' tool reaches them: held weakly, since the harness that
/// holds this is the server's own.
pub(crate) struct Asking<P, L, B>(pub(crate) Weak<Inner<P, L, B>>);

impl<P, L, B> Schedules for Asking<P, L, B>
where
    P: ModelProvider + 'static,
    L: EventLog + 'static,
    B: BlobStore + 'static,
{
    fn act<'a>(&'a self, from: &'a Asker, asked: Asked) -> BoxFuture<'a, Result<String, String>> {
        Box::pin(async move {
            let inner = (self.0.upgrade())
                .ok_or_else(|| "glave is stopping; try again once it runs".to_owned())?;
            let server = Server { inner };
            let done = server.asked(from, asked).await;
            done.map_err(|error| error.message.to_string())
        })
    }
}

impl<P, L, B> Server<P, L, B>
where
    P: ModelProvider + 'static,
    L: EventLog + 'static,
    B: BlobStore + 'static,
{
    /// Does what a chat's `schedule` tool asks, for the person the chat is for.
    async fn asked(&self, from: &Asker, asked: Asked) -> Result<String, ApiError> {
        let caller = Caller {
            person: from
                .person
                .filter(|person| Some(*person) != self.inner.person),
            org: Some(from.scope.org()),
            ..Caller::default()
        };
        match asked {
            Asked::Check(repeat) => self.check(&caller, &repeat).await,
            Asked::Create(repeat) => {
                let new = NewSchedule {
                    project: Some(from.scope.project()),
                    name: Some(repeat.name),
                    when: When::Cron(repeat.cron),
                    zone: None,
                    prompt: repeat.prompt,
                    budget: repeat.budget,
                    from: Some(from.thread),
                };
                let made = self.create_schedule(&caller, new).await?;
                Ok(format!(
                    "Set up \"{}\": {}. Each run is a new turn in its own chat of that name, \
                     starting with the prompt; the person sees and can pause or remove it there \
                     and on their Tasks page.{}",
                    made.name,
                    when_of(&made),
                    next_of(&made)
                ))
            }
            Asked::List => {
                let listed = self.list_schedules(&caller).await?;
                if listed.is_empty() {
                    return Ok("The person has no schedules.".to_owned());
                }
                let lines: Vec<String> = listed.iter().map(line).collect();
                Ok(lines.join("\n"))
            }
            Asked::Remove(text) => self.remove_asked(&caller, &text).await,
        }
    }

    /// Whether `repeat` can be set up for `caller`: when it repeats, as a phrase, or why not.
    async fn check(&self, caller: &Caller, repeat: &Repeat) -> Result<String, ApiError> {
        Cron::parse(&repeat.cron).map_err(|why| ApiError::new(code::INVALID_PARAMS, why))?;
        if repeat.budget == 0 {
            return Err(ApiError::new(
                code::INVALID_PARAMS,
                Sentence::ScheduleNoBudget,
            ));
        }
        if repeat.prompt.trim().is_empty() {
            return Err(ApiError::new(
                code::INVALID_PARAMS,
                Sentence::ScheduleNoPrompt,
            ));
        }
        time_zone(&self.zone_of(caller).await?)?;
        let when = When::Cron(repeat.cron.clone());
        Ok(say(Language::English, &repeats(&when, false)))
    }

    /// Removes the schedule of `caller`'s that `text` names by its id.
    async fn remove_asked(&self, caller: &Caller, text: &str) -> Result<String, ApiError> {
        let missing = || {
            ApiError::new(
                code::NO_SCHEDULE,
                Sentence::NoSchedule {
                    schedule: text.escape_debug().to_string(),
                },
            )
        };
        let uuid = text.trim().parse::<uuid::Uuid>().map_err(|_| missing())?;
        let schedule = ScheduleId::try_from(uuid).map_err(|_| missing())?;
        let listed = self.list_schedules(caller).await?;
        let found = listed.iter().find(|it| it.schedule == schedule);
        let name = found.map(|it| it.name.clone()).ok_or_else(missing)?;
        self.delete_schedule(caller, schedule).await?;
        Ok(format!(
            "Removed \"{name}\": it starts no more runs. Its chat stays, with its past runs."
        ))
    }
}

/// When `schedule` repeats, as a phrase within a sentence, on whose clock.
fn when_of(schedule: &ScheduleInfo) -> String {
    let when = say(Language::English, &repeats(&schedule.when, false));
    format!("{when}, {} time", schedule.zone)
}

/// When `schedule` runs next, on its own clock, as a sentence after another; nothing while it
/// runs no more.
fn next_of(schedule: &ScheduleInfo) -> String {
    let zone = time_zone(&schedule.zone).ok();
    match (schedule.next, zone) {
        (Some(next), Some(zone)) => {
            let at = next.timestamp().to_zoned(zone);
            format!(" Its next run is {}.", at.strftime("%A %-d %B at %H:%M"))
        }
        _ => String::new(),
    }
}

/// One schedule, for the model: its id, name, when it repeats, and whether it is paused.
fn line(schedule: &ScheduleInfo) -> String {
    let paused = if schedule.paused { " Paused." } else { "" };
    format!(
        "- {} \"{}\": {}.{paused}{}",
        schedule.schedule,
        schedule.name,
        when_of(schedule),
        next_of(schedule)
    )
}
