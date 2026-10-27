//! Pages made into a new PDF: `glave doc merge` puts each PDF's pages after the one before's,
//! `pages` keeps the pages named, in the order named, and `rotate` turns them.
//!
//! The first PDF is the base: its catalog stays, with its bookmarks, its metadata and its
//! form, and the pages chosen, its own and the others', become its only pages under one new
//! page tree. Whatever only the pages left out used goes with them, the form fields on them
//! too, so a page taken out is out of the file. The others' forms join the base's; their
//! bookmarks are left out.

use std::collections::BTreeSet;
use std::fmt::Write as _;

use lopdf::{Dictionary, Document, Object, ObjectId};

use super::form::{form_of, unwidgeted};
use super::{DEEPEST, Given, Ranges, Use, counted, open, pruned, resolved, saved};
use crate::error::OfficeError;
use crate::write::Written;

/// What a page takes from the page tree above it when it has none of its own.
const INHERITED: [&[u8]; 4] = [b"Resources", b"MediaBox", b"CropBox", b"Rotate"];

/// The PDFs `given`, each one's pages after the one before's.
pub(crate) fn merge(given: &[Given]) -> Result<Written, OfficeError> {
    let mut opened = Vec::new();
    for pdf in given {
        opened.push(open(pdf, Use::Change)?);
    }
    let mut documents = opened.into_iter().zip(given);
    let Some((mut base, first)) = documents.next() else {
        return Err(OfficeError::Pdf("there are no PDFs to merge".to_owned()));
    };
    let mut pages: Vec<ObjectId> = base.page_iter().collect();
    let mut parts = vec![format!("{}'s {}", first.name, pages.len())];
    let (mut forms, mut unmarked) = (Vec::new(), Vec::new());
    for (document, pdf) in documents {
        let document = shifted(document, base.max_id);
        let theirs: Vec<ObjectId> = document.page_iter().collect();
        parts.push(format!("{}'s {}", pdf.name, theirs.len()));
        if document
            .catalog()
            .is_ok_and(|catalog| catalog.has(b"Outlines"))
        {
            unmarked.push(pdf.name.clone());
        }
        forms.extend(form_of(&document));
        base.max_id = base.max_id.max(document.max_id);
        if document.version > base.version {
            base.version.clone_from(&document.version);
        }
        base.objects.extend(document.objects);
        pages.extend(theirs);
    }
    join_forms(&mut base, forms);
    rebuild(&mut base, &pages, &pages);
    let mut report = format!(
        "{}, {}",
        counted(pages.len(), "page", "pages"),
        parts.join(" then ")
    );
    if !unmarked.is_empty() {
        let _ = write!(
            report,
            ". The bookmarks of {} were left out: only the first PDF's are kept",
            unmarked.join(" and ")
        );
    }
    Ok(Written {
        bytes: saved(base)?,
        report,
    })
}

/// The pages of `given` that `ranges` names, in that order, each once.
pub(crate) fn keep(given: &Given, ranges: &Ranges) -> Result<Written, OfficeError> {
    let mut document = open(given, Use::Change)?;
    let all: Vec<ObjectId> = document.page_iter().collect();
    let numbers = numbered(given, &all, ranges)?;
    let mut seen = BTreeSet::new();
    if let Some(twice) = numbers.iter().find(|number| !seen.insert(**number)) {
        return Err(OfficeError::Pdf(format!(
            "page {twice} is named twice: glave doc pages keeps each page once"
        )));
    }
    let kept: Vec<ObjectId> = numbers.iter().filter_map(|n| page(&all, *n)).collect();
    let unfilled = rebuild(&mut document, &kept, &all);
    let mut report = format!(
        "{}'s pages {ranges}, {} of its {}",
        given.name,
        kept.len(),
        all.len()
    );
    if unfilled {
        report.push_str(". The form fields on the pages left out went with them");
    }
    Ok(Written {
        bytes: saved(document)?,
        report,
    })
}

