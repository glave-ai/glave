//! The model's questions in the terminal: each shown with numbered choices, answered with a
//! number, several numbers, or the person's own words. What a service asks reads the same, a
//! field at a time, each named as the service's.

use std::fmt::Write as _;

use glave_event::{ServiceAsked, ServiceField, ServiceFieldKind};
use glave_harness::Question;
use serde_json::{Number, Value};

use crate::terminal::{DIM, paint};

/// The question as the terminal shows it, ending where the answer is typed.
pub(crate) fn shown(color: bool, question: &Question) -> String {
    let mut text = format!(
        "\n\n{} {}\n",
        paint(color, "\x1b[33m", "question"),
        question.question
    );
    for (index, choice) in question.options.iter().enumerate() {
        let number = index.saturating_add(1);
        let _ = write!(text, "  {number}. {}", choice.label);
        if let Some(description) = &choice.description {
            let _ = write!(text, " {}", paint(color, DIM, &format!("— {description}")));
        }
        text.push('\n');
    }
    let how = match (question.options.is_empty(), question.multiple) {
        (true, _) => "your answer",
        (false, false) => "a number, or your own words",
        (false, true) => "numbers such as 1,3, or your own words",
    };
    format!("{text}  {} › ", paint(color, DIM, how))
}

/// The answer to `question` in what was typed: the chosen labels when it was choice numbers,
/// otherwise the words themselves.
pub(crate) fn answer(question: &Question, typed: &str) -> String {
    let typed = typed.trim();
    let numbers: Option<Vec<usize>> = typed
        .split([',', ' '])
        .filter(|part| !part.is_empty())
        .map(|part| part.parse::<usize>().ok())
        .collect();
    let labels: Option<Vec<&str>> =
        numbers
            .filter(|numbers| !numbers.is_empty())
            .and_then(|numbers| {
                numbers
                    .iter()
                    .map(|number| {
                        number
                            .checked_sub(1)
                            .and_then(|index| question.options.get(index))
                            .map(|choice| choice.label.as_str())
                    })
                    .collect()
            });
    match labels {
        Some(labels) if question.multiple || labels.len() == 1 => labels.join(", "),
        _ => typed.to_owned(),
    }
}

/// Who asks and what, as the terminal shows it: the server's line, then the service's words
/// marked as theirs, then where the answer goes.
pub(crate) fn service_shown(color: bool, asked: &ServiceAsked) -> String {
    format!(
        "\n\n{} {}\n{}: {}\n{} {}\n",
        paint(color, "\x1b[33m", "question"),
        asked.ask,
        asked.service,
        asked.message,
        paint(color, DIM, &asked.note),
        paint(
            color,
            DIM,
            "A required field left empty with nothing to use declines."
        ),
    )
}

/// One field of what a service asks, ending where its answer is typed.
pub(crate) fn field_shown(color: bool, field: &ServiceField) -> String {
    let mut text = format!("  {}", field.label);
    if let Some(description) = &field.description {
        let _ = write!(text, " {}", paint(color, DIM, &format!("— {description}")));
    }
    text.push('\n');
    let how = match &field.kind {
        ServiceFieldKind::Choice {
            options, multiple, ..
        } => {
            for (index, option) in options.iter().enumerate() {
                let number = index.saturating_add(1);
                let _ = writeln!(text, "    {number}. {}", option.label);
            }
            if *multiple {
                "numbers such as 1,3"
            } else {
                "a number"
            }
        }
        ServiceFieldKind::YesNo { .. } => "y or n",
        ServiceFieldKind::Number { .. } => "a number",
        ServiceFieldKind::Text { .. } => "your answer",
    };
    let how = match (default_shown(field), field.required) {
        (Some(default), _) => format!("{how}, or Enter for {default}"),
        (None, true) => how.to_owned(),
        (None, false) => format!("{how}, or nothing"),
    };
    format!("{text}  {} › ", paint(color, DIM, &how))
}

