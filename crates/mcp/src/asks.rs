//! What a server asks the person inside a call (MCP's elicitation, in MCP 2026-07-28's input
//! rounds), from what it sent to what goes back. One form reaches the person at a time, however
//! many a round holds; between them nothing is held but [`Asking`], which the harness keeps
//! with the call's wait. What glave must never ask a person for is declined here, unseen.

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use glave_event::{Part, ServiceAnswer, ServiceField, ServiceFieldKind, ServiceOption};
use glave_ports::ToolOutput;
use rmcp::model::{
    ElicitRequestParams, ElicitResult, ElicitationAction, ElicitationSchema, EnumSchema,
    InputRequest, InputRequests, InputRequiredResult, InputResponses, MultiSelectEnumSchema,
    PrimitiveSchemaDefinition, SingleSelectEnumSchema,
};
use serde::{Deserialize, Serialize};
use serde_json::{Number, Value};

use crate::call::Unanswered;
use crate::client::Client;

/// The most forms of one round the person is shown; any more are declined for them.
const MOST_FORMS: usize = 5;
/// The most fields one form may ask for.
const MOST_FIELDS: usize = 20;
/// The most options one field may offer.
const MOST_OPTIONS: usize = 100;
/// The longest a form's message may be, in characters.
const LONGEST_MESSAGE: usize = 2_000;
/// The longest a field's name, title, description or option may be, in characters.
const LONGEST_WORDS: usize = 500;

/// What the call has come to across its rounds: what the harness keeps while the person answers.
#[derive(Debug, Default, Serialize, Deserialize)]
pub(crate) struct Asking {
    /// How many times the server has been called.
    pub(crate) rounds: usize,
    /// The state it asked to have back, echoed as it was.
    pub(crate) state: Option<String>,
    /// What goes back with the next call.
    pub(crate) responses: InputResponses,
    /// What its last round still asks the person, in order.
    pub(crate) forms: VecDeque<Form>,
    /// What to tell the model of what was asked.
    pub(crate) notes: BTreeSet<Note>,
}

/// One form for the person.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct Form {
    /// The key its answer goes back under.
    key: String,
    pub(crate) message: String,
    pub(crate) fields: Vec<ServiceField>,
}

/// What the model is told came of what the server asked, never what was answered.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub(crate) enum Note {
    Answered,
    Declined,
    NoOneThere,
    Refused,
}

impl Note {
    fn said(self, server: &str) -> String {
        match self {
            Self::Answered => format!(
                "[{server} asked the person something directly, and they answered it. You do not \
                 see what they answered, and need not ask.]"
            ),
            Self::Declined => {
                format!("[{server} asked the person something directly, and they declined.]")
            }
            Self::NoOneThere => format!(
                "[{server} asked the person something, and no one was there to answer, so it was \
                 declined.]"
            ),
            Self::Refused => format!(
                "[{server} asked for something glave does not ask people for, such as a password \
                 or a key, so glave declined for them. Do not ask them for it yourself.]"
            ),
        }
    }
}

impl Asking {
    /// Takes in `answer`, the person's to the first form.
    pub(crate) fn given(&mut self, answer: ServiceAnswer) -> Result<(), serde_json::Error> {
        let Some(form) = self.forms.pop_front() else {
            return Ok(());
        };
        let result = match answer {
            ServiceAnswer::Answered { values } => {
                self.notes.insert(Note::Answered);
                ElicitResult::new(ElicitationAction::Accept).with_content(Value::Object(values))
            }
            ServiceAnswer::Declined => {
                self.notes.insert(Note::Declined);
                ElicitResult::new(ElicitationAction::Decline)
            }
        };
        self.responses
            .insert(form.key, serde_json::to_value(result)?);
        Ok(())
    }

    /// Declines every form left, with no one to ask.
    pub(crate) fn no_one_there(&mut self) -> Result<(), serde_json::Error> {
        while let Some(form) = self.forms.pop_front() {
            self.decline(form.key, Note::NoOneThere)?;
        }
        Ok(())
    }

