//! `glave doc fields`: a PDF form's fields, a line each, with its name, kind, page and value.

use std::collections::BTreeMap;
use std::fmt::Write as _;

use lopdf::{Document, Object, ObjectId};

use super::form::{
    Choice, EDIT, Field, Kind, MULTILINE, READ_ONLY, REQUIRED, fields, form_of, references, text,
};
use super::{Given, Use, counted, open};
use crate::error::OfficeError;

/// The longest value shown whole.
const SHOWN: usize = 200;

/// `glave doc fields`: each field of `given`'s form, with its kind, page and value.
pub(crate) fn listed(given: &Given) -> Result<String, OfficeError> {
    let document = open(given, Use::Read)?;
    let name = &given.name;
    let found = fields(&document);
    let pages = widget_pages(&document);
    let shown: Vec<&Field> = found.iter().filter(|f| f.kind != Kind::Button).collect();
    if shown.is_empty() {
        let xfa = form_of(&document).is_some_and(|form| form.has(b"XFA"));
        return Ok(if xfa {
            format!("{name} is an XFA form, which glave can't fill: it has no PDF form fields")
        } else {
            format!("{name} has no form fields to fill")
        });
    }
    let mut text = format!(
        "{name}: {}. glave doc fill takes them by these names:\n",
        counted(shown.len(), "field", "fields")
    );
    for field in shown {
        let _ = writeln!(text, "{}", line(field, &pages));
    }
    Ok(text)
}

/// A field in a line: `"Name" (text, page 1): empty. Described as "Your full name"`.
fn line(field: &Field, pages: &BTreeMap<ObjectId, usize>) -> String {
    let quoted = |choices: &[Choice]| {
        let labels: Vec<String> = choices.iter().map(|c| format!("{:?}", c.label)).collect();
        labels.join(", ")
    };
    let mut what = match field.kind {
        Kind::Text if field.flags & MULTILINE != 0 => "text of several lines".to_owned(),
        Kind::Text => "text".to_owned(),
        Kind::Checkbox => "checkbox".to_owned(),
        Kind::Choice if field.flags & EDIT != 0 => {
            format!("one of {}, or other text", quoted(&field.choices))
        }
        Kind::Radio | Kind::Choice => format!("one of {}", quoted(&field.choices)),
        Kind::Signature => "a signature, which glave can't fill".to_owned(),
        Kind::Button | Kind::Other => "a field glave can't fill".to_owned(),
    };
    if let Some(most) = field.max_len {
        let _ = write!(what, ", at most {most} characters");
    }
    for (flag, said) in [(READ_ONLY, "read-only"), (REQUIRED, "required")] {
        if field.flags & flag != 0 {
            let _ = write!(what, ", {said}");
        }
    }
    let mut on: Vec<usize> = field
        .widgets
        .iter()
        .filter_map(|w| pages.get(w))
        .copied()
        .collect();
    on.sort_unstable();
    on.dedup();
    let on: Vec<String> = on.iter().map(ToString::to_string).collect();
    match on.as_slice() {
        [] => {}
        [one] => {
            let _ = write!(what, ", page {one}");
        }
        many => {
            let _ = write!(what, ", pages {}", many.join(", "));
        }
    }
    let mut line = format!("{:?} ({what}): {}", field.name, shown(field));
    if let Some(tooltip) = field.tooltip.as_ref().filter(|tip| **tip != field.name) {
        let _ = write!(line, ". Described as {tooltip:?}");
    }
    line
}

/// A field's value as `fields` shows it.
fn shown(field: &Field) -> String {
    let value = field.value.as_ref();
    match field.kind {
        Kind::Checkbox => match state(value) {
            Some(_) => "ticked".to_owned(),
            None => "not ticked".to_owned(),
        },
        Kind::Radio => match state(value) {
            Some(on) => field
                .choices
                .iter()
                .find(|choice| choice.stored == on)
                .map_or_else(
                    || format!("{:?}", String::from_utf8_lossy(on)),
                    |c| format!("{:?}", c.label),
                ),
            None => "none chosen".to_owned(),
        },
        _ => match value.map(text).filter(|text| !text.is_empty()) {
            Some(text) if text.chars().count() > SHOWN => {
                let start: String = text.chars().take(SHOWN).collect();
                format!("{start:?} and more")
            }
            Some(text) => format!("{text:?}"),
            None => "empty".to_owned(),
        },
    }
}

/// The state a button's value names, unless it is off.
fn state(value: Option<&Object>) -> Option<&[u8]> {
    value
        .and_then(|value| value.as_name().ok())
        .filter(|name| *name != b"Off")
}

/// The page, numbered from 1, that each widget on a page is on.
fn widget_pages(document: &Document) -> BTreeMap<ObjectId, usize> {
    let mut pages = BTreeMap::new();
    for (number, page) in document.page_iter().enumerate() {
        let annotations = document
            .get_dictionary(page)
            .ok()
            .and_then(|page| page.get(b"Annots").ok());
        for widget in references(document, annotations) {
            pages.entry(widget).or_insert(number + 1);
        }
    }
    pages
}