/// What a line typed for a field comes to.
#[derive(Debug, PartialEq)]
pub(crate) enum Field {
    Value(Value),
    /// Left out: an optional field, left empty with no default.
    Skipped,
    /// A required field left empty: the person declines all of it.
    Declined,
}

/// What `typed` answers for `field`: why it cannot be read, to ask again, when it does not fit.
pub(crate) fn field_answer(field: &ServiceField, typed: &str) -> Result<Field, String> {
    let typed = typed.trim();
    if typed.is_empty() {
        let none = if field.required {
            Field::Declined
        } else {
            Field::Skipped
        };
        return Ok(default_of(field).map_or(none, Field::Value));
    }
    let value = match &field.kind {
        ServiceFieldKind::Text { .. } => Some(Value::String(typed.to_owned())),
        ServiceFieldKind::Number { integer, .. } => typed
            .parse::<i64>()
            .ok()
            .map(Number::from)
            .or_else(|| {
                let parsed = typed.parse::<f64>().ok().filter(|_| !integer);
                parsed.and_then(Number::from_f64)
            })
            .map(Value::Number),
        ServiceFieldKind::YesNo { .. } => match typed.to_ascii_lowercase().as_str() {
            "y" | "yes" => Some(Value::Bool(true)),
            "n" | "no" => Some(Value::Bool(false)),
            _ => None,
        },
        ServiceFieldKind::Choice {
            options, multiple, ..
        } => {
            let picked: Option<Vec<Value>> = typed
                .split([',', ' '])
                .filter(|part| !part.is_empty())
                .map(|part| {
                    let index = part.parse::<usize>().ok()?.checked_sub(1)?;
                    let option = options.get(index)?;
                    Some(Value::String(option.value.clone()))
                })
                .collect();
            match (picked, multiple) {
                (Some(picked), true) => Some(Value::Array(picked)),
                (Some(picked), false) if picked.len() == 1 => picked.into_iter().next(),
                _ => None,
            }
        }
    };
    value
        .map(Field::Value)
        .ok_or_else(|| format!("That doesn't fit {}: try again.", field.label))
}

/// The default a field left empty stands for, as the person reads it.
fn default_shown(field: &ServiceField) -> Option<String> {
    match (&field.kind, default_of(field)?) {
        (ServiceFieldKind::YesNo { .. }, Value::Bool(yes)) => {
            Some(if yes { "yes" } else { "no" }.to_owned())
        }
        (
            ServiceFieldKind::Choice {
                options, default, ..
            },
            _,
        ) => {
            let labels: Vec<&str> = default
                .iter()
                .map(|value| {
                    let found = options.iter().find(|option| option.value == *value);
                    found.map_or(value.as_str(), |option| option.label.as_str())
                })
                .collect();
            Some(labels.join(", "))
        }
        (_, Value::String(text)) => Some(text),
        (_, other) => Some(other.to_string()),
    }
}

