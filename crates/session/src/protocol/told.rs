//! What a person is told in a sentence, as JSON-RPC carries it: a string.

use std::fmt;
use std::ops::Deref;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::words::{Language, Sentence, say};

/// A sentence for a person: as they read it, and, when it is the server's own, what it says, so
/// a transport writes it again in the caller's language (`specs/session.md`, "Languages"). The
/// server's own is written in English where it is made, so what is logged or kept reads the
/// same everywhere. It travels as its text.
#[derive(Debug, Clone)]
pub struct Told {
    text: String,
    sentence: Option<Box<Sentence>>,
}

impl Told {
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

/// The server's own sentence, in English.
impl From<Sentence> for Told {
    fn from(sentence: Sentence) -> Self {
        Self {
            text: say(Language::English, &sentence),
            sentence: Some(Box::new(sentence)),
        }
    }
}

/// Words another gave the server to pass on, as they are.
impl From<String> for Told {
    fn from(text: String) -> Self {
        Self {
            text,
            sentence: None,
        }
    }
}

impl From<&str> for Told {
    fn from(text: &str) -> Self {
        Self::from(text.to_owned())
    }
}

impl Deref for Told {
    type Target = str;

    fn deref(&self) -> &str {
        &self.text
    }
}

impl fmt::Display for Told {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.text)
    }
}

/// Two sentences are the same when they read the same.
impl PartialEq for Told {
    fn eq(&self, other: &Self) -> bool {
        self.text == other.text
    }
}

impl Eq for Told {}

impl PartialEq<str> for Told {
    fn eq(&self, other: &str) -> bool {
        self.text == other
    }
}

impl PartialEq<&str> for Told {
    fn eq(&self, other: &&str) -> bool {
        self.text == *other
    }
}

impl Serialize for Told {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.text)
    }
}

impl<'de> Deserialize<'de> for Told {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        String::deserialize(deserializer).map(Self::from)
    }
}

/// A string, as the schema has always said.
impl schemars::JsonSchema for Told {
    fn inline_schema() -> bool {
        true
    }

    fn schema_name() -> std::borrow::Cow<'static, str> {
        String::schema_name()
    }

    fn json_schema(generator: &mut schemars::SchemaGenerator) -> schemars::Schema {
        String::json_schema(generator)
    }
}
