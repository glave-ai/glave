//! Files described and previewed for a person: a document and a deck as `glave doc read` reads
//! them without the line that names the file, a workbook's sheets as tables of their first rows,
//! everything bounded however large the file; each described by its title, pages, sheets or
//! slides.

use std::io::{Cursor, Read as _, Write as _};

use glave_event::Parts;
use glave_office::{Described, MOST_MARKDOWN, describe, preview};
use glave_ports::testing::TempDir;
use lopdf::dictionary;

use crate::pdfs::paged;
use crate::workbooks::doc;

const DOCX: &str = "application/vnd.openxmlformats-officedocument.wordprocessingml.document";
const XLSX: &str = "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet";

fn bytes(name: &str) -> Vec<u8> {
    std::fs::read(format!("{}/tests/{name}", env!("CARGO_MANIFEST_DIR"))).unwrap()
}

#[test]
fn a_document_and_a_deck_preview_as_their_text() {
    let report = preview(&bytes("report.docx"), DOCX).unwrap();
    assert!(
        report
            .markdown
            .starts_with("Header: Acme Ltd, confidential\n"),
        "{}",
        report.markdown
    );
    assert!(report.markdown.contains("# Quarterly report\n"));
    assert!(report.markdown.contains("| North | 700 |") && !report.cut);
    let deck = preview(&bytes("deck.pptx"), "").unwrap();
    assert!(deck.markdown.starts_with("## Slide 1"), "{}", deck.markdown);
}

#[test]
fn a_workbook_previews_as_a_table_for_each_sheet() {
    let book = preview(&bytes("workbook.xlsx"), XLSX).unwrap();
    assert!(book.markdown.starts_with("## "), "{}", book.markdown);
    assert!(book.markdown.contains("|---|"), "{}", book.markdown);
    assert!(
        !book.markdown.contains(" → "),
        "values as a person sees them, not formulas beside them"
    );
}

#[test]
fn a_dense_workbook_previews_its_first_rows_and_columns_and_says_it_was_cut() {
    let dir = TempDir::new("office-dense").unwrap();
    let path = dir.path().join("dense.xlsx");
    let rows: Vec<Vec<u32>> = (0..2_000)
        .map(|row| (0..40).map(|column| row * 40 + column).collect())
        .collect();
    let description = serde_json::json!({"sheets": [{"name": "Data", "rows": rows}]});
    let file = dir.path().join("dense.json");
    std::fs::write(&file, description.to_string()).unwrap();
    doc(&["write", &path.to_string_lossy(), &file.to_string_lossy()]).unwrap();
    let dense = preview(&std::fs::read(&path).unwrap(), XLSX).unwrap();
    assert!(dense.cut);
    let lines: Vec<&str> = dense.markdown.lines().collect();
    // A heading, the header row, its rule and 199 rows more.
    assert_eq!(lines.len(), 202, "{}", dense.markdown.len());
    assert_eq!(
        lines.get(1).map(|line| line.matches(" | ").count()),
        Some(25)
    );
    let described = describe(&std::fs::read(&path).unwrap(), XLSX);
    assert_eq!(described.parts, Some(Parts::Sheets(1)));
}

#[test]
fn a_large_document_is_cut_at_a_line_and_says_so() {
    let large = with_paragraphs(&bytes("report.docx"), 40_000);
    let cut = preview(&large, DOCX).unwrap();
    assert!(cut.cut && cut.markdown.len() <= MOST_MARKDOWN);
    assert!(cut.markdown.ends_with("A line the report says again."));
}

#[test]
fn a_file_is_described_by_its_title_and_its_parts() {
    let deck = describe(&bytes("deck.pptx"), "");
    assert_eq!(deck.parts, Some(Parts::Slides(2)));
    assert!(deck.previews);
    // Every sheet its workbook part lists, a hidden one and a chart's among them.
    let book = describe(&bytes("workbook.xlsx"), XLSX);
    assert_eq!(book.parts, Some(Parts::Sheets(7)));
    let mut pdf = Vec::new();
    paged(&["one", "two", "three"]).save_to(&mut pdf).unwrap();
    assert_eq!(
        describe(&pdf, "application/pdf"),
        Described {
            title: None,
            parts: Some(Parts::Pages(3)),
            previews: true,
        }
    );
    assert!(!describe(b"{\\rtf1 a}", "application/rtf").previews);
}

#[test]
fn a_pdf_is_described_by_the_title_its_information_gives_unless_a_program_gives_it_to_all() {
    let titled = |title: &str| {
        let mut document = paged(&["one"]);
        let info = document.add_object(dictionary! {
            "Title" => lopdf::text_string(title),
        });
        document.trailer.set("Info", lopdf::Object::Reference(info));
        let mut pdf = Vec::new();
        document.save_to(&mut pdf).unwrap();
        describe(&pdf, "application/pdf").title
    };
    assert_eq!(
        titled("Lisbon plan · día 1").as_deref(),
        Some("Lisbon plan · día 1")
    );
    assert_eq!(titled("untitled"), None);
    assert_eq!(titled("Microsoft Word - plan.docx"), None);
}

/// The Word document in `docx` with `count` more paragraphs after its body's last.
fn with_paragraphs(docx: &[u8], count: usize) -> Vec<u8> {
    let mut source = zip::ZipArchive::new(Cursor::new(docx)).unwrap();
    let mut out = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let paragraph = "<w:p><w:r><w:t>A line the report says again.</w:t></w:r></w:p>";
    for index in 0..source.len() {
        let mut file = source.by_index(index).unwrap();
        let name = file.name().unwrap().into_owned();
        let mut content = Vec::new();
        file.read_to_end(&mut content).unwrap();
        if name == "word/document.xml" {
            let text = String::from_utf8(content).unwrap();
            let more = paragraph.repeat(count);
            content = text
                .replacen("</w:body>", &format!("{more}</w:body>"), 1)
                .into_bytes();
        }
        out.start_file(name, zip::write::SimpleFileOptions::default())
            .unwrap();
        out.write_all(&content).unwrap();
    }
    out.finish().unwrap().into_inner()
}
