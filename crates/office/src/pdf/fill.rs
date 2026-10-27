//! `glave doc fill`: a PDF's form filled from its fields' values in JSON. Every name and value
//! is checked against the field it names before anything changes, so a wrong one writes
//! nothing and says what is wrong.

use std::fmt::Write as _;

use lopdf::{Document, Object, ObjectId, text_string};
use serde_json::{Map, Value};

use super::appearance::draw;
use super::form::{COMBO, EDIT, Field, Kind, MULTILINE, READ_ONLY, fields, form_mut, on_state};
use super::{Given, Use, counted, open, saved};
use crate::cells::listed;
use crate::error::OfficeError;
use crate::write::Written;

/// The field names said when a name given isn't one of them.
const NAMED: usize = 30;

/// What a field is set to.
#[derive(Debug, PartialEq, Eq)]
enum Setting {
    /// The text of a text field, or the value chosen in a list or a drop-down.
    Text(String),
    /// The state a checkbox or radio buttons are in: one of theirs, or `Off`.
    State(Vec<u8>),
}

/// `given` with its form's fields set to the values `json` gives them by name.
pub(crate) fn fill(given: &Given, json: &str) -> Result<Written, OfficeError> {
    let mut document = open(given, Use::Change)?;
    let values: Map<String, Value> = serde_json::from_str(json).map_err(|error| {
        OfficeError::Values(format!(
            "the values are not a JSON object of field names and values, like \
             {{\"Name\": \"Sam Okafor\", \"Paid\": true}}: {error}"
        ))
    })?;
    if values.is_empty() {
        return Err(OfficeError::Values(
            "the JSON names no fields: give each field's name and value, like \
             {\"Name\": \"Sam Okafor\"}"
                .to_owned(),
        ));
    }
    let found = fields(&document);
    if found.is_empty() {
        return Err(OfficeError::Pdf(format!(
            "{} has no form fields to fill",
            given.name
        )));
    }
    let mut settings = Vec::new();
    let (mut unknown, mut wrong) = (Vec::new(), Vec::new());
    for (name, value) in &values {
        let named: Vec<&Field> = found.iter().filter(|field| field.name == *name).collect();
        if named.is_empty() {
            unknown.push(name.as_str());
        }
        for field in named {
            match setting(field, value) {
                Ok(setting) => settings.push((field, setting)),
                Err(why) => wrong.push(why),
            }
        }
    }
    if !unknown.is_empty() {
        wrong.insert(0, not_found(&unknown, &found));
    }
    if !wrong.is_empty() {
        return Err(OfficeError::Values(format!(
            "{}. glave doc fields {} says what each field takes",
            wrong.join("; "),
            given.name
        )));
    }
    let mut font = None;
    for (field, setting) in &settings {
        set(&mut document, field, setting, &mut font);
    }
    let mut report = format!(
        "{} filled: {}",
        counted(values.len(), "field", "fields"),
        values.keys().cloned().collect::<Vec<_>>().join(", ")
    );
    if let Some(form) = form_mut(&mut document) {
        form.set("NeedAppearances", true);
        if form.remove(b"XFA").is_some() {
            report.push_str(
                ". The form's XFA copy was taken out, so viewers show these values, not its own",
            );
        }
    }
    Ok(Written {
        bytes: saved(document)?,
        report,
    })
}

