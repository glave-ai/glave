//! `glave doc merge`, `pages`, `rotate` and `fill`: the words after each, the new PDF it makes
//! and what from, and making it.

use std::fmt::Write as _;
use std::io::Read;
use std::path::{Path, PathBuf};

use super::{Answer, bytes, ends_in, json, make, usage};
use crate::error::OfficeError;
use crate::pdf::{self, Given, Ranges};
use crate::write::Written;

/// A new PDF to write.
#[derive(Debug, PartialEq, Eq)]
pub(super) struct Making {
    pub(super) out: PathBuf,
    /// Whether a file already at `out` may be written over.
    pub(super) replace: bool,
}

/// What a new PDF is made from.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum Change {
    /// PDFs whose pages it holds, each one's after the one before's.
    Merge(Vec<PathBuf>),
    /// A PDF, and the pages of it to keep, in order.
    Pages(PathBuf, Ranges),
    /// A PDF, how far to turn its pages clockwise, and which, or every page.
    Rotate(PathBuf, i64, Option<Ranges>),
    /// A PDF whose form to fill, and the values' file; standard input when `None`.
    Fill(PathBuf, Option<PathBuf>),
}

/// What `glave doc <verb>` asks of `files`, the words that aren't flags.
pub(super) fn changing(
    verb: &str,
    files: &[PathBuf],
    replace: bool,
) -> Result<(Making, Change), OfficeError> {
    let making = |out: &PathBuf| Making {
        out: out.clone(),
        replace,
    };
    match (verb, files) {
        ("merge", [out, ..]) if !ends_in(out, "pdf") => Err(usage(&format!(
            "glave doc merge makes a new PDF: name it first, like all.pdf, then the PDFs to \
             merge, not {}",
            out.display()
        ))),
        ("merge", [out, merged @ ..]) if merged.len() >= 2 => {
            Ok((making(out), Change::Merge(merged.to_vec())))
        }
        ("pages", [file, out, pages]) => {
            let ranges = ranges(verb, pages)?;
            Ok((making(out), Change::Pages(file.clone(), ranges)))
        }
        ("rotate", [file, out, degrees, pages @ ..]) if pages.len() <= 1 => {
            let said = shown(degrees);
            let turn = said
                .parse::<i64>()
                .ok()
                .filter(|turn| turn % 90 == 0 && turn.rem_euclid(360) != 0)
                .ok_or_else(|| {
                    usage(&format!(
                        "glave doc rotate turns pages by 90, 180 or 270 degrees clockwise, or \
                         -90 anticlockwise, not {said:?}"
                    ))
                })?;
            let ranges = match pages.first() {
                Some(pages) => Some(ranges(verb, pages)?),
                None => None,
            };
            Ok((making(out), Change::Rotate(file.clone(), turn, ranges)))
        }
        ("fill", [file, out, values @ ..]) if values.len() <= 1 => {
            let values = values.first().filter(|path| path.as_os_str() != "-");
            Ok((making(out), Change::Fill(file.clone(), values.cloned())))
        }
        _ => Err(usage(&format!(
            "glave doc {verb} was not given what it takes"
        ))),
    }
}

/// The new PDF `change` asks for, written as `making` says; `input` gives fill's values
/// when no file is named for them.
pub(super) fn changed(
    making: &Making,
    change: &Change,
    input: &mut dyn Read,
) -> Result<Answer, OfficeError> {
    let out = &making.out;
    let mut answer = make(out, making.replace, OfficeError::AlreadyThere, || {
        made(change, input)
    })?;
    if let Change::Fill(..) = change {
        let _ = write!(
            answer.printed,
            ". Check them: glave doc fields {}",
            out.display()
        );
    }
    Ok(answer)
}

/// The bytes of the new PDF `change` asks for, and what to say of it.
fn made(change: &Change, input: &mut dyn Read) -> Result<Written, OfficeError> {
    match change {
        Change::Merge(files) => {
            let given: Result<Vec<Given>, _> = files.iter().map(|file| given(file)).collect();
            pdf::merge(&given?)
        }
        Change::Pages(file, ranges) => pdf::keep(&given(file)?, ranges),
        Change::Rotate(file, degrees, ranges) => {
            pdf::rotate(&given(file)?, *degrees, ranges.as_ref())
        }
        Change::Fill(file, values) => {
            let json = json(
                values.as_deref(),
                input,
                "the JSON",
                OfficeError::Values,
                "no field values were given: give them as JSON on standard input, or name \
                 their file after the new PDF's",
            )?;
            pdf::fill(&given(file)?, &json)
        }
    }
}

/// The PDF at `file`, named as it was given.
pub(super) fn given(file: &Path) -> Result<Given, OfficeError> {
    Ok(Given {
        name: file.display().to_string(),
        bytes: bytes(file)?,
    })
}

/// The pages `word` names, for `glave doc <verb>`.
fn ranges(verb: &str, word: &Path) -> Result<Ranges, OfficeError> {
    let said = shown(word);
    Ranges::parse(&said).ok_or_else(|| {
        usage(&format!(
            "glave doc {verb} takes pages like 1-3,7, not {said:?}"
        ))
    })
}

/// A word that isn't a file's name after all, as it was given.
fn shown(word: &Path) -> String {
    word.to_string_lossy().into_owned()
}
