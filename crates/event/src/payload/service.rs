//! `service.asked` and `service.answered`: a connected service asks the person something in the
//! middle of a call (MCP's elicitation), and their answer. Linked by the call's `call_id`; a call
//! may ask more than once. What the person typed, and what the service needs back with it, is
//! kept in blobs the events only point to, so no client or search ever reads it from an event.

use serde::{Deserialize, Serialize};
use serde_json::value::RawValue;
use serde_json::{Map, Number, Value};

use crate::blob::BlobRef;
use crate::codec::{Payload, parse};
use crate::id::CallId;
use crate::payload::principal::Principal;

pub(crate) const ASKED: &str = "service.asked";
pub(crate) const ANSWERED: &str = "service.answered";

/// A call waits for the person to answer what the service it runs on asks.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ServiceAsked {
    pub call_id: CallId,
    /// The service, by the name the person knows it by: shown as who asks, never altered.
    pub service: String,
    /// Who asks, in a sentence of glave's, so a tool cannot pass as the model: "GitHub asks you,
    /// not the model". Recorded in English, written again in the reader's language.
    pub ask: String,
    /// Where the answer goes, in sentences of glave's: "Your answer goes only to GitHub. The
    /// model sees that you answered, not what you typed." Recorded in English, written again in
    /// the reader's language.
    pub note: String,
    /// What the service asks, in its own words: outside text, shown as the service's.
    pub message: String,
    /// What it asks for, in the order it listed them.
    pub fields: Vec<ServiceField>,
    /// Where the source keeps what it needs to go on, which no client reads.
    pub state: BlobRef,
}

/// One thing a service asks for.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ServiceField {
    /// The name its answer goes back under.
    pub name: String,
    /// What the person is shown: the service's title for it, else its name.
    pub label: String,
    /// More about it, in the service's words.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Whether an answer needs it.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub required: bool,
    pub kind: ServiceFieldKind,
}

/// What kind of answer a field takes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "type")]
pub enum ServiceFieldKind {
    /// Words.
    Text {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        default: Option<String>,
    },
    /// A number; a whole one when `integer`.
    Number {
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        integer: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        default: Option<Number>,
    },
    /// Yes or no.
    YesNo {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        default: Option<bool>,
    },
    /// One of `options`, or any of them when `multiple`.
    Choice {
        options: Vec<ServiceOption>,
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        multiple: bool,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        default: Vec<String>,
    },
}

/// One option of a choice.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ServiceOption {
    /// What goes back when it is picked.
    pub value: String,
    /// What the person is shown.
    pub label: String,
}

impl ServiceField {
    /// Whether `value` answers this field, as its kind says.
    fn accepts(&self, value: &Value) -> bool {
        match (&self.kind, value) {
            (ServiceFieldKind::Text { .. }, Value::String(text)) => {
                !self.required || !text.trim().is_empty()
            }
            (ServiceFieldKind::Number { integer, .. }, Value::Number(number)) => {
                !integer || number.is_i64() || number.is_u64()
            }
            (ServiceFieldKind::YesNo { .. }, Value::Bool(_)) => true,
            (
                ServiceFieldKind::Choice {
                    options,
                    multiple: false,
                    ..
                },
                Value::String(picked),
            ) => options.iter().any(|option| option.value == *picked),
            (
                ServiceFieldKind::Choice {
                    options,
                    multiple: true,
                    ..
                },
                Value::Array(picked),
            ) => {
                (!self.required || !picked.is_empty())
                    && picked.iter().all(|picked| {
                        picked.as_str().is_some_and(|picked| {
                            options.iter().any(|option| option.value == picked)
                        })
                    })
            }
            _ => false,
        }
    }
}

/// The name of the first field `values` does not answer as `fields` ask: one it leaves out that
/// an answer needs, one it answers wrongly, or one nothing asked for. `None` when all fit.
pub fn misfit<'a>(fields: &'a [ServiceField], values: &'a Map<String, Value>) -> Option<&'a str> {
    let unasked = values
        .keys()
        .find(|name| !fields.iter().any(|field| field.name == **name));
    let wrong = fields.iter().find(|field| match values.get(&field.name) {
        Some(value) => !field.accepts(value),
        None => field.required,
    });
    unasked
        .map(String::as_str)
        .or_else(|| wrong.map(|field| field.name.as_str()))
}

