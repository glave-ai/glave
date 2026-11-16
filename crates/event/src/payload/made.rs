//! Files a turn made for the person (`specs/session.md`, "Files it made"): a document, a
//! spreadsheet, a deck or a PDF new or changed in the thread's folder over the turn, kept for
//! the thread so it outlives the computer, and named in its `turn.ended`, under `made`.

use serde::{Deserialize, Serialize};

use crate::blob::BlobRef;
use crate::payload::library::AssetKind;

/// A file a turn made, as its `turn.ended` records it: where it was made, its size, and its
/// content kept for the thread, or why it was not. The model never sees it; the person opens
/// and downloads it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FileMade {
    /// Where it was made, relative to the thread's folder.
    pub path: String,
    /// Its size on the computer.
    pub bytes: u64,
    pub kept: Kept,
}

/// Whether a file a turn made was kept for the thread.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Kept {
    /// Kept, sealed, in the blob store, the thread holding it.
    Blob(BlobRef),
    /// Left on the computer: larger than a thread keeps.
    TooLarge,
    /// Left on the computer: it holds the value of one of the person's secrets.
    Secret,
}

/// The files counted as made, by the end of their name: each one's media type and kind.
const MADE: [(&str, &str, AssetKind); 14] = [
    (
        "docx",
        "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        AssetKind::Document,
    ),
    ("doc", "application/msword", AssetKind::Document),
    (
        "odt",
        "application/vnd.oasis.opendocument.text",
        AssetKind::Document,
    ),
    ("rtf", "application/rtf", AssetKind::Document),
    (
        "xlsx",
        "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        AssetKind::Spreadsheet,
    ),
    (
        "xlsm",
        "application/vnd.ms-excel.sheet.macroEnabled.12",
        AssetKind::Spreadsheet,
    ),
    ("xls", "application/vnd.ms-excel", AssetKind::Spreadsheet),
    (
        "ods",
        "application/vnd.oasis.opendocument.spreadsheet",
        AssetKind::Spreadsheet,
    ),
    ("csv", "text/csv", AssetKind::Spreadsheet),
    (
        "pptx",
        "application/vnd.openxmlformats-officedocument.presentationml.presentation",
        AssetKind::Deck,
    ),
    ("ppt", "application/vnd.ms-powerpoint", AssetKind::Deck),
    (
        "odp",
        "application/vnd.oasis.opendocument.presentation",
        AssetKind::Deck,
    ),
    ("pdf", "application/pdf", AssetKind::Pdf),
    ("epub", "application/epub+zip", AssetKind::Other),
];

impl AssetKind {
    /// The media type a file named `path` is kept as when it counts as made: a document,
    /// spreadsheet, deck, PDF or e-book, by the end of its name, whatever its case. `None` for
    /// any other file.
    pub fn made_as(path: &str) -> Option<&'static str> {
        let name = path.rsplit('/').next().unwrap_or(path);
        let (stem, extension) = name.rsplit_once('.')?;
        if stem.is_empty() {
            return None;
        }
        MADE.iter()
            .find(|(end, _, _)| end.eq_ignore_ascii_case(extension))
            .map(|(_, media_type, _)| *media_type)
    }

    /// The file pattern (`files/glob`) of every file that would count as made, in lower and
    /// upper case: `*.{docx,DOCX,…}`.
    pub fn made_pattern() -> String {
        let ends: Vec<String> = MADE
            .iter()
            .flat_map(|(end, _, _)| [(*end).to_owned(), end.to_ascii_uppercase()])
            .collect();
        format!("*.{{{}}}", ends.join(","))
    }

    /// The kind of a made file kept as `media_type`; [`Self::Other`] for one glave doesn't name.
    pub fn of(media_type: &str) -> Self {
        MADE.iter()
            .find(|(_, made, _)| *made == media_type)
            .map_or(Self::Other, |(_, _, kind)| *kind)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::blob::Sha256;

    #[test]
    fn a_file_counts_as_made_by_the_end_of_its_name() {
        let sheet = AssetKind::made_as("out/Trip Budget.XLSX");
        assert_eq!(sheet.map(AssetKind::of), Some(AssetKind::Spreadsheet));
        assert_eq!(
            AssetKind::made_as("plan.docx").map(AssetKind::of),
            Some(AssetKind::Document)
        );
        assert_eq!(
            AssetKind::made_as("pitch.pptx").map(AssetKind::of),
            Some(AssetKind::Deck)
        );
        assert_eq!(AssetKind::made_as("a.pdf"), Some("application/pdf"));
        assert_eq!(
            AssetKind::made_as("book.epub").map(AssetKind::of),
            Some(AssetKind::Other)
        );
        for not in [
            "main.rs",
            "notes.md",
            "pdf",
            ".pdf",
            "image.png",
            "dir.docx/",
        ] {
            assert_eq!(AssetKind::made_as(not), None, "{not}");
        }
        assert_eq!(AssetKind::of("application/zip"), AssetKind::Other);
        let pattern = AssetKind::made_pattern();
        assert!(pattern.starts_with("*.{docx,DOCX,doc,DOC,") && pattern.ends_with("epub,EPUB}"));
    }

    #[test]
    fn a_made_file_says_whether_it_was_kept() {
        let kept = FileMade {
            path: "out/plan.pdf".into(),
            bytes: 3,
            kept: Kept::Blob(BlobRef {
                sha256: Sha256::new("a".repeat(64)).unwrap(),
                bytes: 3,
                media_type: "application/pdf".into(),
            }),
        };
        let json = serde_json::to_value(&kept).unwrap();
        assert_eq!(json["kept"]["blob"]["bytes"], 3);
        let left = FileMade {
            kept: Kept::TooLarge,
            ..kept
        };
        assert_eq!(serde_json::to_value(&left).unwrap()["kept"], "too_large");
    }
}
