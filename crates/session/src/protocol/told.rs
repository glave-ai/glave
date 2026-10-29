//! What a person is told in a sentence, as JSON-RPC carries it: a string.

use std::fmt;
use std::ops::Deref;

use glave_event::{Caution, Worded};
use glave_ports::ProjectRefusal;
use glave_ports::computer::{Reason, Refusal};
use glave_ports::memories::MemoryRefusal;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::words::{Language, Sentence, caution_in, say, word};

/// A sentence for a person: as they read it, and, when it is the server's own, what it says, so
/// a transport writes it again in the caller's language (`specs/session.md`, "Languages"). The
/// server's own is written in English where it is made, so what is logged or kept reads the
/// same everywhere. It travels as its text.
#[derive(Debug, Clone)]
pub struct Told {
    text: String,
    /// What it says, when it is the server's own; boxed, since most are not, and errors carry
    /// one.
    said: Option<Box<Said>>,
}

/// What the server's own words say.
#[derive(Debug, Clone)]
enum Said {
    /// One sentence, or the clauses of one, each carrying what comes before it ("Ran 2
    /// commands", ", read 1 file").
    Sentences(Vec<Sentence>),
    /// Why a call needs the person, from the parts its `approval.requested` records.
    Caution(Caution),
    /// Words of another crate's table.
    Theirs(Theirs),
}

/// Words of another crate's table that the server passes on.
#[derive(Debug, Clone)]
enum Theirs {
    /// A computer's refusal.
    Computer(Refusal),
    /// A project's refusal.
    Project(ProjectRefusal),
    /// Memory's refusal.
    Memory(MemoryRefusal),
}

impl Theirs {
    fn words(&self) -> &dyn Worded {
        match self {
            Self::Computer(refusal) => refusal,
            Self::Project(refusal) => refusal,
            Self::Memory(refusal) => refusal,
        }
    }
}

impl Told {
    /// The server's own clauses as one sentence, in English, each written after the last.
    pub(crate) fn joined(said: Vec<Sentence>) -> Self {
        Self {
            text: said_in(Language::English, &said),
            said: Some(Box::new(Said::Sentences(said))),
        }
    }

    /// Why a call needs the person, in English as its `approval.requested` records it: `None`
    /// when the caution says nothing.
    pub(crate) fn caution(caution: Caution) -> Option<Self> {
        Some(Self {
            text: caution.sentences()?,
            said: Some(Box::new(Said::Caution(caution))),
        })
    }

    /// Whether a transport writes it again in the caller's language: it is the server's own.
    pub(crate) fn translatable(&self) -> bool {
        self.said.is_some()
    }

    /// Writes it in `language`, when it is the server's own; any other is left as it is.
    pub(crate) fn translate(&mut self, language: Language) {
        let written = match self.said.as_deref() {
            Some(Said::Sentences(said)) => Some(said_in(language, said)),
            Some(Said::Caution(caution)) => caution_in(language, caution),
            Some(Said::Theirs(theirs)) => Some(word(language, theirs.words())),
            None => None,
        };
        if let Some(written) = written {
            self.text = written;
        }
    }
}

/// `said` written in `language`, each after the last.
fn said_in(language: Language, said: &[Sentence]) -> String {
    said.iter()
        .map(|sentence| say(language, sentence))
        .collect()
}

/// The server's own sentence, in English.
impl From<Sentence> for Told {
    fn from(sentence: Sentence) -> Self {
        Self::joined(vec![sentence])
    }
}

/// A computer's refusal: written again in the caller's language when glave's own words said it,
/// else passed on as it is.
impl From<&Reason> for Told {
    fn from(reason: &Reason) -> Self {
        Self {
            text: reason.to_string(),
            said: reason
                .refusal()
                .map(|refusal| Box::new(Said::Theirs(Theirs::Computer(refusal.clone())))),
        }
    }
}

/// Memory's refusal, written again in the caller's language.
impl From<MemoryRefusal> for Told {
    fn from(refusal: MemoryRefusal) -> Self {
        Self {
            text: refusal.english(),
            said: Some(Box::new(Said::Theirs(Theirs::Memory(refusal)))),
        }
    }
}

/// A project's refusal, written again in the caller's language.
impl From<ProjectRefusal> for Told {
    fn from(refusal: ProjectRefusal) -> Self {
        Self {
            text: refusal.english(),
            said: Some(Box::new(Said::Theirs(Theirs::Project(refusal)))),
        }
    }
}

/// Words another gave the server to pass on, as they are.
impl From<String> for Told {
    fn from(text: String) -> Self {
        Self { text, said: None }
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
