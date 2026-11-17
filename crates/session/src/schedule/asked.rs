//! The schedules a chat's `schedule` tool asks of ([`Schedules`]): the session's, for the
//! person the chat is for, in the project it is in. What the person is asked is the server's own
//! card (`card.rs`); each answer for the model is in English, as a tool's result is.

use std::sync::{Arc, Weak};

use glave_event::{Ask, At, Caution, Repeating, Risk, ScheduleId, When};
use glave_harness::{Asked, Asker, Confirm, Done, Repeat, Schedules};
use glave_ports::{BlobStore, BoxFuture, Caller, EventLog, ModelProvider, title_from};
use serde_json::json;

use super::NewSchedule;
use super::card::{asked_in, card_in, first_runs};
use crate::cron::Cron;
use crate::protocol::{ApiError, ScheduleInfo, code};
use crate::repeats::repeats_in;
use crate::server::{Inner, Server};
use crate::words::{Language, Sentence};
use crate::zone::time_zone;

/// The most tokens each run of a schedule the agent sets up may use: the server's own budget
/// for them, which no one is asked for.
pub(crate) const RUN_BUDGET: u64 = 200_000;

/// Whether the card shows what each run starts with, in the model's words, so a prompt under an
/// innocent name is never confirmed unseen. Awaits the owner's decision (#1510): on until then.
const SHOWS_PROMPT: bool = true;

/// The fewest minutes between two runs of a schedule the agent sets up: an hour, so a model
/// never proposes one that runs all day; the person can still set one up down to the server's
/// own floor (`Cron::for_a_schedule`) themselves. Awaits the owner's decision (#1510).
const PROPOSED_GAP: u16 = 60;

/// The server's schedules, as its chats' tool reaches them: held weakly, since the harness that
/// holds this is the server's own.
struct Asking<P, L, B>(Weak<Inner<P, L, B>>);

/// What a server's chats' `schedule` tool reaches, where schedules run (`fires`): its schedules,
/// held weakly, since the harness that holds them is the server's own.
pub(crate) fn asking<P, L, B>(
    server: &Weak<Inner<P, L, B>>,
    fires: bool,
) -> Option<Arc<dyn Schedules>>
where
    P: ModelProvider + 'static,
    L: EventLog + 'static,
    B: BlobStore + 'static,
{
    fires.then(|| Arc::new(Asking(Weak::clone(server))) as Arc<dyn Schedules>)
}

impl<P, L, B> Asking<P, L, B>
where
    P: ModelProvider + 'static,
    L: EventLog + 'static,
    B: BlobStore + 'static,
{
    /// The server, while it runs.
    fn server(&self) -> Result<Server<P, L, B>, String> {
        let inner = (self.0.upgrade())
            .ok_or_else(|| "glave is stopping; try again once it runs".to_owned())?;
        Ok(Server { inner })
    }
}

