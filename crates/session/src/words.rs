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

use crate::protocol::{ApiError, Reply};

/// Whether `answered` holds a sentence of the server's own, which a transport writes in the
/// caller's language.
pub(crate) fn says(answered: &Result<Reply, ApiError>) -> bool {
    match answered {
        Err(error) => error.message.translatable(),
        Ok(Reply::Projects { gone, .. }) => gone.iter().any(|gone| gone.message.translatable()),
        Ok(_) => false,
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
        Ok(_) => {}
    }
}

#[cfg(test)]
mod tests;
