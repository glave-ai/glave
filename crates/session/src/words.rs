//! The server's own words, in the caller's language (`specs/session.md`, "Languages"). Every
//! sentence the session API writes for a person is a [`Sentence`]: a message of a Fluent
//! catalog, written in English where it is made, so what is logged, kept or told to a model
//! reads the same everywhere, and again in the caller's language where a transport answers
//! them (`in_language`).

mod language;
mod say;
mod sentence;

pub(crate) use self::language::Language;
pub(crate) use self::say::say;
pub(crate) use self::sentence::Sentence;

use crate::protocol::{ApiError, Reply, SearchInfo, Told, Usage};

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
        Ok(Reply::Search {
            search: SearchInfo::Never { reason },
        }) => reason.translatable(),
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
        Ok(Reply::Search {
            search: SearchInfo::Never { reason },
        }) => reason.translate(language),
        Ok(_) => {}
    }
}

#[cfg(test)]
mod tests;
