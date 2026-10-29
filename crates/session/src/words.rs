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
pub(crate) use self::say::{caution_in, say, word};
pub(crate) use self::sentence::Sentence;

use self::events::{asks, event_in};
use crate::protocol::{ApiError, AwakeInfo, Reply, SearchInfo, Told, Usage};

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
        Ok(Reply::Search {
            search: SearchInfo::Chosen { detail, .. },
        }) => detail.translatable(),
        Ok(
            Reply::Search {
                search: SearchInfo::Never { reason },
            }
            | Reply::Awake {
                awake: AwakeInfo::Cannot { reason },
            },
        ) => reason.translatable(),
        Ok(Reply::Sources { sources }) => sources.iter().any(|each| each.sentence.translatable()),
        Ok(Reply::Changes { changes }) => changes.note.translatable(),
        Ok(Reply::Steps { steps }) => steps.runs.iter().any(|run| run.sentence.translatable()),
        Ok(Reply::Plan { plan: Some(plan) }) => plan.sentence.translatable(),
        Ok(Reply::Events { events, .. }) => events.iter().any(asks),
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
                awake: AwakeInfo::Cannot { reason },
            },
        ) => reason.translate(language),
        Ok(Reply::Sources { sources }) => {
            for each in sources {
                each.sentence.translate(language);
            }
        }
        Ok(Reply::Changes { changes }) => changes.note.translate(language),
        Ok(Reply::Steps { steps }) => {
            for run in &mut steps.runs {
                run.sentence.translate(language);
            }
        }
        Ok(Reply::Plan { plan: Some(plan) }) => plan.sentence.translate(language),
        Ok(Reply::Events { events, .. }) => {
            for event in events {
                if let Some(written) = event_in(event, language) {
                    *event = written;
                }
            }
        }
        Ok(_) => {}
    }
}

#[cfg(test)]
mod tests;
