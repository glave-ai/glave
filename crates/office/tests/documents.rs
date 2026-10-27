//! `glave doc read` on a Word document and a PowerPoint deck, as the model runs it. Both were
//! made for these tests from hand-written XML, zipped as Office writes its files. `report.docx`
//! has a header and a footer, a title, a heading whose word Word split across two runs, a
//! comment, a tracked deletion and insertion, a bulleted and a numbered list, a table and a
//! link. `deck.pptx` has a title slide, then a slide with bullets at two levels, a table and
//! speaker notes.

use glave_office::USAGE;

use crate::workbooks::doc;

const REPORT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/report.docx");
const DECK: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/deck.pptx");

#[test]
fn a_word_document_reads_as_its_text_in_order() {
    assert_eq!(
        doc(&["read", REPORT]).unwrap(),
        "report.docx: a Word document with tracked changes not yet accepted: insertions read as \
         text here, and deletions are left out\n\
         Header: Acme Ltd, confidential\n\
         Footer: Page 1\n\
         # Quarterly report\n\
         # Results\n\
         Revenue rose to 1,250 this quarter, up 5% on the last.\n\
         [comment by Ana Lee on \"1,250\": Check against the ledger.]\n\
         - North led\n\
         - South held\n\
         1. Hire two people\n\
         2. Open the new office\n\
         | Region | Revenue |\n\
         |---|---|\n\
         | North | 700 |\n\
         | South | 550 |\n\
         See the ledger.\n"
    );
}

#[test]
fn a_deck_reads_slide_by_slide_with_its_notes() {
    assert_eq!(
        doc(&["read", DECK]).unwrap(),
        "deck.pptx: 2 slides\n\
         \n## Slide 1: Quarterly review\n\
         Q3 2026\n\
         \n## Slide 2: Results\n\
         - Revenue up 5%\n\
         \x20 - North led\n\
         | Region | Revenue |\n\
         |---|---|\n\
         | North | 700 |\n\
         Notes: Mention the West's growth.\n"
    );
}

#[test]
fn what_only_a_workbook_has_is_not_asked_of_a_document() {
    let sheet = doc(&["read", REPORT, "--sheet", "Sales"])
        .unwrap_err()
        .to_string();
    assert!(
        sheet.starts_with(
            "--sheet and --rows are for workbooks, and report.docx is a Word document\n"
        ) && sheet.ends_with(USAGE),
        "{sheet}"
    );
    let csv = doc(&["csv", DECK]).unwrap_err().to_string();
    assert!(
        csv.ends_with(": deck.pptx is a PowerPoint deck, which glave doc read reads"),
        "{csv}"
    );
}
