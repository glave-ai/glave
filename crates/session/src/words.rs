//! The server's own words, in the caller's language (`specs/session.md`, "Languages"). Every
//! sentence the session API writes for a person is a [`Sentence`]: a message of a Fluent
//! catalog, written in English where it is made, so what is logged, kept or told to a model
//! reads the same everywhere, and again in the caller's language where a transport answers
//! them (`in_language`).

mod events;
mod language;
mod say;
mod sentence;

pub(crate) use self::events::note_in;
pub(crate) use self::language::Language;
pub(crate) use self::say::{catalogued, caution_in, say, word};
pub(crate) use self::sentence::Sentence;

use self::events::{asks, event_in};
use crate::protocol::{
    ApiError, AwakeInfo, ConnectorCategory, ConnectorInfo, ConnectorTag, ListedCommand, PushInfo,
    Reply, ScheduleInfo, SearchInfo, ThreadSummary, Told, Usage,
};

/// Whether `answered` holds a sentence of the server's own, which a transport writes in the
/// caller's language.
pub(crate) fn says(answered: &Result<Reply, ApiError>) -> bool {
    let used = |usage: &Option<Usage>| usage.as_ref().is_some_and(|it| it.sentence.translatable());
    match answered {
        Err(error) => error.message.translatable(),
        Ok(Reply::Projects { gone, .. }) => gone.iter().any(|gone| gone.message.translatable()),
        Ok(Reply::Folders { refused, .. }) => refused.as_ref().is_some_and(Told::translatable),
        Ok(Reply::Connections { connections }) => connections.iter().any(|it| used(&it.usage)),
        Ok(Reply::Connection { connection }) => used(&connection.usage),
        Ok(
            Reply::Connectors { .. }
            | Reply::Connector { .. }
            | Reply::Files { .. }
            | Reply::FilePreview { .. }
            | Reply::Share { share: Some(_) }
            | Reply::Schedule { .. },
        ) => true,
        Ok(Reply::Library { items }) => items.iter().any(|item| item.shared.is_some()),
        Ok(Reply::Search {
            search: SearchInfo::Chosen { detail, .. },
        }) => detail.translatable(),
        Ok(
            Reply::Search {
                search: SearchInfo::Never { reason },
            }
            | Reply::Awake {
                awake: AwakeInfo::Cannot { reason } | AwakeInfo::Never { reason },
            }
            | Reply::Push {
                push: PushInfo::Never { reason },
            },
        ) => reason.translatable(),
        Ok(Reply::Sources { sources }) => sources.iter().any(|each| each.sentence.translatable()),
        Ok(Reply::Changes { changes }) => changes.note.translatable(),
        Ok(Reply::Commands { commands, .. }) => commands
            .iter()
            .flat_map(listening_told)
            .any(Told::translatable),
        Ok(Reply::Passkeys {
            notice: Some(notice),
            ..
        }) => notice.translatable(),
        Ok(Reply::Organizations { organizations }) => {
            organizations.iter().any(|org| org.name.translatable())
        }
        Ok(Reply::Steps { steps }) => steps.runs.iter().any(|run| run.sentence.translatable()),
        Ok(Reply::Plan { plan: Some(plan) }) => plan.sentence.translatable(),
        Ok(Reply::Schedules { schedules }) => !schedules.is_empty(),
        Ok(Reply::Events { events, .. }) => events.iter().any(asks),
        Ok(Reply::Thread { thread }) => in_its_own_words(thread),
        Ok(Reply::Threads { threads, .. }) => threads.iter().any(in_its_own_words),
        Ok(_) => false,
    }
}

/// Writes the server's own sentences in `answered` in `language`.
pub(crate) fn translate(answered: &mut Result<Reply, ApiError>, language: Language) {
    let used = |usage: &mut Option<Usage>| {
        if let Some(usage) = usage {
            usage.sentence.translate(language);
        }
    };
    match answered {
        Err(error) => error.message.translate(language),
        Ok(Reply::Projects { gone, .. }) => {
            for each in gone {
                each.message.translate(language);
            }
        }
        Ok(Reply::Folders {
            refused: Some(refused),
            ..
        }) => refused.translate(language),
        Ok(Reply::Connections { connections }) => {
            for connection in connections {
                used(&mut connection.usage);
            }
        }
        Ok(Reply::Connection { connection }) => used(&mut connection.usage),
        Ok(Reply::Connectors {
            categories,
            tags,
            connectors,
            ..
        }) => connectors_in(categories, tags, connectors, language),
        Ok(Reply::Connector { connector }) => connector.description.translate(language),
        Ok(Reply::Search {
            search: SearchInfo::Chosen { detail, usage, .. },
        }) => {
            detail.translate(language);
            used(usage);
        }
        Ok(
            Reply::Search {
                search: SearchInfo::Never { reason },
            }
            | Reply::Awake {
                awake: AwakeInfo::Cannot { reason } | AwakeInfo::Never { reason },
            }
            | Reply::Push {
                push: PushInfo::Never { reason },
            },
        ) => reason.translate(language),
        Ok(Reply::Sources { sources }) => {
            for each in sources {
                each.sentence.translate(language);
            }
        }
        Ok(Reply::Changes { changes }) => changes.note.translate(language),
        Ok(Reply::Commands { commands, .. }) => commands_in(commands, language),
        Ok(Reply::Passkeys {
            notice: Some(notice),
            ..
        }) => notice.translate(language),
        Ok(Reply::Organizations { organizations }) => {
            for org in organizations {
                org.name.translate(language);
            }
        }
        Ok(Reply::Steps { steps }) => {
            for run in &mut steps.runs {
                run.sentence.translate(language);
            }
        }
        Ok(Reply::Plan { plan: Some(plan) }) => plan.sentence.translate(language),
        Ok(Reply::Schedule { schedule }) => schedule_in(schedule, language),
        Ok(Reply::Schedules { schedules }) => schedules_in(schedules, language),
        Ok(Reply::Events { events, .. }) => events_in(events, language),
        Ok(Reply::Thread { thread }) => summary_in(thread, language),
        Ok(Reply::Threads { threads, .. }) => {
            for thread in threads {
                summary_in(thread, language);
            }
        }
        // The files threads made, a preview, a share and the Library, each in its own words.
        Ok(reply) => made_in(reply, language),
    }
}