/// The answer a field left empty stands for, when it has one.
fn default_of(field: &ServiceField) -> Option<Value> {
    match &field.kind {
        ServiceFieldKind::Text { default } => default.clone().map(Value::String),
        ServiceFieldKind::Number { default, .. } => default.clone().map(Value::Number),
        ServiceFieldKind::YesNo { default } => default.map(Value::Bool),
        ServiceFieldKind::Choice {
            multiple, default, ..
        } => {
            let picked = default.iter().cloned().map(Value::String);
            if *multiple {
                (!default.is_empty()).then(|| Value::Array(picked.collect()))
            } else {
                picked.into_iter().next()
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use glave_event::ServiceOption;
    use glave_harness::Choice;
    use serde_json::json;

    fn question(multiple: bool) -> Question {
        let choice = |label: &str| Choice {
            label: label.to_owned(),
            description: None,
        };
        Question {
            question: "Which?".to_owned(),
            options: vec![choice("Postgres"), choice("SQLite"), choice("Both")],
            multiple,
        }
    }

    #[test]
    fn numbers_pick_choices_and_words_stand_as_they_are() {
        let one = question(false);
        assert_eq!(answer(&one, " 2 "), "SQLite");
        assert_eq!(answer(&one, "MySQL, please"), "MySQL, please");
        assert_eq!(answer(&one, "4"), "4", "no such choice: taken as words");
        assert_eq!(answer(&one, "1,2"), "1,2", "one choice only");
        assert_eq!(answer(&question(true), "1, 3"), "Postgres, Both");
        assert_eq!(answer(&one, ""), "");
    }

    #[test]
    fn choices_are_numbered() {
        let shown = shown(false, &question(true));
        assert!(shown.contains("question Which?\n  1. Postgres\n  2. SQLite\n  3. Both\n"));
        assert!(shown.ends_with("numbers such as 1,3, or your own words › "));
    }

    fn field(required: bool, kind: ServiceFieldKind) -> ServiceField {
        ServiceField {
            name: "f".to_owned(),
            label: "Label".to_owned(),
            description: None,
            required,
            kind,
        }
    }

    fn choice(multiple: bool, default: &[&str]) -> ServiceFieldKind {
        let option = |value: &str| ServiceOption {
            value: value.to_owned(),
            label: value.to_uppercase(),
        };
        ServiceFieldKind::Choice {
            options: vec![option("bug"), option("idea")],
            multiple,
            default: default.iter().map(|value| (*value).to_owned()).collect(),
        }
    }

    #[test]
    fn a_field_is_answered_as_its_kind_reads() {
        let value = |field: &ServiceField, typed: &str| field_answer(field, typed).unwrap();
        let text = field(true, ServiceFieldKind::Text { default: None });
        assert_eq!(value(&text, " hi "), Field::Value(json!("hi")));
        assert_eq!(
            value(&text, ""),
            Field::Declined,
            "required, so it declines"
        );
        let given = field(
            true,
            ServiceFieldKind::Text {
                default: Some("hi".to_owned()),
            },
        );
        assert_eq!(
            value(&given, ""),
            Field::Value(json!("hi")),
            "as the app does"
        );
        let number = |integer| {
            field(
                false,
                ServiceFieldKind::Number {
                    integer,
                    default: None,
                },
            )
        };
        assert_eq!(value(&number(true), "3"), Field::Value(json!(3)));
        assert_eq!(value(&number(false), "1.5"), Field::Value(json!(1.5)));
        assert!(field_answer(&number(true), "1.5").is_err());
        assert_eq!(value(&number(true), ""), Field::Skipped);
        let yes_no = field(
            false,
            ServiceFieldKind::YesNo {
                default: Some(true),
            },
        );
        assert_eq!(value(&yes_no, "n"), Field::Value(json!(false)));
        assert_eq!(value(&yes_no, ""), Field::Value(json!(true)), "its default");
        assert!(field_answer(&yes_no, "maybe").is_err());
        let one = field(true, choice(false, &[]));
        assert_eq!(value(&one, "2"), Field::Value(json!("idea")));
        assert!(field_answer(&one, "3").is_err());
        assert!(field_answer(&one, "1,2").is_err());
        let many = field(false, choice(true, &["bug"]));
        assert_eq!(value(&many, "2 1"), Field::Value(json!(["idea", "bug"])));
        assert_eq!(
            value(&many, ""),
            Field::Value(json!(["bug"])),
            "its default"
        );
    }

    #[test]
    fn a_field_shows_how_it_is_answered() {
        let shown = field_shown(false, &field(false, choice(false, &[])));
        assert!(
            shown.starts_with("  Label\n    1. BUG\n    2. IDEA\n"),
            "{shown}"
        );
        assert!(shown.ends_with("a number, or nothing › "), "{shown}");
        let yes = field(
            false,
            ServiceFieldKind::YesNo {
                default: Some(true),
            },
        );
        assert!(field_shown(false, &yes).ends_with("y or n, or Enter for yes › "));
        let picked = field_shown(false, &field(true, choice(false, &["idea"])));
        assert!(
            picked.ends_with("a number, or Enter for IDEA › "),
            "{picked}"
        );
    }
}