/// What `value` sets `field` to, or why it can't.
fn setting(field: &Field, value: &Value) -> Result<Setting, String> {
    let name = &field.name;
    if field.flags & READ_ONLY != 0 {
        return Err(format!("{name:?} is read-only"));
    }
    let one_of = || {
        let labels: Vec<String> = field
            .choices
            .iter()
            .map(|c| format!("{:?}", c.label))
            .collect();
        format!("{name:?} takes one of {}", labels.join(", "))
    };
    let chosen = |given: &str| {
        field
            .choices
            .iter()
            .find(|c| c.label == given || c.stored == given.as_bytes())
            .map(|c| c.stored.clone())
    };
    match (field.kind, value) {
        (Kind::Text | Kind::Choice, Value::Null) => Ok(Setting::Text(String::new())),
        (Kind::Text, Value::String(_) | Value::Number(_)) => {
            let text = written(value);
            match field.max_len {
                Some(most) if text.chars().count() > most => Err(format!(
                    "{name:?} takes at most {most} characters, and {text:?} has {}",
                    text.chars().count()
                )),
                _ => Ok(Setting::Text(text)),
            }
        }
        (Kind::Text, _) => Err(format!("{name:?} takes text")),
        (Kind::Choice, Value::String(_) | Value::Number(_)) => {
            let text = written(value);
            match chosen(&text) {
                Some(stored) => Ok(Setting::Text(String::from_utf8_lossy(&stored).into_owned())),
                None if field.flags & EDIT != 0 => Ok(Setting::Text(text)),
                None => Err(one_of()),
            }
        }
        (Kind::Checkbox, Value::Bool(true)) => Ok(Setting::State(
            field
                .choices
                .first()
                .map_or_else(|| b"Yes".to_vec(), |c| c.stored.clone()),
        )),
        (Kind::Checkbox | Kind::Radio, Value::Bool(false) | Value::Null) => {
            Ok(Setting::State(b"Off".to_vec()))
        }
        (Kind::Checkbox, Value::String(text)) => chosen(text)
            .or_else(|| (text == "Off").then(|| b"Off".to_vec()))
            .map(Setting::State)
            .ok_or_else(|| format!("{name:?} is a checkbox: true ticks it and false clears it")),
        (Kind::Checkbox, _) => Err(format!(
            "{name:?} is a checkbox: true ticks it and false clears it"
        )),
        (Kind::Radio, Value::String(text)) => chosen(text).map(Setting::State).ok_or_else(one_of),
        (Kind::Radio | Kind::Choice, _) => Err(one_of()),
        (Kind::Signature, _) => Err(format!("{name:?} is a signature, which glave can't fill")),
        (Kind::Button | Kind::Other, _) => Err(format!("{name:?} is not a field glave can fill")),
    }
}

/// A JSON string or number as text.
fn written(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        other => other.to_string(),
    }
}

/// Why names given aren't fields: each `unknown` one, a field it may have meant, and the
/// fields `found` there are.
fn not_found(unknown: &[&str], found: &[Field]) -> String {
    let names: Vec<String> = found
        .iter()
        .map(|field| format!("{:?}", field.name))
        .collect();
    let mut said = Vec::new();
    for name in unknown {
        let near = found
            .iter()
            .find(|field| field.name.trim().to_lowercase() == name.trim().to_lowercase());
        said.push(match near {
            Some(field) => format!("{name:?} (did you mean {:?}?)", field.name),
            None => format!("{name:?}"),
        });
    }
    let mut sentence = format!("no field is named {}", said.join(" or "));
    let shown: Vec<String> = names.iter().take(NAMED).cloned().collect();
    let _ = write!(sentence, "; the fields are {}", listed(&shown, names.len()));
    sentence
}

/// `field` set as `setting` says: its value, and how each of its widgets shows it.
fn set(document: &mut Document, field: &Field, setting: &Setting, font: &mut Option<ObjectId>) {
    let value = match setting {
        Setting::Text(text) => text_string(text),
        Setting::State(state) => Object::Name(state.clone()),
    };
    if let Ok(dictionary) = document.get_dictionary_mut(field.id) {
        dictionary.set("V", value);
        // The places a list's chosen values are at, which would no longer match.
        dictionary.remove(b"I");
    }
    for &widget in &field.widgets {
        match setting {
            Setting::Text(text) => {
                if field.kind == Kind::Text || field.flags & COMBO != 0 {
                    let several = field.flags & MULTILINE != 0;
                    draw(document, widget, field.look.as_deref(), text, several, font);
                }
            }
            Setting::State(state) => {
                let shows = on_state(document, widget).is_some_and(|on| on == *state);
                if let Ok(widget) = document.get_dictionary_mut(widget) {
                    let state = if shows {
                        state.clone()
                    } else {
                        b"Off".to_vec()
                    };
                    widget.set("AS", Object::Name(state));
                }
            }
        }
    }
}
