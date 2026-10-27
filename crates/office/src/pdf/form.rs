//! A PDF's form: its fields, found from the catalog's `AcroForm`, each with its full name,
//! kind, value and widgets (the boxes on the pages that show it), and the fields on pages
//! taken out of it.

use std::collections::BTreeSet;

use lopdf::{Dictionary, Document, Object, ObjectId, decode_text_string};

use super::{DEEPEST, resolved};

/// The flags (`Ff`) glave reads, by their bits.
pub(super) const READ_ONLY: i64 = 1;
pub(super) const REQUIRED: i64 = 1 << 1;
pub(super) const MULTILINE: i64 = 1 << 12;
const RADIO: i64 = 1 << 15;
const PUSH_BUTTON: i64 = 1 << 16;
pub(super) const COMBO: i64 = 1 << 17;
pub(super) const EDIT: i64 = 1 << 18;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Kind {
    Text,
    Checkbox,
    Radio,
    /// A list or a drop-down to choose from.
    Choice,
    /// A push button, which holds no value.
    Button,
    Signature,
    Other,
}

/// A value a field takes: what a person sees, and what the file stores for it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Choice {
    pub(super) label: String,
    /// A checkbox's or a radio button's state name, or a choice's export value.
    pub(super) stored: Vec<u8>,
}

/// A field that holds a value, with what it took from the fields above it.
#[derive(Debug)]
pub(super) struct Field {
    /// Its full name: its parents' and its own, joined by dots.
    pub(super) name: String,
    pub(super) id: ObjectId,
    pub(super) kind: Kind,
    pub(super) flags: i64,
    pub(super) widgets: Vec<ObjectId>,
    /// What the form says the field is for (`TU`), when it says.
    pub(super) tooltip: Option<String>,
    pub(super) max_len: Option<usize>,
    pub(super) choices: Vec<Choice>,
    pub(super) value: Option<Object>,
    /// Its default look (`DA`): the font, size and colour of its text.
    pub(super) look: Option<Vec<u8>>,
}

/// What a field takes from the fields above it.
#[derive(Clone, Default)]
struct Above {
    name: String,
    kind: Option<Vec<u8>>,
    flags: Option<i64>,
    value: Option<Object>,
    look: Option<Object>,
    options: Option<Object>,
    max_len: Option<i64>,
}

/// Every field in `document`'s form that holds a value, in the form's order.
pub(super) fn fields(document: &Document) -> Vec<Field> {
    let Some(form) = form_of(document) else {
        return Vec::new();
    };
    let above = Above {
        look: form.get(b"DA").ok().cloned(),
        ..Above::default()
    };
    let (mut found, mut seen) = (Vec::new(), BTreeSet::new());
    for id in references(document, form.get(b"Fields").ok()) {
        walk(document, id, &above, 0, &mut seen, &mut found);
    }
    found
}

/// The field `id` and those under it, added to `found`.
fn walk(
    document: &Document,
    id: ObjectId,
    above: &Above,
    depth: usize,
    seen: &mut BTreeSet<ObjectId>,
    found: &mut Vec<Field>,
) {
    if depth > DEEPEST || !seen.insert(id) {
        return;
    }
    let Ok(dictionary) = document.get_dictionary(id) else {
        return;
    };
    let mut here = above.clone();
    if let Ok(part) = dictionary.get(b"T") {
        let part = text(part);
        here.name = if above.name.is_empty() {
            part
        } else {
            format!("{}.{part}", above.name)
        };
    }
    let own = |key: &[u8]| {
        dictionary
            .get(key)
            .ok()
            .and_then(|o| resolved(document, o))
            .cloned()
    };
    here.kind = own(b"FT")
        .and_then(|o| o.as_name().ok().map(<[u8]>::to_vec))
        .or(here.kind);
    here.flags = own(b"Ff").and_then(|o| o.as_i64().ok()).or(here.flags);
    here.max_len = own(b"MaxLen")
        .and_then(|o| o.as_i64().ok())
        .or(here.max_len);
    here.value = own(b"V").or(here.value);
    here.look = own(b"DA").or(here.look);
    here.options = own(b"Opt").or(here.options);
    let kids = references(document, dictionary.get(b"Kids").ok());
    let named = |kid: &ObjectId| document.get_dictionary(*kid).is_ok_and(|d| d.has(b"T"));
    let (subfields, widgets): (Vec<ObjectId>, Vec<ObjectId>) =
        kids.iter().copied().partition(named);
    if !subfields.is_empty() {
        for kid in subfields {
            walk(document, kid, &here, depth + 1, seen, found);
        }
        return;
    }
    if here.name.is_empty() {
        return;
    }
    let widgets = if kids.is_empty() { vec![id] } else { widgets };
    let tooltip = dictionary.get(b"TU").ok().map(text);
    found.push(field(document, id, here, widgets, tooltip));
}

