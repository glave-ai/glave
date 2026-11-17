//! Files a turn made for the person (`specs/session.md`, "Files it made"): a document, a
//! spreadsheet, a deck or a PDF changed in the thread's folder over the turn, kept for the
//! thread so it outlives the computer, and named in its `turn.ended`, under `made`, with what
//! glave read of it apart from itself: its title, its pages, sheets or slides, and its preview.
//! A turn's end says where its look-over stopped (`made_until_ns`), and whether a command the thread started
//! in the background still ran then (`made_waits`), so that what may still have been being
//! written is looked over later: a `turn.made`, once the last such command ends while the thread
//! is idle, or the next working turn's end.

use serde::{Deserialize, Serialize};
use serde_json::value::RawValue;

use crate::blob::BlobRef;
use crate::codec::{Body, Payload, parse};
use crate::envelope::Seq;
use crate::payload::library::AssetKind;

pub(crate) const TURN_MADE: &str = "turn.made";

/// How many of its parts a file has: a document's pages, a workbook's sheets, a deck's slides.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Parts {
    Pages(u64),
    Sheets(u64),
    Slides(u64),
}

/// A file a turn made, as its `turn.ended` records it: where it was made, its size, its content
/// kept for the thread or why it was not, and what glave read of it. The model never sees it;
/// the person opens and downloads it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FileMade {
    /// Where it was made, relative to the thread's folder.
    pub path: String,
    /// Its size on the computer.
    pub bytes: u64,
    pub kept: Kept,
    /// What glave read of it apart from itself, once kept; absent when it couldn't be read.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub read: Option<FileRead>,
}

/// What glave read of a file a turn made, in a process of its own.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FileRead {
    /// The title its own properties give it, when they give one that names it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// How many pages, sheets or slides it has, when that is known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parts: Option<Parts>,
    /// Its preview, when glave previews a file of its kind.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preview: Option<MadePreview>,
}

/// A file's preview: Markdown, kept for the thread as the file is.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MadePreview {
    pub blob: BlobRef,
    /// Whether it leaves out some of the file.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub cut: bool,
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
    /// Left on the computer: its parts inflate past what is looked through for secrets.
    Unchecked,
}

/// What the files a turn made leave out.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum MadeLeft {
    /// More were made than a turn keeps: these many more stayed on the computer, unnamed.
    More { files: u64 },
    /// The folder held too many files to look over in time, so none are named.
    TooMany,
}

/// What a look-over while the thread was idle found (`turn.made`): once the last command the
/// thread ran in the background ended, the files changed since where the turn's own look-over
/// stopped, which are that turn's. It looks up to its end, as no command runs then.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TurnMade {
    /// The turn's `turn.started`.
    pub turn: Seq,
    /// Where this look-over stopped, in nanoseconds since 1970 by the computer's clock: the
    /// next starts no earlier.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub made_until_ns: Option<u64>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub made: Vec<FileMade>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub made_left: Option<MadeLeft>,
}

impl Payload for TurnMade {
    const TYPE: &'static str = TURN_MADE;
    // 2 has what was read of each file (`read`); a version 1 look-over read none.
    const VERSION: u32 = 2;
}

/// Decodes `turn.made`; `None` for any other type or version.
pub(crate) fn decode(
    kind: &str,
    version: u32,
    data: &RawValue,
) -> Option<serde_json::Result<Body>> {
    (kind == TURN_MADE && matches!(version, 1 | 2)).then(|| parse(data).map(Body::TurnMade))
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
    fn parts_are_written_by_what_they_count() {
        let written = serde_json::to_string(&Parts::Sheets(2)).unwrap();
        assert_eq!(written, r#"{"sheets":2}"#);
        assert_eq!(
            serde_json::from_str::<Parts>(&written).unwrap(),
            Parts::Sheets(2)
        );
    }

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
    fn a_made_file_says_whether_it_was_kept_and_what_was_read_of_it() {
        let blob = BlobRef {
            sha256: Sha256::new("a".repeat(64)).unwrap(),
            bytes: 3,
            media_type: "application/pdf".into(),
        };
        let kept = FileMade {
            path: "out/plan.pdf".into(),
            bytes: 3,
            kept: Kept::Blob(blob),
            read: Some(FileRead {
                title: Some("Lisbon plan".into()),
                parts: Some(Parts::Pages(1)),
                preview: None,
            }),
        };
        let json = serde_json::to_value(&kept).unwrap();
        assert_eq!(json["kept"]["blob"]["bytes"], 3);
        assert_eq!(json["read"]["parts"]["pages"], 1);
        let left = FileMade {
            kept: Kept::Unchecked,
            read: None,
            ..kept
        };
        let json = serde_json::to_value(&left).unwrap();
        assert_eq!(json["kept"], "unchecked");
        assert!(json.get("read").is_none());
        let more = serde_json::to_value(MadeLeft::More { files: 3 }).unwrap();
        assert_eq!(more, serde_json::json!({"kind": "more", "files": 3}));
    }
}
