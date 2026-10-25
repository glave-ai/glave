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

use crate::protocol::{ApiError, ConnectionInfo, Reply, Told};

/// Whether `answered` holds a sentence of the server's own, which a transport writes in the
/// caller's language.
pub(crate) fn says(answered: &Result<Reply, ApiError>) -> bool {
    match answered {
        Err(error) => error.message.translatable(),
        Ok(Reply::Projects { gone, .. }) => gone.iter().any(|gone| gone.message.translatable()),
        Ok(Reply::Folders { refused, .. }) => refused.as_ref().is_some_and(Told::translatable),
        Ok(Reply::Connections { connections }) => connections.iter().any(speaks),
        Ok(Reply::Connection { connection }) => speaks(connection),
        Ok(_) => false,
    }
}

/// Whether `connection` holds a sentence of the server's own: what a suggestion is for, or how
/// much it was used.
fn speaks(connection: &ConnectionInfo) -> bool {
    let suggested = connection.suggested.as_ref();
    suggested.is_some_and(|suggested| suggested.title.translatable())
        || connection
            .usage
            .as_ref()
            .is_some_and(|usage| usage.sentence.translatable())
}

/// Writes the server's own sentences in `connection` in `language`.
fn translate_connection(connection: &mut ConnectionInfo, language: Language) {
    if let Some(suggested) = &mut connection.suggested {
        suggested.title.translate(language);
        suggested.detail.translate(language);
    }
    if let Some(usage) = &mut connection.usage {
        usage.sentence.translate(language);
    }
}

/// Writes the server's own sentences in `answered` in `language`.
pub(crate) fn translate(answered: &mut Result<Reply, ApiError>, language: Language) {
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
                translate_connection(connection, language);
            }
        }
        Ok(Reply::Connection { connection }) => translate_connection(connection, language),
        Ok(_) => {}
    }
}

#[cfg(test)]
mod tests;