impl<P, L, B> Schedules for Asking<P, L, B>
where
    P: ModelProvider + 'static,
    L: EventLog + 'static,
    B: BlobStore + 'static,
{
    fn check<'a>(
        &'a self,
        from: &'a Asker,
        repeat: &'a Repeat,
    ) -> BoxFuture<'a, Result<Confirm, String>> {
        Box::pin(async move {
            let server = self.server()?;
            let caller = server.caller_of(from);
            let checked = server.check_repeat(&caller, from, repeat).await;
            checked.map_err(|error| error.message.to_string())
        })
    }

    fn check_removal<'a>(
        &'a self,
        from: &'a Asker,
        schedule: &'a str,
    ) -> BoxFuture<'a, Result<Confirm, String>> {
        Box::pin(async move {
            let server = self.server()?;
            let caller = server.caller_of(from);
            let found = server.named(&caller, from, schedule).await;
            let found = found.map_err(|error| error.message.to_string())?;
            let name = found.name.clone();
            Ok(Confirm {
                ask: Ask::StopRepeating { name: name.clone() },
                detail: found.name,
                caution: Some(Caution {
                    risk: Some(Risk::StopsRepeating { name }),
                    ..Caution::default()
                }),
                repeating: None,
            })
        })
    }

    fn act<'a>(&'a self, from: &'a Asker, asked: Asked) -> BoxFuture<'a, Result<Done, String>> {
        Box::pin(async move {
            let server = self.server()?;
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
    /// Who asks through `from`: the person the chat is for, in its organization, as glave acts
    /// for them on its own, never as the person on this computer unless they are.
    fn caller_of(&self, from: &Asker) -> Caller {
        self.acting_as(from.person, from.scope.org())
    }

    /// `caller`'s schedules in the project `from` is in, the only ones its chat's tool reaches.
    async fn in_project(
        &self,
        caller: &Caller,
        from: &Asker,
    ) -> Result<Vec<ScheduleInfo>, ApiError> {
        let listed = self.list_schedules(caller).await?;
        let project = from.scope.project();
        Ok(listed
            .into_iter()
            .filter(|it| it.project == project)
            .collect())
    }

    /// What setting up `repeat` would ask `caller`, refused for whatever setting it up would
    /// refuse: where schedules don't run, a time that doesn't parse, never comes or runs more
    /// often than the agent may propose, no prompt, a zone glave doesn't know, a project they
    /// can't work in, or no room under policy.
    async fn check_repeat(
        &self,
        caller: &Caller,
        from: &Asker,
        repeat: &Repeat,
    ) -> Result<Confirm, ApiError> {
        if !self.inner.schedules {
            return Err(ApiError::new(
                code::NOT_AVAILABLE,
                Sentence::SchedulesNotHere,
            ));
        }
        let invalid = |why| ApiError::new(code::INVALID_PARAMS, why);
        let cron = Cron::for_a_schedule(&repeat.cron).map_err(invalid)?;
        if cron.shortest_gap() < u32::from(PROPOSED_GAP) {
            let minutes = PROPOSED_GAP;
            return Err(invalid(Sentence::CronTooOften { minutes }));
        }
        let first =
            (title_from(&repeat.prompt)).ok_or_else(|| invalid(Sentence::ScheduleNoPrompt))?;
        let zone = self.zone_of(caller).await?;
        let clocks = time_zone(&zone)?;
        let now = self.inner.harness.clock().now();
        // One that never runs was refused above; on these clocks, its first run.
        let Some(first_run) = first_runs(&cron, now, &clocks).first().copied() else {
            let text = repeat.cron.escape_debug().to_string();
            return Err(invalid(Sentence::CronNever { text }));
        };
        self.project_for(caller, Some(from.scope.project())).await?;
        self.room_for_one_more(caller).await?;
        let repeating = Repeating {
            cron: repeat.cron.clone(),
            zone,
            name: title_from(&repeat.name).unwrap_or(first),
            first: At::from(first_run),
            prompt: SHOWS_PROMPT.then(|| repeat.prompt.clone()),
        };
        let english = Language::English;
        let ask = asked_in(english, &repeating);
        let detail = card_in(english, &repeating).unwrap_or_default();
        // A schedule on a person's own computer runs only while glave does there.
        let caution = self.inner.awake.is_some().then(|| Caution {
            risk: Some(Risk::RunsWhileGlaveRuns),
            ..Caution::default()
        });
        Ok(Confirm {
            ask,
            detail,
            caution,
            repeating: Some(repeating),
        })
    }

    /// Does what a chat's `schedule` tool asks, for the person the chat is for.
    async fn asked(&self, from: &Asker, asked: Asked) -> Result<Done, ApiError> {
        let caller = self.caller_of(from);
        match asked {
            Asked::Create(repeat) => {
                let new = NewSchedule {
                    project: Some(from.scope.project()),
                    name: Some(repeat.name),
                    when: When::Cron(repeat.cron),
                    // The zone its card showed: one set meanwhile never makes another.
                    zone: repeat.zone,
                    prompt: repeat.prompt,
                    budget: RUN_BUDGET,
                    from: Some(from.thread),
                };
                let made = self.create_schedule(&caller, new).await?;
                let said = format!(
                    "Set up \"{}\": {}. Each run is a new turn in its own chat of that name, \
                     starting with the prompt; the person sees and can pause or remove it there \
                     and on their Tasks page.{}",
                    made.name,
                    when_of(&made),
                    next_of(&made)
                );
                let about = json!({
                    "schedule": made.schedule, "chat": made.thread, "repeats": made.repeats
                });
                Ok(Done {
                    said,
                    about: Some(about),
                })
            }
            Asked::List => {
                let listed = self.in_project(&caller, from).await?;
                let said = if listed.is_empty() {
                    "Nothing repeats in this project.".to_owned()
                } else {
                    let lines: Vec<String> = listed.iter().map(line).collect();
                    lines.join("\n")
                };
                Ok(Done { said, about: None })
            }
            Asked::Remove(text) => {
                let found = self.named(&caller, from, &text).await?;
                self.delete_schedule(&caller, found.schedule).await?;
                let said = format!(
                    "Removed \"{}\": it starts no more runs. Its chat stays, with its past runs.",
                    found.name
                );
                let about = json!({"schedule": found.schedule});
                Ok(Done {
                    said,
                    about: Some(about),
                })
            }
        }
    }

    /// The schedule of `caller`'s, in the project `from` is in, that `text` names by its id.
    async fn named(
        &self,
        caller: &Caller,
        from: &Asker,
        text: &str,
    ) -> Result<ScheduleInfo, ApiError> {
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
        let listed = self.in_project(caller, from).await?;
        listed
            .into_iter()
            .find(|it| it.schedule == schedule)
            .ok_or_else(missing)
    }
}

/// When `schedule` repeats, as a phrase within a sentence, on whose clock.
fn when_of(schedule: &ScheduleInfo) -> String {
    let when = repeats_in(Language::English, &schedule.when, false);
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