/// `given` with the pages `ranges` names, or every page, turned `degrees` clockwise.
pub(crate) fn rotate(
    given: &Given,
    degrees: i64,
    ranges: Option<&Ranges>,
) -> Result<Written, OfficeError> {
    let mut document = open(given, Use::Change)?;
    let all: Vec<ObjectId> = document.page_iter().collect();
    let numbers = match ranges {
        Some(ranges) => numbered(given, &all, ranges)?,
        None => (1..=u32::try_from(all.len()).unwrap_or(u32::MAX)).collect(),
    };
    let mut turned = BTreeSet::new();
    for id in numbers.iter().filter_map(|n| page(&all, *n)) {
        if turned.insert(id) {
            settle(&mut document, id);
            if let Ok(page) = document.get_dictionary_mut(id) {
                let now = page.get(b"Rotate").and_then(Object::as_i64).unwrap_or(0);
                page.set("Rotate", (now + degrees).rem_euclid(360));
            }
        }
    }
    let which = match ranges {
        Some(ranges) => format!("pages {ranges} of {}", all.len()),
        None if all.len() == 1 => "its page".to_owned(),
        None => format!("all {} pages", all.len()),
    };
    let way = if degrees < 0 {
        format!("{} degrees anticlockwise", degrees.unsigned_abs())
    } else {
        format!("{degrees} degrees clockwise")
    };
    Ok(Written {
        bytes: saved(document)?,
        report: format!("{which} turned {way}"),
    })
}

/// The page numbers `ranges` names in `given`, whose pages are `all`.
fn numbered(given: &Given, all: &[ObjectId], ranges: &Ranges) -> Result<Vec<u32>, OfficeError> {
    ranges
        .pages(u32::try_from(all.len()).unwrap_or(u32::MAX))
        .map_err(|missing| {
            OfficeError::Pdf(format!(
                "{} has {}, so it has no page {missing}",
                given.name,
                counted(all.len(), "page", "pages")
            ))
        })
}

/// The page numbered `number`, from 1, of `all`.
fn page(all: &[ObjectId], number: u32) -> Option<ObjectId> {
    let index = usize::try_from(number).ok()?.checked_sub(1)?;
    all.get(index).copied()
}

/// `document` with every object numbered past `past`, so it can join a document whose
/// objects are numbered up to it.
fn shifted(mut document: Document, past: u32) -> Document {
    let objects = std::mem::take(&mut document.objects);
    for ((number, generation), mut object) in objects {
        moved(&mut object, past);
        document
            .objects
            .insert((number.saturating_add(past), generation), object);
    }
    for (_, value) in &mut document.trailer {
        moved(value, past);
    }
    document.max_id = document.max_id.saturating_add(past);
    document
}

/// `object` with each reference it holds numbered past `past`.
fn moved(object: &mut Object, past: u32) {
    match object {
        Object::Reference((number, _)) => *number = number.saturating_add(past),
        Object::Array(items) => items.iter_mut().for_each(|item| moved(item, past)),
        Object::Dictionary(dictionary) => {
            for (_, value) in dictionary.iter_mut() {
                moved(value, past);
            }
        }
        Object::Stream(stream) => {
            for (_, value) in &mut stream.dict {
                moved(value, past);
            }
        }
        _ => {}
    }
}

