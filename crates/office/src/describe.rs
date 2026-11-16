//! A file glave made, described for a person (`specs/session.md`, "Files it made"): the title
//! it gives itself, how many pages, sheets or slides it has, and whether glave previews it.

use std::io::Cursor;

use calamine::Reader;

use crate::command::office;
use crate::package::{Kind, Package};
use crate::pdf;

/// What a file is, for its card.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Described {
    /// The title its own properties give it, when they give one.
    pub title: Option<String>,
    /// How many pages, sheets or slides it has, when that is known.
    pub parts: Option<Parts>,
    /// Whether glave previews it: an Office file or a workbook it reads, a CSV file or a PDF.
    pub previews: bool,
}

/// How many of a file's parts it has.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Parts {
    Pages(u64),
    Sheets(u64),
    Slides(u64),
}

/// The file in `bytes`, kept as `media_type`, described. One glave can't read is described
/// with nothing known and no preview.
pub fn describe(bytes: &[u8], media_type: &str) -> Described {
    if media_type == "text/csv" {
        return Described {
            previews: true,
            ..Described::default()
        };
    }
    if media_type == "application/pdf" {
        return Described {
            title: None,
            parts: pdf::pages(bytes).map(Parts::Pages),
            previews: true,
        };
    }
    match office(bytes) {
        Some((mut package, main, kind)) => {
            let title = titled(&mut package);
            let parts = match kind {
                Kind::Word => counted(&mut package, "Pages").map(Parts::Pages),
                Kind::PowerPoint => {
                    let slides = package.listed(&main, "p:sldIdLst").ok();
                    slides.map(|slides| Parts::Slides(slides.len() as u64))
                }
                Kind::Workbook => sheets(bytes).map(Parts::Sheets),
            };
            Described {
                title,
                parts,
                previews: true,
            }
        }
        // A workbook of another kind than Office's (xls, ods), which calamine reads.
        None => match sheets(bytes) {
            Some(count) => Described {
                title: None,
                parts: Some(Parts::Sheets(count)),
                previews: true,
            },
            None => Described::default(),
        },
    }
}

/// The title an Office file's core properties give it, when it is not blank.
fn titled(package: &mut Package<'_>) -> Option<String> {
    let core = package.xml("docProps/core.xml").ok()??;
    let title = core.child("dc:title")?.text();
    let title = title.trim();
    (!title.is_empty()).then(|| title.chars().take(200).collect())
}

/// A count an Office file's extended properties give, such as its `Pages`, when they give it.
fn counted(package: &mut Package<'_>, name: &str) -> Option<u64> {
    let app = package.xml("docProps/app.xml").ok()??;
    app.child(name)?.text().trim().parse().ok()
}

/// How many worksheets the workbook in `bytes` has, when calamine reads it.
fn sheets(bytes: &[u8]) -> Option<u64> {
    let book = calamine::open_workbook_auto_from_rs(Cursor::new(bytes.to_vec())).ok()?;
    Some(book.sheet_names().len() as u64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_csv_file_previews_and_an_unknown_one_does_not() {
        assert!(describe(b"a,b\n", "text/csv").previews);
        assert_eq!(
            describe(b"not a file", "application/rtf"),
            Described::default()
        );
    }
}
