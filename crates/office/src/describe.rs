//! A file glave made, described for a person (`specs/session.md`, "Files it made"): the title
//! it gives itself, how many pages, sheets or slides it has, and whether glave previews it. A
//! title the program that made it gives every file it makes (python-docx's "Word Document",
//! python-pptx's "PowerPoint Presentation") names none of them, and is left out, as is one that
//! is only a file's name: its card names it by its file name then.

use std::io::Cursor;

use calamine::Reader;
use glave_event::Parts;

use crate::command::office;
use crate::package::{Kind, Package};
use crate::pdf;

/// The titles programs give every file they make, in lowercase: python-docx's and
/// python-pptx's, Office's and its templates', and other programs' placeholders.
const DEFAULT_TITLES: [&str; 14] = [
    "word document",
    "powerpoint presentation",
    "microsoft word document",
    "microsoft powerpoint presentation",
    "microsoft excel worksheet",
    "document",
    "presentation",
    "spreadsheet",
    "title",
    "presentation title",
    "untitled",
    "untitled document",
    "untitled presentation",
    "untitled spreadsheet",
];

/// The ends of file names a title that is only a file's name has.
const FILE_ENDS: [&str; 16] = [
    "docx", "doc", "odt", "rtf", "xlsx", "xlsm", "xls", "ods", "csv", "pptx", "ppt", "odp", "pdf",
    "txt", "md", "html",
];

/// The longest title kept, in characters.
const MOST_TITLE: usize = 200;

/// What a file is, for its card.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Described {
    /// The title its own properties give it, when they give one that names it.
    pub title: Option<String>,
    /// How many pages, sheets or slides it has, when that is known.
    pub parts: Option<Parts>,
    /// Whether glave previews it: an Office file or a workbook it reads, a CSV file or a PDF.
    pub previews: bool,
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
        let read = pdf::described(bytes);
        return Described {
            title: read
                .as_ref()
                .and_then(|(_, title)| naming(title.as_deref()?)),
            parts: read.map(|(pages, _)| Parts::Pages(pages)),
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

/// The title an Office file's core properties give it, when it names the file.
fn titled(package: &mut Package<'_>) -> Option<String> {
    let core = package.xml("docProps/core.xml").ok()??;
    naming(&core.child("dc:title")?.text())
}

/// `title`, its spaces made single, when it names a file: not blank, not a title a program
/// gives every file it makes, and not only a file's name.
fn naming(title: &str) -> Option<String> {
    let title = title.split_whitespace().collect::<Vec<_>>().join(" ");
    let lower = title.to_lowercase();
    let a_file = lower
        .rsplit_once('.')
        .is_some_and(|(_, end)| FILE_ENDS.contains(&end));
    // Word's printer to PDF titles each one "Microsoft Word - " and the file's name.
    let printed = lower.starts_with("microsoft ") && lower.contains(" - ");
    if title.is_empty() || DEFAULT_TITLES.contains(&lower.as_str()) || a_file || printed {
        return None;
    }
    Some(title.chars().take(MOST_TITLE).collect())
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

    #[test]
    fn a_title_every_file_of_a_program_gets_or_a_file_s_name_names_none() {
        for left in [
            "Word Document",
            " PowerPoint  Presentation ",
            "Untitled",
            "",
            "report.docx",
            "Microsoft Word - report.docx",
        ] {
            assert_eq!(naming(left), None, "{left}");
        }
        assert_eq!(
            naming("Trip  budget\n2026").as_deref(),
            Some("Trip budget 2026")
        );
        assert_eq!(naming("Release v2.1").as_deref(), Some("Release v2.1"));
        assert_eq!(naming(&"a".repeat(300)).map(|title| title.len()), Some(200));
    }
}