    /// Declines what `key` asked, for the person, saying so as `note`.
    fn decline(&mut self, key: String, note: Note) -> Result<(), serde_json::Error> {
        self.notes.insert(note);
        let declined = ElicitResult::new(ElicitationAction::Decline);
        self.responses.insert(key, serde_json::to_value(declined)?);
        Ok(())
    }

    /// Takes in what `server` asked in its last round: what glave answers itself goes back with
    /// the next call, and what is the person's waits in `forms`.
    pub(crate) fn asked(
        &mut self,
        server: &str,
        client: &Client,
        asked: InputRound,
    ) -> Result<(), Unanswered> {
        let bad = |err: serde_json::Error| Unanswered::Failed(format!("{server}: {err}"));
        self.state = asked.state;
        for (key, request) in asked.requests {
            let Asked::Readable(request) = request else {
                self.decline(key, Note::Refused).map_err(bad)?;
                continue;
            };
            match *request {
                InputRequest::ListRoots(_) => {
                    let roots = serde_json::to_value(client.roots()).map_err(bad)?;
                    self.responses.insert(key, roots);
                }
                InputRequest::Elicitation(request) => {
                    let form = form(&key, request.params);
                    match form.filter(|_| self.forms.len() < MOST_FORMS) {
                        Some(form) => self.forms.push_back(form),
                        None => self.decline(key, Note::Refused).map_err(bad)?,
                    }
                }
                _ => {
                    return Err(Unanswered::Failed(format!(
                        "{server} asked for more input, which glave cannot give it yet."
                    )));
                }
            }
        }
        Ok(())
    }

    /// `output`, with what the model is told of what was asked before it.
    pub(crate) fn told(&self, server: &str, mut output: ToolOutput) -> ToolOutput {
        if let Some(Part::Text { text }) = output.parts.first_mut() {
            *text = self.told_with(server, std::mem::take(text));
        }
        output
    }

    /// `why`, an error in the server's words, with what the model is told of what was asked.
    pub(crate) fn told_with(&self, server: &str, why: String) -> String {
        let mut lines: Vec<String> = self.notes.iter().map(|note| note.said(server)).collect();
        lines.push(why);
        lines.join("\n")
    }
}

/// A server's input round, as far as it could be read.
pub(crate) struct InputRound {
    requests: BTreeMap<String, Asked>,
    state: Option<String>,
}

/// One request of a round.
enum Asked {
    Readable(Box<InputRequest>),
    /// A form glave cannot read, as one with a field whose format MCP does not know
    /// (`"format": "password"`), which could be asking for anything.
    Unreadable,
}

impl From<InputRequiredResult> for InputRound {
    fn from(asked: InputRequiredResult) -> Self {
        let requests: InputRequests = asked.input_requests.unwrap_or_default();
        let requests = requests.into_iter();
        Self {
            requests: requests
                .map(|(key, request)| (key, Asked::Readable(Box::new(request))))
                .collect(),
            state: asked.request_state,
        }
    }
}

impl InputRound {
    /// The round in `result`, a result rmcp could not read as any it knows, when it is one: a
    /// form it cannot read makes the whole result so. Each request is read on its own, so only
    /// that form is refused. `None` when it is not a round, or holds what is not a form.
    pub(crate) fn unread(result: &Value) -> Option<Self> {
        if result.get("resultType")?.as_str()? != "input_required" {
            return None;
        }
        let state = result.get("requestState").and_then(Value::as_str);
        let listed = result.get("inputRequests").and_then(Value::as_object);
        let mut requests = BTreeMap::new();
        for (key, request) in listed.into_iter().flatten() {
            let asked = match serde_json::from_value(request.clone()) {
                Ok(readable) => Asked::Readable(Box::new(readable)),
                Err(_) if request.get("method")?.as_str()? == "elicitation/create" => {
                    Asked::Unreadable
                }
                Err(_) => return None,
            };
            requests.insert(key.clone(), asked);
        }
        Some(Self {
            requests,
            state: state.map(str::to_owned),
        })
    }
}