/// The field `id`, from what it has and took from above.
fn field(
    document: &Document,
    id: ObjectId,
    here: Above,
    widgets: Vec<ObjectId>,
    tooltip: Option<String>,
) -> Field {
    let flags = here.flags.unwrap_or(0);
    let kind = match here.kind.as_deref() {
        Some(b"Tx") => Kind::Text,
        Some(b"Btn") if flags & PUSH_BUTTON != 0 => Kind::Button,
        Some(b"Btn") if flags & RADIO != 0 => Kind::Radio,
        Some(b"Btn") => Kind::Checkbox,
        Some(b"Ch") => Kind::Choice,
        Some(b"Sig") => Kind::Signature,
        _ => Kind::Other,
    };
    let options = match &here.options {
        Some(Object::Array(options)) => options.clone(),
        _ => Vec::new(),
    };
    let choices = match kind {
        Kind::Checkbox | Kind::Radio => states(document, &widgets, &options),
        Kind::Choice => options.iter().filter_map(|o| option(document, o)).collect(),
        _ => Vec::new(),
    };
    Field {
        name: here.name,
        id,
        kind,
        flags,
        widgets,
        tooltip,
        max_len: here.max_len.and_then(|most| usize::try_from(most).ok()),
        choices,
        value: here.value,
        look: here
            .look
            .and_then(|look| look.as_str().ok().map(<[u8]>::to_vec)),
    }
}

/// The states a checkbox's or radio buttons' `widgets` can be on in, each once, labelled by
/// the field's `options` where it has them.
fn states(document: &Document, widgets: &[ObjectId], options: &[Object]) -> Vec<Choice> {
    let mut found: Vec<Choice> = Vec::new();
    for (index, widget) in widgets.iter().enumerate() {
        let Some(stored) = on_state(document, *widget) else {
            continue;
        };
        let label = options
            .get(index)
            .and_then(|o| resolved(document, o))
            .map_or_else(|| String::from_utf8_lossy(&stored).into_owned(), text);
        if !found.iter().any(|choice| choice.stored == stored) {
            found.push(Choice { label, stored });
        }
    }
    found
}

/// The state the widget `id` shows when it is on: the name of its normal look that isn't
/// `Off`.
pub(super) fn on_state(document: &Document, id: ObjectId) -> Option<Vec<u8>> {
    let widget = document.get_dictionary(id).ok()?;
    let looks = resolved(document, widget.get(b"AP").ok()?)?
        .as_dict()
        .ok()?;
    let normal = resolved(document, looks.get(b"N").ok()?)?.as_dict().ok()?;
    let (name, _) = normal.iter().find(|(name, _)| name.as_slice() != b"Off")?;
    Some(name.clone())
}

/// A choice's option: a text, or its export value and the text shown for it.
fn option(document: &Document, object: &Object) -> Option<Choice> {
    match resolved(document, object)? {
        Object::Array(pair) => {
            let stored = text(resolved(document, pair.first()?)?);
            let label = pair
                .get(1)
                .and_then(|o| resolved(document, o))
                .map_or_else(|| stored.clone(), text);
            Some(Choice {
                label,
                stored: stored.into_bytes(),
            })
        }
        shown => {
            let label = text(shown);
            Some(Choice {
                stored: label.clone().into_bytes(),
                label,
            })
        }
    }
}