/// `document` with `kept`, of its pages `all`, as its only pages, in order, under one new
/// page tree, and the rest of `all` taken out with whatever only they used. Whether form
/// fields went with them.
fn rebuild(document: &mut Document, kept: &[ObjectId], all: &[ObjectId]) -> bool {
    for &page in kept {
        settle(document, page);
    }
    let root = document.new_object_id();
    for &page in kept {
        if let Ok(page) = document.get_dictionary_mut(page) {
            page.set("Parent", root);
        }
    }
    let mut tree = Dictionary::new();
    tree.set("Type", "Pages");
    tree.set(
        "Kids",
        kept.iter().map(|&id| id.into()).collect::<Vec<Object>>(),
    );
    tree.set("Count", i64::try_from(kept.len()).unwrap_or(i64::MAX));
    document.objects.insert(root, tree.into());
    if let Ok(catalog) = document.catalog_mut() {
        catalog.set("Pages", root);
        // Labels count pages by place, so they would name the wrong ones now.
        catalog.remove(b"PageLabels");
    }
    let kept: BTreeSet<ObjectId> = kept.iter().copied().collect();
    let mut gone = BTreeSet::new();
    for id in all.iter().filter(|id| !kept.contains(id)) {
        if let Some(Object::Dictionary(page)) = document.objects.remove(id)
            && let Some(Object::Array(annotations)) =
                page.get(b"Annots").ok().and_then(|a| resolved(document, a))
        {
            gone.extend(annotations.iter().filter_map(|a| a.as_reference().ok()));
        }
    }
    let unfilled = !gone.is_empty() && unwidgeted(document, &gone);
    pruned(document);
    unfilled
}

/// `page` given what it inherits from the page tree above it, so it stands on its own.
fn settle(document: &mut Document, page: ObjectId) {
    let Ok(own) = document.get_dictionary(page) else {
        return;
    };
    let mut found = Vec::new();
    for key in INHERITED.into_iter().filter(|key| !own.has(key)) {
        let mut parent = own.get(b"Parent").and_then(Object::as_reference).ok();
        for _ in 0..DEEPEST {
            let Some(node) = parent.and_then(|id| document.get_dictionary(id).ok()) else {
                break;
            };
            if let Ok(value) = node.get(key) {
                found.push((key, value.clone()));
                break;
            }
            parent = node.get(b"Parent").and_then(Object::as_reference).ok();
        }
    }
    if let Ok(own) = document.get_dictionary_mut(page) {
        for (key, value) in found {
            own.set(key, value);
        }
    }
}

/// `base`'s form with the fields of `others` after its own: each keeps the look its own form
/// gave it, and the fonts they name join the base's. An XFA form is dropped: it describes the
/// pages one PDF had, so viewers that prefer it would show the merged form wrong.
fn join_forms(base: &mut Document, others: Vec<Dictionary>) {
    if others.is_empty() {
        return;
    }
    let mut form = form_of(base).unwrap_or_default();
    let mut fields = listed(base, form.get(b"Fields").ok());
    let mut resources = owned(base, form.get(b"DR").ok());
    let mut fonts = owned(base, resources.get(b"Font").ok());
    for other in others {
        let theirs = listed(base, other.get(b"Fields").ok());
        if let Ok(look) = other.get(b"DA") {
            for id in theirs.iter().filter_map(|field| field.as_reference().ok()) {
                if let Ok(field) = base.get_dictionary_mut(id)
                    && !field.has(b"DA")
                {
                    field.set("DA", look.clone());
                }
            }
        }
        fields.extend(theirs);
        let their_resources = owned(base, other.get(b"DR").ok());
        for (name, font) in &owned(base, their_resources.get(b"Font").ok()) {
            if !fonts.has(name) {
                fonts.set(name.clone(), font.clone());
            }
        }
        if other
            .get(b"NeedAppearances")
            .and_then(Object::as_bool)
            .unwrap_or(false)
        {
            form.set("NeedAppearances", true);
        }
    }
    resources.set("Font", fonts);
    form.set("DR", resources);
    form.set("Fields", fields);
    form.remove(b"XFA");
    if let Ok(catalog) = base.catalog_mut() {
        catalog.set("AcroForm", form);
    }
}

/// The array `object` is or refers to, as its own copy; empty when it is none.
fn listed(document: &Document, object: Option<&Object>) -> Vec<Object> {
    object
        .and_then(|object| resolved(document, object))
        .and_then(|object| object.as_array().ok())
        .cloned()
        .unwrap_or_default()
}

/// The dictionary `object` is or refers to, as its own copy; empty when it is none.
fn owned(document: &Document, object: Option<&Object>) -> Dictionary {
    object
        .and_then(|object| resolved(document, object))
        .and_then(|object| object.as_dict().ok())
        .cloned()
        .unwrap_or_default()
}