/// The form `params` ask for under `key`, unless glave refuses it: a page to open, which glave
/// does not, a form it cannot show in full, or one that seems to ask for a secret.
fn form(key: &str, params: ElicitRequestParams) -> Option<Form> {
    let ElicitRequestParams::FormElicitationParams {
        message,
        requested_schema,
        ..
    } = params
    else {
        return None;
    };
    let fields = fields(&requested_schema)?;
    let fits = message.chars().count() <= LONGEST_MESSAGE && fields.len() <= MOST_FIELDS;
    let typed = fields.iter().any(|field| {
        matches!(
            field.kind,
            ServiceFieldKind::Text { .. } | ServiceFieldKind::Number { .. }
        )
    });
    let secret = fields.iter().any(field_asks_for_secret) || (typed && asks_for_secret(&message));
    (fits && !secret).then(|| Form {
        key: key.to_owned(),
        message,
        fields,
    })
}

/// `schema`'s fields, in the order the server listed them; `None` when one is of a kind glave
/// does not know, or too much to show.
fn fields(schema: &ElicitationSchema) -> Option<Vec<ServiceField>> {
    let listed = schema.property_order.clone();
    let names = listed.unwrap_or_else(|| schema.properties.keys().cloned().collect());
    let required = schema.required.as_deref().unwrap_or_default();
    names
        .into_iter()
        .map(|name| {
            let property = schema.properties.get(&name)?;
            let (title, description, kind) = described(property)?;
            let label = title.unwrap_or_else(|| name.clone());
            let shown = [Some(&name), Some(&label), description.as_ref()];
            let fits = shown
                .iter()
                .flatten()
                .all(|words| words.chars().count() <= LONGEST_WORDS);
            fits.then(|| ServiceField {
                required: required.contains(&name),
                name,
                label,
                description,
                kind,
            })
        })
        .collect()
}

/// A property's title, description and kind.
fn described(
    property: &PrimitiveSchemaDefinition,
) -> Option<(Option<String>, Option<String>, ServiceFieldKind)> {
    let words = |word: &Option<std::borrow::Cow<'static, str>>| word.as_deref().map(str::to_owned);
    Some(match property {
        PrimitiveSchemaDefinition::String(text) => (
            words(&text.title),
            words(&text.description),
            ServiceFieldKind::Text {
                default: text.default.clone(),
            },
        ),
        PrimitiveSchemaDefinition::Number(number) => (
            words(&number.title),
            words(&number.description),
            ServiceFieldKind::Number {
                integer: false,
                default: number.default.and_then(Number::from_f64),
            },
        ),
        PrimitiveSchemaDefinition::Integer(number) => (
            words(&number.title),
            words(&number.description),
            ServiceFieldKind::Number {
                integer: true,
                default: number.default.map(Number::from),
            },
        ),
        PrimitiveSchemaDefinition::Boolean(boolean) => (
            words(&boolean.title),
            words(&boolean.description),
            ServiceFieldKind::YesNo {
                default: boolean.default,
            },
        ),
        PrimitiveSchemaDefinition::Enum(choice) => return chosen(choice),
        _ => return None,
    })
}

