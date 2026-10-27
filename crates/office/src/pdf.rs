//! PDFs changed with lopdf: pages merged, kept, reordered and turned (`glave doc merge`,
//! `pages` and `rotate`), and forms listed and filled (`fields` and `fill`). Reading stays
//! with the `read` tool, which shows a PDF to the model as it is.
//!
//! Each reads whole PDFs and gives the bytes of a new one, which `glave doc` writes. An
//! encrypted PDF is refused, so its protection is never taken off unseen.

mod appearance;
mod fields;
mod fill;
mod form;
mod pages;
mod ranges;

use std::collections::BTreeSet;

use lopdf::{Document, LoadOptions, Object, ObjectId};

use crate::error::OfficeError;

pub(crate) use fields::listed as fields;
pub(crate) use fill::fill;
pub(crate) use pages::{keep, merge, rotate};
pub(crate) use ranges::Ranges;

/// A PDF given to a command: its name as it was given, and its bytes.
pub(crate) struct Given {
    pub(crate) name: String,
    pub(crate) bytes: Vec<u8>,
}

/// The most bytes one stream may inflate to while a PDF loads.
const MAX_STREAM: usize = 256 * 1024 * 1024;

/// How deep a page tree or a form's fields may nest before they are taken for a loop.
const DEEPEST: usize = 64;

/// What a command does with a PDF.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Use {
    /// Reads it only: one whose maker protected it against changes may still be read.
    Read,
    /// Makes a new PDF from it.
    Change,
}

/// The PDF `given`, unless it is not one, can't be read, has no pages, or is encrypted in a
/// way `using` it can't get past.
fn open(given: &Given, using: Use) -> Result<Document, OfficeError> {
    let name = &given.name;
    let head = given.bytes.get(..1024).unwrap_or(&given.bytes);
    if !head.windows(5).any(|window| window == b"%PDF-") {
        return Err(OfficeError::NotAPdf(name.clone()));
    }
    let options = LoadOptions {
        max_decompressed_size: Some(MAX_STREAM),
        ..LoadOptions::default()
    };
    let document = Document::load_mem_with_options(&given.bytes, options)
        .map_err(|error| OfficeError::Pdf(format!("{name} could not be read as a PDF: {error}")))?;
    // lopdf opens one that needs no password to read, and leaves `Encrypt` on one that does.
    if document.trailer.has(b"Encrypt") {
        return Err(OfficeError::Encrypted(format!(
            "{name} is encrypted and needs a password to open"
        )));
    }
    if using == Use::Change && document.was_encrypted() {
        return Err(OfficeError::Encrypted(format!(
            "{name} is encrypted by its maker against changes"
        )));
    }
    if document.page_iter().next().is_none() {
        return Err(OfficeError::Pdf(format!("{name} has no pages")));
    }
    Ok(document)
}

/// `document` as the bytes of a PDF file.
fn saved(mut document: Document) -> Result<Vec<u8>, OfficeError> {
    let mut bytes = Vec::new();
    document
        .save_to(&mut bytes)
        .map_err(|error| OfficeError::Pdf(format!("the new PDF could not be written: {error}")))?;
    Ok(bytes)
}

/// The object `object` is, following a reference to it.
fn resolved<'a>(document: &'a Document, object: &'a Object) -> Option<&'a Object> {
    document.dereference(object).ok().map(|(_, object)| object)
}

/// Every object reachable from `document`'s trailer; the rest is what nothing uses any more.
fn reachable(document: &Document) -> BTreeSet<ObjectId> {
    let mut seen = BTreeSet::new();
    let mut waiting = Vec::new();
    for (_, value) in &document.trailer {
        references(value, &mut waiting);
    }
    while let Some(id) = waiting.pop() {
        if seen.insert(id)
            && let Some(object) = document.objects.get(&id)
        {
            references(object, &mut waiting);
        }
    }
    seen
}

/// Each reference `object` holds, however deep, added to `found`.
fn references(object: &Object, found: &mut Vec<ObjectId>) {
    match object {
        Object::Reference(id) => found.push(*id),
        Object::Array(items) => items.iter().for_each(|item| references(item, found)),
        Object::Dictionary(dictionary) => {
            for (_, value) in dictionary {
                references(value, found);
            }
        }
        Object::Stream(stream) => {
            for (_, value) in &stream.dict {
                references(value, found);
            }
        }
        _ => {}
    }
}

/// `document` without the objects nothing reaches any more.
fn pruned(document: &mut Document) {
    let kept = reachable(document);
    document.objects.retain(|id, _| kept.contains(id));
}

/// `count` of a thing, its plural when it isn't one: `1 page`, `3 pages`.
fn counted(count: usize, one: &str, many: &str) -> String {
    if count == 1 {
        format!("1 {one}")
    } else {
        format!("{count} {many}")
    }
}
