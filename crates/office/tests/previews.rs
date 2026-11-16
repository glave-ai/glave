//! Files previewed for a person, as Markdown a client draws: a document and a deck as `glave doc
//! read` reads them without the line that names the file, a workbook's sheets as tables.

use glave_office::preview;

fn bytes(name: &str) -> Vec<u8> {
    std::fs::read(format!("{}/tests/{name}", env!("CARGO_MANIFEST_DIR"))).unwrap()
}

#[test]
fn a_document_and_a_deck_preview_as_their_text() {
    let report = preview(&bytes("report.docx"), "").unwrap();
    assert!(
        report.starts_with("Header: Acme Ltd, confidential\n"),
        "{report}"
    );
    assert!(report.contains("# Quarterly report\n") && report.contains("| North | 700 |"));
    let deck = preview(&bytes("deck.pptx"), "").unwrap();
    assert!(deck.starts_with("## Slide 1"), "{deck}");
}

#[test]
fn a_workbook_previews_as_a_table_for_each_sheet() {
    let book = preview(&bytes("workbook.xlsx"), "").unwrap();
    assert!(book.starts_with("## "), "{book}");
    assert!(book.contains("|---|"), "{book}");
    assert!(
        !book.contains(" → "),
        "values as a person sees them, not formulas beside them"
    );
}