/// A choice's title, description and kind.
fn chosen(choice: &EnumSchema) -> Option<(Option<String>, Option<String>, ServiceFieldKind)> {
    let words = |word: &Option<std::borrow::Cow<'static, str>>| word.as_deref().map(str::to_owned);
    let plain = |values: &[String]| -> Vec<ServiceOption> {
        values.iter().map(|value| option(value, value)).collect()
    };
    let titled = |pairs: &[rmcp::model::ConstTitle]| -> Vec<ServiceOption> {
        pairs
            .iter()
            .map(|pair| option(&pair.const_, &pair.title))
            .collect()
    };
    let (title, description, options, multiple, default) = match choice {
        EnumSchema::Single(SingleSelectEnumSchema::Untitled(one)) => (
            words(&one.title),
            words(&one.description),
            plain(&one.enum_),
            false,
            one.default.clone().into_iter().collect(),
        ),
        EnumSchema::Single(SingleSelectEnumSchema::Titled(one)) => (
            words(&one.title),
            words(&one.description),
            titled(&one.one_of),
            false,
            one.default.clone().into_iter().collect(),
        ),
        EnumSchema::Multi(MultiSelectEnumSchema::Untitled(many)) => (
            words(&many.title),
            words(&many.description),
            plain(&many.items.enum_),
            true,
            many.default.clone().unwrap_or_default(),
        ),
        EnumSchema::Multi(MultiSelectEnumSchema::Titled(many)) => (
            words(&many.title),
            words(&many.description),
            titled(&many.items.any_of),
            true,
            many.default.clone().unwrap_or_default(),
        ),
        EnumSchema::Legacy(old) => {
            let names = old.enum_names.as_deref().unwrap_or_default();
            let options = old.enum_.iter().enumerate().map(|(at, value)| {
                option(value, names.get(at).map_or(value.as_str(), String::as_str))
            });
            (
                words(&old.title),
                words(&old.description),
                options.collect(),
                false,
                old.default.clone().into_iter().collect(),
            )
        }
        _ => return None,
    };
    let fits = options.len() <= MOST_OPTIONS
        && options.iter().all(|option| {
            option.value.chars().count() <= LONGEST_WORDS
                && option.label.chars().count() <= LONGEST_WORDS
        });
    fits.then_some((
        title,
        description,
        ServiceFieldKind::Choice {
            options,
            multiple,
            default,
        },
    ))
}

fn option(value: &str, label: &str) -> ServiceOption {
    ServiceOption {
        value: value.to_owned(),
        label: label.to_owned(),
    }
}

/// Whether `field`'s name, title or description seems to ask for a secret.
fn field_asks_for_secret(field: &ServiceField) -> bool {
    words(&field.name) == ["key"]
        || asks_for_secret(&field.name)
        || asks_for_secret(&field.label)
        || field.description.as_deref().is_some_and(asks_for_secret)
}

/// Words that ask for a secret wherever they appear.
const SECRET_WORDS: [&str; 15] = [
    "password",
    "passwords",
    "passwd",
    "pwd",
    "passphrase",
    "passcode",
    "secret",
    "secrets",
    "token",
    "tokens",
    "credential",
    "credentials",
    "apikey",
    "otp",
    "mfa",
];

/// Words that make the next `key` a secret, where "key" alone may be anything ("issue key").
const KEY_KINDS: [&str; 12] = [
    "api",
    "access",
    "auth",
    "client",
    "deploy",
    "encryption",
    "gpg",
    "license",
    "pgp",
    "private",
    "signing",
    "ssh",
];

/// Whether `text` seems to ask for a password, key, token, secret or credential: a person's way
/// in to something, which a form is never the place for, whoever asks. A service that hides what
/// it asks for gets past this; a service that says so does not.
fn asks_for_secret(text: &str) -> bool {
    let words = words(text);
    words
        .iter()
        .any(|word| SECRET_WORDS.contains(&word.as_str()))
        || words.windows(2).any(|pair| match pair {
            [kind, key] => {
                KEY_KINDS.contains(&kind.as_str()) && matches!(key.as_str(), "key" | "keys")
            }
            _ => false,
        })
}

/// `text`'s words in lowercase, split at anything not a letter or a digit and where a lowercase
/// letter meets a capital (`apiKey`, `client_secret`).
fn words(text: &str) -> Vec<String> {
    let mut words = vec![String::new()];
    let mut lower = false;
    for letter in text.chars() {
        if !letter.is_alphanumeric() {
            words.push(String::new());
            lower = false;
            continue;
        }
        if lower && letter.is_uppercase() {
            words.push(String::new());
        }
        lower = letter.is_lowercase();
        if let Some(word) = words.last_mut() {
            word.extend(letter.to_lowercase());
        }
    }
    words.retain(|word| !word.is_empty());
    words
}

#[cfg(test)]
mod tests;
