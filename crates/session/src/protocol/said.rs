//! A sentence the server writes for a person, as JSON-RPC carries it: a string.

use std::fmt;
use std::ops::Deref;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::words::{Language, Sentence, say};

/// A sentence for a person: as they read it, and, when it is the server's own, what it says, so
/// a transport writes it again in the caller's language (`specs/session.md`, "Languages").
/// Written in English where it is made, so what is logged or kept reads the same everywhere; it
/// travels as its text.
#[derive(Debug, Clone)]
pub struct Said {
    text: String,
    sentence: Option<Sentence>,
}

impl Said {
    /// The server's own sentence, in English.
    pub(crate) fn of(sentence: Sentence) -> Self {
        Self {
            text: say(Language::English, &sentence),
            sentence: Some(sentence),
        }
    }

    /// Its text.
    pub fn as_str(&self) -> &str {
        &self.text
    }

    /// Whether a transport writes it again in the caller's language: it is the server's own.
    pub(crate) fn translatable(&self) -> bool {
        self.sentence.is_some()
    }

    /// Writes it in `language`, when it is the server's own; any other is left as it is.
    pub(crate) fn translate(&mut self, language: Language) {
        if let Some(sentence) = &self.sentence {
            self.text = say(language, sentence);
        }
    }
}

impl From<String> for Said {
    fn from(text: String) -> Self {
        Self {
            text,
            sentence: None,
        }
    }
}

impl From<&str> for Said {
    fn from(text: &str) -> Self {
        Self::from(text.to_owned())
    }
}

impl From<Sentence> for Said {
    fn from(sentence: Sentence) -> Self {
        Self::of(sentence)
    }
}

impl Deref for Said {
    type Target = str;

    fn deref(&self) -> &str {
        &self.text
    }
}

impl fmt::Display for Said {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.text)
    }
}

/// Two sentences are the same when they read the same.
impl PartialEq for Said {
    fn eq(&self, other: &Self) -> bool {
        self.text == other.text
    }
}

impl Eq for Said {}

impl PartialEq<str> for Said {
    fn eq(&self, other: &str) -> bool {
        self.text == other
    }
}

impl PartialEq<&str> for Said {
    fn eq(&self, other: &&str) -> bool {
        self.text == *other
    }
}

impl Serialize for Said {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.text)
    }
}

impl<'de> Deserialize<'de> for Said {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        String::deserialize(deserializer).map(Self::from)
    }
}

impl schemars::JsonSchema for Said {
    fn inline_schema() -> bool {
        true
    }

    fn schema_name() -> std::borrow::Cow<'static, str> {
        "String".into()
    }

    fn json_schema(generator: &mut schemars::SchemaGenerator) -> schemars::Schema {
        String::json_schema(generator)
    }
}
