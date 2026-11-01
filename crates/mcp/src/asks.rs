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

use self::secrets::{can_be_typed, collapsed, field_asks_for_secret, typed_asks_for_secret};
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
    /// The server that asked, so an answer goes to no other.
    pub(crate) server: String,
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
    /// It seemed to ask for a secret, or could not be read.
    Refused,
    /// It asked the person to open a page.
    Page,
    /// It was too much to show, or more forms in a round than the person is shown.
    TooMuch,
    /// It asked under a name that could pass as glave or the model.
    Name,
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
            Self::Page => format!(
                "[{server} asked the person to open a page, which glave does not do, so glave \
                 declined for them.]"
            ),
            Self::TooMuch => format!(
                "[{server} asked the person more than glave shows at once, so glave declined for \
                 them.]"
            ),
            Self::Name => format!(
                "[{server} asked the person something, but its name could pass as glave or the \
                 model, so glave declined for them. They can rename the service in their \
                 settings.]"
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
        server.clone_into(&mut self.server);
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
                    let made = if may_ask(server) {
                        form(&key, request.params)
                    } else {
                        Err(Note::Name)
                    };
                    let made = made.and_then(|form| {
                        (self.forms.len() < MOST_FORMS)
                            .then_some(form)
                            .ok_or(Note::TooMuch)
                    });
                    match made {
                        Ok(form) => self.forms.push_back(form),
                        Err(note) => self.decline(key, note).map_err(bad)?,
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
        } else if !self.notes.is_empty() {
            let text = self.told_with(server, String::new());
            output.parts.insert(0, Part::Text { text });
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

/// The form `params` ask for under `key`, its words clean, unless glave refuses it, saying why: a
/// page to open, which glave does not, a form it cannot show in full, or one that seems to ask
/// for a secret.
fn form(key: &str, params: ElicitRequestParams) -> Result<Form, Note> {
    let ElicitRequestParams::FormElicitationParams {
        message,
        requested_schema,
        ..
    } = params
    else {
        return Err(Note::Page);
    };
    let message = clean(&message);
    let fields = fields(&requested_schema)?;
    if message.is_empty() || message.chars().count() > LONGEST_MESSAGE || fields.len() > MOST_FIELDS
    {
        return Err(Note::TooMuch);
    }
    let typed = fields.iter().any(can_be_typed);
    if fields.iter().any(field_asks_for_secret) || (typed && typed_asks_for_secret(&message)) {
        return Err(Note::Refused);
    }
    Ok(Form {
        key: key.to_owned(),
        message,
        fields,
    })
}

/// `text` as a client may show it: on one line, without what could rewrite the terminal it is
/// shown in or hide letters from the eye, control and format characters and the line and
/// paragraph separators. Line ends, tabs among them, become a space.
pub(crate) fn clean(text: &str) -> String {
    let spaced = text.chars().map(|letter| match letter {
        '\n' | '\r' | '\t' | '\u{2028}' | '\u{2029}' => ' ',
        other => other,
    });
    let kept: String = spaced.filter(|letter| shown(*letter)).collect();
    kept.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Whether `letter` is one a person sees: not a control or format character.
fn shown(letter: char) -> bool {
    !letter.is_control()
        && !matches!(
            letter,
            '\u{ad}'
                | '\u{600}'..='\u{605}'
                | '\u{61c}'
                | '\u{6dd}'
                | '\u{70f}'
                | '\u{890}'..='\u{891}'
                | '\u{8e2}'
                | '\u{180e}'
                | '\u{200b}'..='\u{200f}'
                | '\u{202a}'..='\u{202e}'
                | '\u{2060}'..='\u{2064}'
                | '\u{2066}'..='\u{206f}'
                | '\u{feff}'
                | '\u{fff9}'..='\u{fffb}'
                | '\u{1d173}'..='\u{1d17a}'
                | '\u{e0001}'
                | '\u{e0020}'..='\u{e007f}'
        )
}

/// Names a service may not ask the person under, as it could pass as glave or the model: its
/// letters alone, in lowercase.
const NAMES_NOT_ITS_OWN: [&str; 9] = [
    "glave",
    "model",
    "themodel",
    "assistant",
    "system",
    "hope",
    "you",
    "yourassistant",
    "yourmodel",
];

/// Whether a service of this name may ask the person: one with nothing unseen in its name, which
/// could not pass as glave or the model.
fn may_ask(server: &str) -> bool {
    let plain = clean(server) == server && !server.is_empty();
    plain && !NAMES_NOT_ITS_OWN.contains(&collapsed(server).as_str())
}

/// `schema`'s fields, in the order the server listed them; what it asks that glave cannot show
/// when one is of a kind glave does not know, or too much to show.
fn fields(schema: &ElicitationSchema) -> Result<Vec<ServiceField>, Note> {
    let listed = schema.property_order.clone();
    let names = listed.unwrap_or_else(|| schema.properties.keys().cloned().collect());
    let required = schema.required.as_deref().unwrap_or_default();
    names
        .into_iter()
        .map(|name| {
            // Its name goes back as it is, so it can be shown only if nothing in it is unseen.
            if clean(&name) != name {
                return Err(Note::Refused);
            }
            let property = schema.properties.get(&name).ok_or(Note::Refused)?;
            let (title, description, kind) = described(property)?;
            let title = title.map(|title| clean(&title)).filter(|t| !t.is_empty());
            let label = title.unwrap_or_else(|| clean(&name));
            let description = description.map(|words| clean(&words));
            let description = description.filter(|words| !words.is_empty());
            let shown = [Some(&name), Some(&label), description.as_ref()];
            let fits = shown
                .iter()
                .flatten()
                .all(|words| words.chars().count() <= LONGEST_WORDS);
            if !fits || label.is_empty() {
                return Err(Note::TooMuch);
            }
            Ok(ServiceField {
                required: required.contains(&name),
                name,
                label,
                description,
                kind,
            })
        })
        .collect()
}

/// A property's title, description and kind, as far as it can be shown: a default that does not
/// fit it is not one, and one too long to show is too much.
fn described(
    property: &PrimitiveSchemaDefinition,
) -> Result<(Option<String>, Option<String>, ServiceFieldKind), Note> {
    let words = |word: &Option<std::borrow::Cow<'static, str>>| word.as_deref().map(str::to_owned);
    Ok(match property {
        PrimitiveSchemaDefinition::String(text) => {
            let default = text.default.as_deref().map(clean);
            let default = default.filter(|default| !default.is_empty());
            if default
                .as_ref()
                .is_some_and(|text| text.chars().count() > LONGEST_WORDS)
            {
                return Err(Note::TooMuch);
            }
            (
                words(&text.title),
                words(&text.description),
                ServiceFieldKind::Text { default },
            )
        }
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
        _ => return Err(Note::Refused),
    })
}

/// A choice's title, description and kind: its options' labels clean, and of its defaults only
/// those it offers.
fn chosen(choice: &EnumSchema) -> Result<(Option<String>, Option<String>, ServiceFieldKind), Note> {
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
    let (title, description, options, multiple, default): (_, _, _, _, Vec<String>) = match choice {
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
        _ => return Err(Note::Refused),
    };
    let fits = options.len() <= MOST_OPTIONS
        && options.iter().all(|option| {
            !option.label.is_empty()
                && option.value.chars().count() <= LONGEST_WORDS
                && option.label.chars().count() <= LONGEST_WORDS
        });
    if !fits {
        return Err(Note::TooMuch);
    }
    let default = default
        .into_iter()
        .filter(|value| options.iter().any(|option| option.value == *value))
        .collect();
    Ok((
        title,
        description,
        ServiceFieldKind::Choice {
            options,
            multiple,
            default,
        },
    ))
}

/// An option: its value goes back as the server sent it, its label as a client may show it.
fn option(value: &str, label: &str) -> ServiceOption {
    ServiceOption {
        value: value.to_owned(),
        label: clean(label),
    }
}

mod secrets;

#[cfg(test)]
mod tests;
