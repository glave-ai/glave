//! Why a tool call needs the person, typed (`approval.requested`'s `reason`): what it would do
//! (a [`Risk`]), what of the sandbox it would run without, and how glave settled it when no one
//! was asked. The English the model reads and the record keeps is made from it, and the session
//! writes it again in the reader's language from the same parts (`specs/session.md`,
//! "Languages").

use serde::{Deserialize, Serialize};

pub use self::risk::Risk;
use crate::payload::approval::Lifted;

/// Words of glave's that a table holds: each with a message id, the values it is written with,
/// and its English, a pattern in Fluent's syntax naming each value as `{ $name }`, so the
/// session's catalog writes it in any language and its English is the same everywhere.
pub trait Worded {
    /// Its message's id in the catalog.
    fn id(&self) -> &'static str;
    /// Its English, a pattern naming each value as `{ $name }`.
    fn pattern(&self) -> &'static str;
    /// The values it is written with, by name.
    fn values(&self) -> Vec<(&'static str, &str)>;
    /// In English: its pattern with its values in place.
    fn english(&self) -> String {
        fill(self.pattern(), &self.values())
    }
}

/// Makes a table of words: each line `Variant { value, … } = "id" "English"`, its values text.
macro_rules! worded {
    ($(#[$doc:meta])* $name:ident { $(
        $variant:ident $({ $($value:ident),+ $(,)? })? = $id:literal $english:literal,
    )* }) => {
        $(#[$doc])*
        #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
        #[serde(tag = "says")]
        pub enum $name {
            $(
                #[serde(rename = $id)]
                $variant $({ $($value: String),+ })?,
            )*
        }

        impl $name {
            /// Each one's message id, the names of the values it is written with, and its
            /// English.
            pub const ALL: &[(&str, &[&str], &str)] = &[
                $(($id, &[$($(stringify!($value)),+)?], $english),)*
            ];

            /// Each one, its values named as themselves (`<path>`): for a table that writes
            /// them again to hold its words to these.
            pub fn every() -> Vec<Self> {
                vec![$(
                    Self::$variant $({ $(
                        $value: concat!("<", stringify!($value), ">").to_owned()
                    ),+ })?,
                )*]
            }
        }

        impl Worded for $name {
            fn id(&self) -> &'static str {
                match self {
                    $(Self::$variant { .. } => $id,)*
                }
            }

            fn pattern(&self) -> &'static str {
                match self {
                    $(Self::$variant { .. } => $english,)*
                }
            }

            fn values(&self) -> Vec<(&'static str, &str)> {
                match self {
                    $(Self::$variant $({ $($value),+ })? => vec![
                        $($((stringify!($value), $value.as_str())),+)?
                    ],)*
                }
            }
        }
    };
}

mod risk;

worded! {
    /// How a caution's parts are put together, and how glave settled it.
    Phrase {
        Outside = "caution-outside"
            "it runs outside the sandbox, where it can read and change any file you can",
        WithNetwork = "caution-network"
            "it can reach the internet, which commands here otherwise cannot",
        And { risk, lifted } = "caution-and" "{ $risk }, and { $lifted }",
        Cleared = "caution-cleared" "allowed without asking",
        ClearedBecause { why } = "caution-cleared-because" "allowed without asking: { $why }",
        Declined = "caution-declined"
            "declined without asking: Full access never shows the model a secret",
    }
}

/// Why a call needs the person in any mode that asks.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Caution {
    /// What the call would do that needs the person, when that is why.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub risk: Option<Risk>,
    /// What of the sandbox the command would run without, when it would.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lifted: Option<Lifted>,
    /// How glave settled it without asking anyone, when it did.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub settled: Option<Settled>,
}

/// How glave settled a caution without asking anyone.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Settled {
    /// The judge let it run, with its own words for why when it gave any.
    Cleared {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        why: Option<String>,
    },
    /// Full access declined it: it never shows the model a secret.
    Declined,
}

impl Caution {
    /// The caution for a call that would do `risk`, or run without `lifted` of the sandbox:
    /// none when it would do neither.
    pub fn of(risk: Option<Risk>, lifted: Option<Lifted>) -> Option<Self> {
        (risk.is_some() || lifted.is_some()).then_some(Self {
            risk,
            lifted,
            settled: None,
        })
    }

    /// Why the call needs the person, as the model and the judge are told it: "it deletes ~/x,
    /// and it runs outside the sandbox, …". None when only how glave settled it is known.
    pub fn because(&self) -> Option<String> {
        self.clause(&|words| words.english())
    }

    /// The caution as the person reads it, in English, as `approval.requested` records it:
    /// why, a sentence of its own, then how glave settled it.
    pub fn sentences(&self) -> Option<String> {
        self.written(&|words| words.english())
    }

    /// The caution as the person reads it, each part written by `write`: in English, or in the
    /// reader's language.
    pub fn written(&self, write: &dyn Fn(&dyn Worded) -> String) -> Option<String> {
        let settled = self.settled.as_ref().map(|settled| match settled {
            Settled::Cleared { why: None } => write(&Phrase::Cleared),
            Settled::Cleared { why: Some(why) } => write(&Phrase::ClearedBecause {
                why: why.trim().to_owned(),
            }),
            Settled::Declined => write(&Phrase::Declined),
        });
        let parts: Vec<String> = self
            .clause(write)
            .into_iter()
            .chain(settled)
            .map(|part| sentence(&part))
            .collect();
        (!parts.is_empty()).then(|| parts.join(" "))
    }

    /// What it would do, then what of the sandbox it would run without, in one clause.
    fn clause(&self, write: &dyn Fn(&dyn Worded) -> String) -> Option<String> {
        let lifted = self.lifted.map(|lifted| match lifted {
            Lifted::Sandbox => Phrase::Outside,
            Lifted::Network => Phrase::WithNetwork,
        });
        match (&self.risk, lifted) {
            (Some(risk), Some(lifted)) => Some(write(&Phrase::And {
                risk: write(risk),
                lifted: write(&lifted),
            })),
            (Some(risk), None) => Some(write(risk)),
            (None, Some(lifted)) => Some(write(&lifted)),
            (None, None) => None,
        }
    }
}

/// A clause as a person reads it, a sentence of its own: "it deletes ~/x" is "It deletes
/// ~/x.".
pub fn sentence(clause: &str) -> String {
    let clause = clause.trim();
    let mut chars = clause.chars();
    let mut sentence: String = chars
        .next()
        .map(char::to_uppercase)
        .into_iter()
        .flatten()
        .chain(chars)
        .collect();
    if !sentence.is_empty() && !sentence.ends_with(['.', '!', '?']) {
        sentence.push('.');
    }
    sentence
}

/// `pattern` with each `{ $name }` it names written as its value; one it has no value for is
/// left as it is.
fn fill(pattern: &str, values: &[(&str, &str)]) -> String {
    let mut written = String::with_capacity(pattern.len());
    let mut rest = pattern;
    while let Some(at) = rest.find("{ $") {
        let (before, from) = rest.split_at(at);
        written.push_str(before);
        let named = from.find(" }").and_then(|end| {
            let name = from.get(3..end)?;
            let (_, value) = values.iter().find(|(each, _)| *each == name)?;
            Some((value, end.saturating_add(2)))
        });
        let Some((value, after)) = named else {
            written.push_str(from);
            return written;
        };
        written.push_str(value);
        rest = from.get(after..).unwrap_or_default();
    }
    written.push_str(rest);
    written
}

#[cfg(test)]
mod tests;
