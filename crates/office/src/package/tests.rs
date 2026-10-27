use std::io::{Cursor, Write as _};

use zip::ZipWriter;
use zip::write::SimpleFileOptions;

use super::*;

pub(crate) const TYPES: &str = "http://schemas.openxmlformats.org/package/2006/content-types";
pub(crate) const RELS: &str = "http://schemas.openxmlformats.org/package/2006/relationships";
pub(crate) const TYPE: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";

/// `parts` zipped in order, as an Office app writes a file.
pub(crate) fn zipped(parts: &[(&str, &str)]) -> Vec<u8> {
    let mut zip = ZipWriter::new(Cursor::new(Vec::new()));
    for (name, text) in parts {
        zip.start_file(*name, SimpleFileOptions::default()).unwrap();
        zip.write_all(text.as_bytes()).unwrap();
    }
    zip.finish().unwrap().into_inner()
}

/// `[Content_Types].xml` with the main part `main` of the content type `main_type`.
pub(crate) fn types(main: &str, main_type: &str) -> String {
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\r\n<Types xmlns=\"{TYPES}\">\
         <Default Extension=\"rels\" ContentType=\"application/vnd.openxmlformats-package.\
         relationships+xml\"/><Default Extension=\"xml\" ContentType=\"application/xml\"/>\
         <Override PartName=\"/{main}\" ContentType=\"{main_type}\"/></Types>"
    )
}

/// A relationships part: each relationship's id, the last word of its type, and its target.
pub(crate) fn rels(relationships: &[(&str, &str, &str)]) -> String {
    let listed = relationships
        .iter()
        .map(|(id, kind, target)| {
            format!("<Relationship Id=\"{id}\" Type=\"{TYPE}/{kind}\" Target=\"{target}\"/>")
        })
        .collect::<Vec<_>>()
        .concat();
    format!("<Relationships xmlns=\"{RELS}\">{listed}</Relationships>")
}

#[test]
fn targets_are_named_from_the_root() {
    assert_eq!(
        resolved("ppt", "slides/slide1.xml"),
        "ppt/slides/slide1.xml"
    );
    assert_eq!(
        resolved("ppt/slides", "../notesSlides/n.xml"),
        "ppt/notesSlides/n.xml"
    );
    assert_eq!(resolved("word", "/word/./styles.xml"), "word/styles.xml");
    assert_eq!(resolved("", "word/document.xml"), "word/document.xml");
    assert_eq!(resolved("", "../../a.xml"), "a.xml");
}

#[test]
fn a_package_says_its_main_part_and_its_kind() {
    let word = "application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml";
    let doc = "word/document.xml";
    let bytes = zipped(&[
        (CONTENT_TYPES, &types(doc, word)),
        ("_rels/.rels", &rels(&[("rId1", "officeDocument", doc)])),
        (doc, "<w:document/>"),
        (
            "word/_rels/document.xml.rels",
            &format!(
                "<Relationships xmlns=\"{RELS}\"><Relationship Id=\"rId1\" Type=\"{TYPE}/styles\" \
                 Target=\"styles.xml\"/><Relationship Id=\"rId2\" Type=\"{TYPE}/hyperlink\" \
                 Target=\"https://example.com/\" TargetMode=\"External\"/></Relationships>"
            ),
        ),
    ]);
    let mut package = Package::open(&bytes).unwrap();
    assert_eq!(package.main().unwrap(), Some((doc.to_owned(), Kind::Word)));
    let related = package.related(doc).unwrap();
    assert_eq!(related.len(), 2);
    assert_eq!(related[0].kind, "styles");
    assert_eq!(related[0].target.as_deref(), Some("word/styles.xml"));
    assert_eq!(related[1].target, None, "a link points outside the file");
    // Part names are compared as OPC compares them, their case aside.
    assert!(package.part("WORD/Document.xml").unwrap().is_some());
    assert!(package.part("word/missing.xml").unwrap().is_none());
    let types = package.xml(CONTENT_TYPES).unwrap().unwrap();
    assert_eq!(content_type(&types, "word/DOCUMENT.xml"), Some(word));
    assert_eq!(
        content_type(&types, "word/styles.xml"),
        Some("application/xml")
    );
    assert_eq!(content_type(&types, "word/media/a.png"), None);

    assert!(Package::open(b"not a zip").is_none());
    assert!(
        Package::open(&zipped(&[("mimetype", "x")])).is_none(),
        "no content types"
    );
}

#[test]
fn a_part_that_is_not_xml_says_which() {
    let bytes = zipped(&[(CONTENT_TYPES, "<Types>"), ("a.xml", "<a>\u{0}")]);
    let mut package = Package::open(&bytes).unwrap();
    let broken = package.xml(CONTENT_TYPES).unwrap_err().to_string();
    assert!(
        broken.starts_with("[Content_Types].xml is not well-formed XML: at line 1"),
        "{broken}"
    );
    // A file with no relationships is taken for a workbook, for calamine to try.
    assert_eq!(package.main().unwrap(), None);
}