/// A PDF text string as text, or the texts of an array of them.
pub(super) fn text(object: &Object) -> String {
    match object {
        Object::Array(items) => items.iter().map(text).collect::<Vec<_>>().join(", "),
        Object::Name(name) => String::from_utf8_lossy(name).into_owned(),
        // lopdf drops line breaks and tabs, which PDFDocEncoding shares with ASCII.
        Object::String(ascii, _) if ascii.is_ascii() => String::from_utf8_lossy(ascii).into_owned(),
        _ => decode_text_string(object)
            .map(|text| text.trim_start_matches('\u{feff}').to_owned())
            .unwrap_or_default(),
    }
}

/// The object ids an array of references holds, the array given as `object`.
pub(super) fn references(document: &Document, object: Option<&Object>) -> Vec<ObjectId> {
    object
        .and_then(|object| resolved(document, object))
        .and_then(|object| object.as_array().ok())
        .map(|items| items.iter().filter_map(|i| i.as_reference().ok()).collect())
        .unwrap_or_default()
}

/// `document`'s form, as its own copy, when it has one.
pub(super) fn form_of(document: &Document) -> Option<Dictionary> {
    let form = document.catalog().ok()?.get(b"AcroForm").ok()?;
    resolved(document, form)?.as_dict().ok().cloned()
}

/// `document`'s form, to change in place, when it has one.
pub(super) fn form_mut(document: &mut Document) -> Option<&mut Dictionary> {
    let held = document
        .catalog()
        .ok()?
        .get(b"AcroForm")
        .ok()?
        .as_reference()
        .ok();
    match held {
        Some(id) => document.get_dictionary_mut(id).ok(),
        None => document
            .catalog_mut()
            .ok()?
            .get_mut(b"AcroForm")
            .ok()?
            .as_dict_mut()
            .ok(),
    }
}

/// `document`'s form without the widgets `gone`, and without the fields left with none of
/// theirs. Whether any field went.
pub(super) fn unwidgeted(document: &mut Document, gone: &BTreeSet<ObjectId>) -> bool {
    let Some(form) = form_of(document) else {
        return false;
    };
    let top = references(document, form.get(b"Fields").ok());
    let mut went = false;
    let kept: Vec<ObjectId> = top
        .iter()
        .copied()
        .filter(|&id| stays(document, id, gone, 0, &mut went))
        .collect();
    if kept.len() != top.len()
        && let Some(form) = form_mut(document)
    {
        form.set(
            "Fields",
            kept.into_iter().map(Object::from).collect::<Vec<_>>(),
        );
    }
    went
}

/// Whether the field or widget `id` keeps a widget on a page that stays; those of its kids
/// that don't are taken out of it, and `went` is set when a field goes.
fn stays(
    document: &mut Document,
    id: ObjectId,
    gone: &BTreeSet<ObjectId>,
    depth: usize,
    went: &mut bool,
) -> bool {
    let named = document.get_dictionary(id).is_ok_and(|d| d.has(b"T"));
    let stays = if gone.contains(&id) {
        false
    } else if depth > DEEPEST {
        true
    } else {
        let kids = document
            .get_dictionary(id)
            .ok()
            .map(|d| references(document, d.get(b"Kids").ok()))
            .unwrap_or_default();
        let kept: Vec<ObjectId> = kids
            .iter()
            .copied()
            .filter(|&kid| stays(document, kid, gone, depth + 1, went))
            .collect();
        if kept.len() != kids.len()
            && let Ok(dictionary) = document.get_dictionary_mut(id)
        {
            dictionary.set(
                "Kids",
                kept.iter()
                    .map(|&kid| Object::from(kid))
                    .collect::<Vec<_>>(),
            );
        }
        kids.is_empty() || !kept.is_empty()
    };
    if !stays && named {
        *went = true;
    }
    stays
}