/// Writes the sentences of the files threads made in `language`: where each is kept, its parts and
/// what was made, and what a preview left out.
fn made_in(reply: &mut Reply, language: Language) {
    match reply {
        Reply::Files { made } => {
            for turn in made.iter_mut() {
                if let Some(note) = &mut turn.note {
                    note.translate(language);
                }
                for file in &mut turn.files {
                    file.kept.translate(language);
                    file.sentence.translate(language);
                    let told = [&mut file.parts, &mut file.unread, &mut file.shared];
                    for told in told.into_iter().flatten() {
                        told.translate(language);
                    }
                }
            }
        }
        Reply::FilePreview { preview } => {
            if let Some(cut) = &mut preview.cut {
                cut.translate(language);
            }
        }
        Reply::Share { share: Some(share) } => share.said.translate(language),
        Reply::Library { items } => {
            for told in items.iter_mut().filter_map(|item| item.shared.as_mut()) {
                told.translate(language);
            }
        }
        _ => {}
    }
}

/// Writes when each of `schedules` repeats in `language`.
fn schedules_in(schedules: &mut [ScheduleInfo], language: Language) {
    for schedule in schedules {
        schedule_in(schedule, language);
    }
}

/// Writes `schedule`'s own words in `language`: when it repeats, how its last run went, and
/// its note.
fn schedule_in(schedule: &mut ScheduleInfo, language: Language) {
    schedule.repeats.translate(language);
    schedule.stop.translate(language);
    if let Some(last) = &mut schedule.last {
        last.said.translate(language);
    }
    if let Some(note) = &mut schedule.note {
        note.translate(language);
    }
}

/// Writes what `commands` say of where they listen in `language`.
fn commands_in(commands: &mut [ListedCommand], language: Language) {
    for command in commands {
        if let Some(listens) = &mut command.listens {
            listens.translate(language);
        }
        for port in &mut command.ports {
            if let Some(unframed) = &mut port.unframed {
                unframed.translate(language);
            }
        }
    }
}

/// Writes the server's own sentences in `events` in `language`.
fn events_in(events: &mut [glave_event::RawEvent], language: Language) {
    for event in events {
        if let Some(written) = event_in(event, language) {
            *event = written;
        }
    }
}

/// What `command` says of where it listens.
fn listening_told(command: &ListedCommand) -> impl Iterator<Item = &Told> {
    let unframed = command
        .ports
        .iter()
        .filter_map(|port| port.unframed.as_ref());
    command.listens.iter().chain(unframed)
}

/// Writes the catalog's words for `categories`, `tags` and `connectors` in `language`.
fn connectors_in(
    categories: &mut [ConnectorCategory],
    tags: &mut [ConnectorTag],
    connectors: &mut [ConnectorInfo],
    language: Language,
) {
    for category in categories {
        category.name.translate(language);
    }
    for tag in tags {
        tag.name.translate(language);
    }
    for connector in connectors {
        connector.description.translate(language);
    }
}

/// Whether `thread` says what its project asks to be trusted, that its computer is short of
/// room, or its schedule, in the server's own words.
fn in_its_own_words(thread: &ThreadSummary) -> bool {
    thread
        .trust
        .iter()
        .flat_map(|ask| &ask.items)
        .any(|item| item.text.translatable())
        || thread.disk.is_some()
        || thread.schedule.is_some()
}

/// Writes what `thread`'s project asks to be trusted, what it says of room, and its schedule,
/// in `language`.
pub(crate) fn summary_in(thread: &mut ThreadSummary, language: Language) {
    if let Some(ask) = &mut thread.trust {
        for item in &mut ask.items {
            item.text.translate(language);
        }
    }
    if let Some(disk) = &mut thread.disk {
        disk.told.translate(language);
    }
    if let Some(schedule) = &mut thread.schedule {
        schedule_in(schedule, language);
    }
}

#[cfg(test)]
mod tests;
