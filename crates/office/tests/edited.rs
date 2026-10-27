//! Office files edited in place as the skills teach: `glave doc unpack`, an edit to a part's
//! XML, `glave doc pack`, then `glave doc read` on what was packed.

use std::path::Path;

use glave_ports::testing::TempDir;

use crate::workbooks::{doc, has};

const REPORT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/report.docx");
const DECK: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/deck.pptx");
const WORKBOOK: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/workbook.xlsx");

fn shown(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

/// `file` unpacked into `folder` in `dir`, with what unpacking said.
fn unpacked(dir: &Path, file: &str, folder: &str) -> (std::path::PathBuf, String) {
    let folder = dir.join(folder);
    let said = doc(&["unpack", file, &shown(&folder)]).unwrap();
    (folder, said)
}

/// The part `name` in `folder` with `old` made `new`, once.
fn edit(folder: &Path, name: &str, old: &str, new: &str) {
    let path = folder.join(name);
    let text = std::fs::read_to_string(&path).unwrap();
    assert_eq!(text.matches(old).count(), 1, "{old:?} in {text}");
    std::fs::write(&path, text.replace(old, new)).unwrap();
}

#[test]
fn a_paragraph_changed_in_its_run_reads_back_with_everything_else_kept() {
    let dir = TempDir::new("office-edit-docx").unwrap();
    let (folder, said) = unpacked(&dir, REPORT, "report");
    assert!(
        said.contains(": 12 parts. The text is in word/document.xml.\n"),
        "{said}"
    );
    edit(
        &folder,
        "word/document.xml",
        "<w:t>Hire two people</w:t>",
        "<w:t>Hire three people</w:t>",
    );
    let out = dir.join("edited.docx");
    doc(&["pack", &shown(&folder), &shown(&out)]).unwrap();
    let before = doc(&["read", REPORT]).unwrap();
    let after = doc(&["read", &shown(&out)]).unwrap();
    assert_eq!(
        after,
        before
            .replace("report.docx:", "edited.docx:")
            .replace("1. Hire two people", "1. Hire three people")
    );
}

#[test]
fn broken_xml_is_never_packed_and_says_where_it_breaks() {
    let dir = TempDir::new("office-broken").unwrap();
    let (folder, _) = unpacked(&dir, REPORT, "report");
    edit(
        &folder,
        "word/document.xml",
        "<w:t>North led</w:t>",
        "<w:t>North led</w:r>",
    );
    let out = dir.join("broken.docx");
    let broken = doc(&["pack", &shown(&folder), &shown(&out)])
        .unwrap_err()
        .to_string();
    assert!(
        broken.starts_with("word/document.xml is not well-formed XML: at line ")
            && broken.contains("expected </w:t>, but </w:r> was found")
            && broken.contains("Near: ")
            && broken.ends_with("Nothing was packed: mend it, then pack again"),
        "{broken}"
    );
    assert!(!out.exists());
}

#[test]
fn a_file_is_only_packed_over_with_replace() {
    let dir = TempDir::new("office-replace").unwrap();
    let (folder, _) = unpacked(&dir, DECK, "deck");
    let out = dir.join("deck.pptx");
    std::fs::write(&out, "mine").unwrap();
    let kept = doc(&["pack", &shown(&folder), &shown(&out)])
        .unwrap_err()
        .to_string();
    assert!(kept.contains("is already there. Add --replace"), "{kept}");
    assert_eq!(std::fs::read(&out).unwrap(), b"mine");
    doc(&["pack", &shown(&folder), &shown(&out), "--replace"]).unwrap();
    assert!(
        doc(&["read", &shown(&out)])
            .unwrap()
            .contains("## Slide 2: Results")
    );
}

#[test]
fn a_deck_and_a_workbook_say_which_part_is_which() {
    let dir = TempDir::new("office-guide").unwrap();
    let (_, deck) = unpacked(&dir, DECK, "deck");
    assert!(
        deck.contains(
            ": 8 parts. The slides, in the deck's order: ppt/slides/slide1.xml, \
             ppt/slides/slide2.xml.\n"
        ),
        "{deck}"
    );
    let (folder, book) = unpacked(&dir, WORKBOOK, "book");
    assert!(
        book.contains("The sheets: Sales is xl/worksheets/sheet1.xml, Q1 sales is "),
        "{book}"
    );
    // A formula changed in place, as the spreadsheets skill says: its last value taken out.
    let sheet = "xl/worksheets/sheet1.xml";
    edit(&folder, sheet, "<f>SUM(E2:E4)</f>", "<f>SUM(E2:E3)</f>");
    edit(&folder, sheet, "<v>5905</v>", "");
    let out = dir.join("fixed.xlsx");
    doc(&["pack", &shown(&folder), &shown(&out)]).unwrap();
    has(
        &doc(&["read", &shown(&out), "--sheet", "Sales"]).unwrap(),
        &[
            "| 4 | 2026-01-07 | West | 80 | 18 | =C4*D4 → 1440 |",
            "| 5 | Total |  |  |  | =SUM(E2:E3) → not computed yet |",
        ],
    );
}

#[test]
fn only_an_office_file_unpacks_and_only_into_a_new_folder() {
    let dir = TempDir::new("office-unpack-wrong").unwrap();
    let text = dir.join("notes.txt");
    std::fs::write(&text, "not a zip").unwrap();
    let not = doc(&["unpack", &shown(&text), &shown(&dir.join("notes"))])
        .unwrap_err()
        .to_string();
    assert!(
        not.contains("is not an Office file glave can unpack"),
        "{not}"
    );
    let full = doc(&["unpack", REPORT, &shown(&dir)])
        .unwrap_err()
        .to_string();
    assert!(
        full.ends_with("is not empty: unpack into a new folder"),
        "{full}"
    );
}