/// The person's answer to what a service asks.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ServiceAnswer {
    /// They answered: each field's name, with its value.
    Answered { values: Map<String, Value> },
    /// They said no: the service is told so, and nothing else.
    Declined,
}

/// The answer to a `service.asked`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ServiceAnswered {
    pub call_id: CallId,
    /// The service, as its `service.asked` names it.
    pub service: String,
    pub outcome: ServiceOutcome,
    /// What came of it, in a line of glave's: "You answered GitHub". Recorded in English, written
    /// again in the reader's language.
    pub told: String,
    /// Where what the person answered is kept, which no client reads, when they answered.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub answers: Option<BlobRef>,
    /// Who answered. Absent: the thread's `for`. A stopped turn's is glave's.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub by: Option<Principal>,
}

/// What came of what a service asked.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ServiceOutcome {
    Answered,
    Declined,
    /// The turn was stopped while it waited.
    Cancelled,
}

impl Payload for ServiceAsked {
    const TYPE: &'static str = ASKED;
    const VERSION: u32 = 1;
}

impl Payload for ServiceAnswered {
    const TYPE: &'static str = ANSWERED;
    const VERSION: u32 = 1;
}

pub(crate) fn decode_asked(
    version: u32,
    data: &RawValue,
) -> Option<serde_json::Result<ServiceAsked>> {
    match version {
        1 => Some(parse(data)),
        _ => None,
    }
}

pub(crate) fn decode_answered(
    version: u32,
    data: &RawValue,
) -> Option<serde_json::Result<ServiceAnswered>> {
    match version {
        1 => Some(parse(data)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn field(name: &str, required: bool, kind: ServiceFieldKind) -> ServiceField {
        ServiceField {
            name: name.to_owned(),
            label: name.to_owned(),
            description: None,
            required,
            kind,
        }
    }

    fn options() -> Vec<ServiceOption> {
        let option = |value: &str| ServiceOption {
            value: value.to_owned(),
            label: value.to_uppercase(),
        };
        vec![option("a"), option("b")]
    }

    fn form() -> Vec<ServiceField> {
        vec![
            field("words", true, ServiceFieldKind::Text { default: None }),
            field(
                "count",
                false,
                ServiceFieldKind::Number {
                    integer: true,
                    default: None,
                },
            ),
            field("sure", false, ServiceFieldKind::YesNo { default: None }),
            field(
                "one",
                false,
                ServiceFieldKind::Choice {
                    options: options(),
                    multiple: false,
                    default: Vec::new(),
                },
            ),
            field(
                "many",
                false,
                ServiceFieldKind::Choice {
                    options: options(),
                    multiple: true,
                    default: Vec::new(),
                },
            ),
        ]
    }

    /// The field `values` do not fit, or nothing when they all fit.
    fn checked(values: &Value) -> String {
        let fields = form();
        let values = values.as_object().unwrap();
        misfit(&fields, values).unwrap_or_default().to_owned()
    }

    #[test]
    fn an_answer_fits_only_what_was_asked() {
        assert_eq!(
            checked(&json!({
                "words": "hi", "count": 2, "sure": true, "one": "a", "many": ["a", "b"]
            })),
            ""
        );
        assert_eq!(checked(&json!({"words": "hi"})), "", "the rest is optional");
        for (wrong, values) in [
            ("words", json!({})),
            ("words", json!({"words": "  "})),
            ("words", json!({"words": 3})),
            ("count", json!({"words": "w", "count": 1.5})),
            ("sure", json!({"words": "w", "sure": "yes"})),
            ("one", json!({"words": "w", "one": "c"})),
            ("one", json!({"words": "w", "one": ["a"]})),
            ("many", json!({"words": "w", "many": ["a", "c"]})),
            ("many", json!({"words": "w", "many": "a"})),
            ("extra", json!({"words": "w", "extra": 1})),
        ] {
            assert_eq!(checked(&values), wrong, "{values}");
        }
    }
}
