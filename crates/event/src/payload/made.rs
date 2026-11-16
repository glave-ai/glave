//! Files a call made for the person (`specs/session.md`, "Files it made"): a document, a
//! spreadsheet, a deck or a PDF a tool wrote on the thread's computer, kept for the thread so it
//! outlives the computer, and named in the call's `tool.call.completed`, in `structured`.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::blob::BlobRef;

/// A file a call made, as its `tool.call.completed` records it in `structured`, under `made`:
/// where it was made and its content, kept in the blob store for the thread. The model never
/// sees it; the person opens and downloads it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileMade {
    /// Where it was made, as the computer resolved it, relative to the thread's folder.
    pub path: String,
    /// Its content, kept for the thread.
    pub blob: BlobRef,
}

impl FileMade {
    /// The tools whose calls make files: glave's own `write` and `bash`. Another tool's result
    /// is never taken for one that made a file, whatever it records.
    pub const TOOLS: [&'static str; 2] = ["write", "bash"];

    /// The `structured` result of a call that made `made`, kept beside what `structured`
    /// already holds; `structured` itself when it made none.
    pub fn structured(structured: Option<Value>, made: &[Self]) -> Option<Value> {
        if made.is_empty() {
            return structured;
        }
        let mut object = match structured {
            Some(Value::Object(object)) => object,
            _ => serde_json::Map::new(),
        };
        object.insert("made".to_owned(), serde_json::json!(made));
        Some(Value::Object(object))
    }

    /// The files a call's `structured` result says it made, in order; none when it says none.
    pub fn read(structured: &Value) -> Vec<Self> {
        structured
            .get("made")
            .and_then(|made| Vec::<Self>::deserialize(made).ok())
            .unwrap_or_default()
    }
}

/// What kind of file a made file is, for the person: what a card names it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum FileKind {
    Document,
    Spreadsheet,
    Deck,
    Pdf,
    Other,
}

/// The files counted as made, by the end of their name: each one's media type and kind.
const MADE: [(&str, &str, FileKind); 14] = [
    (
        "docx",
        "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        FileKind::Document,
    ),
    ("doc", "application/msword", FileKind::Document),
    (
        "odt",
        "application/vnd.oasis.opendocument.text",
        FileKind::Document,
    ),
    ("rtf", "application/rtf", FileKind::Document),
    (
        "xlsx",
        "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        FileKind::Spreadsheet,
    ),
    (
        "xlsm",
        "application/vnd.ms-excel.sheet.macroEnabled.12",
        FileKind::Spreadsheet,
    ),
    ("xls", "application/vnd.ms-excel", FileKind::Spreadsheet),
    (
        "ods",
        "application/vnd.oasis.opendocument.spreadsheet",
        FileKind::Spreadsheet,
    ),
    ("csv", "text/csv", FileKind::Spreadsheet),
    (
        "pptx",
        "application/vnd.openxmlformats-officedocument.presentationml.presentation",
        FileKind::Deck,
    ),
    ("ppt", "application/vnd.ms-powerpoint", FileKind::Deck),
    (
        "odp",
        "application/vnd.oasis.opendocument.presentation",
        FileKind::Deck,
    ),
    ("pdf", "application/pdf", FileKind::Pdf),
    ("epub", "application/epub+zip", FileKind::Other),
];

impl FileKind {
    /// The media type a file named `path` is kept as when it counts as made: a document,
    /// spreadsheet, deck, PDF or e-book, by the end of its name, whatever its case. `None` for
    /// any other file, which a call may write without making anything for the person.
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
        let sheet = FileKind::made_as("out/Trip Budget.XLSX");
        assert_eq!(sheet.map(FileKind::of), Some(FileKind::Spreadsheet));
        assert_eq!(
            FileKind::made_as("plan.docx").map(FileKind::of),
            Some(FileKind::Document)
        );
        assert_eq!(
            FileKind::made_as("pitch.pptx").map(FileKind::of),
            Some(FileKind::Deck)
        );
        assert_eq!(FileKind::made_as("a.pdf"), Some("application/pdf"));
        assert_eq!(
            FileKind::made_as("book.epub").map(FileKind::of),
            Some(FileKind::Other)
        );
        for not in [
            "main.rs",
            "notes.md",
            "pdf",
            ".pdf",
            "image.png",
            "dir.docx/",
        ] {
            assert_eq!(FileKind::made_as(not), None, "{not}");
        }
        assert_eq!(FileKind::of("application/zip"), FileKind::Other);
    }

    #[test]
    fn made_files_are_kept_beside_what_a_result_records() {
        let made = FileMade {
            path: "out/budget.xlsx".into(),
            blob: BlobRef {
                sha256: Sha256::new("a".repeat(64)).unwrap(),
                bytes: 3,
                media_type: "text/csv".into(),
            },
        };
        let structured = FileMade::structured(
            Some(serde_json::json!({"answers": []})),
            std::slice::from_ref(&made),
        );
        let structured = structured.unwrap_or_default();
        assert_eq!(structured["answers"], serde_json::json!([]));
        assert_eq!(FileMade::read(&structured), vec![made]);
        assert_eq!(FileMade::structured(None, &[]), None);
        assert!(FileMade::read(&serde_json::json!({"made": "no"})).is_empty());
    }
}
