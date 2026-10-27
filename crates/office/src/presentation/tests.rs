use super::*;
use crate::package::tests::{rels, types, zipped};
use crate::package::{CONTENT_TYPES, Kind};

const MAIN: &str =
    "application/vnd.openxmlformats-officedocument.presentationml.presentation.main+xml";
const NAMESPACES: &str = "xmlns:a=\"http://schemas.openxmlformats.org/drawingml/2006/main\" \
    xmlns:r=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships\" \
    xmlns:p=\"http://schemas.openxmlformats.org/presentationml/2006/main\"";

/// A shape: a placeholder of `kind` (`""` for one with no type, `None` for a text box) holding
/// `paragraphs`.
fn shape(kind: Option<&str>, paragraphs: &str) -> String {
    let placeholder = match kind {
        None => String::new(),
        Some("") => "<p:ph idx=\"1\"/>".to_owned(),
        Some(kind) => format!("<p:ph type=\"{kind}\"/>"),
    };
    format!(
        "<p:sp><p:nvSpPr><p:cNvPr id=\"2\" name=\"Shape\"/><p:cNvSpPr/><p:nvPr>{placeholder}\
         </p:nvPr></p:nvSpPr><p:spPr/><p:txBody><a:bodyPr/><a:lstStyle/>{paragraphs}\
         </p:txBody></p:sp>"
    )
}

/// A paragraph saying `text` in runs, with its own properties `properties`.
fn said(properties: &str, text: &str) -> String {
    let runs = text
        .split('|')
        .map(|run| format!("<a:r><a:rPr lang=\"en-GB\"/><a:t>{run}</a:t></a:r>"))
        .collect::<Vec<_>>()
        .concat();
    format!("<a:p>{properties}{runs}</a:p>")
}

fn slide(show: &str, shapes: &str) -> String {
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\r\n<p:sld {NAMESPACES}{show}>\
         <p:cSld><p:spTree><p:nvGrpSpPr/><p:grpSpPr/>{shapes}</p:spTree></p:cSld></p:sld>"
    )
}

/// A deck of two slides whose parts are named against their order, the second hidden, with
/// a title, bullets, a text box in a group, a table and speaker notes.
fn deck() -> Vec<u8> {
    let first = slide(
        "",
        &[
            shape(Some("ctrTitle"), &said("", "Quarterly |review")),
            shape(Some("subTitle"), &said("", "Q3 2026")),
            shape(Some("sldNum"), &said("", "1")),
        ]
        .concat(),
    );
    let table = "<p:graphicFrame><a:graphic><a:graphicData><a:tbl><a:tblGrid/>\
        <a:tr><a:tc><a:txBody><a:p><a:r><a:t>Region</a:t></a:r></a:p></a:txBody></a:tc>\
        <a:tc><a:txBody><a:p><a:r><a:t>Revenue</a:t></a:r></a:p></a:txBody></a:tc></a:tr>\
        <a:tr><a:tc gridSpan=\"2\"><a:txBody><a:p><a:r><a:t>All</a:t></a:r></a:p></a:txBody>\
        </a:tc><a:tc hMerge=\"1\"><a:txBody><a:p><a:r><a:t>gone</a:t></a:r></a:p></a:txBody>\
        </a:tc></a:tr></a:tbl></a:graphicData></a:graphic></p:graphicFrame>";
    let second = slide(
        " show=\"0\"",
        &[
            shape(Some("title"), &said("", "Results")),
            shape(
                Some(""),
                &[
                    said("", "North led"),
                    said("<a:pPr lvl=\"1\"/>", "Up 4%"),
                    said("<a:pPr><a:buNone/></a:pPr>", "Source: ledger"),
                    "<a:p><a:r><a:t>Line</a:t></a:r><a:br/><a:r><a:t>break</a:t></a:r></a:p>"
                        .to_owned(),
                    "<a:p><a:endParaRPr/></a:p>".to_owned(),
                ]
                .concat(),
            ),
            format!(
                "<p:grpSp><p:nvGrpSpPr/><p:grpSpPr/>{}</p:grpSp>",
                shape(None, &said("", "Draft"))
            ),
            table.to_owned(),
        ]
        .concat(),
    );
    let notes = format!(
        "<p:notes {NAMESPACES}><p:cSld><p:spTree>{}{}</p:spTree></p:cSld></p:notes>",
        shape(Some("sldImg"), ""),
        shape(Some("body"), &said("", "Mention the West."))
    );
    let main = "ppt/presentation.xml";
    zipped(&[
        (CONTENT_TYPES, &types(main, MAIN)),
        ("_rels/.rels", &rels(&[("rId1", "officeDocument", main)])),
        (
            main,
            &format!(
                "<p:presentation {NAMESPACES}><p:sldIdLst><p:sldId id=\"256\" r:id=\"rId3\"/>\
                 <p:sldId id=\"257\" r:id=\"rId2\"/></p:sldIdLst></p:presentation>"
            ),
        ),
        (
            "ppt/_rels/presentation.xml.rels",
            &rels(&[
                ("rId2", "slide", "slides/slide1.xml"),
                ("rId3", "slide", "slides/slide2.xml"),
            ]),
        ),
        ("ppt/slides/slide1.xml", &second),
        ("ppt/slides/slide2.xml", &first),
        (
            "ppt/slides/_rels/slide1.xml.rels",
            &rels(&[("rId1", "notesSlide", "../notesSlides/notesSlide1.xml")]),
        ),
        ("ppt/notesSlides/notesSlide1.xml", &notes),
    ])
}

#[test]
fn a_deck_reads_slide_by_slide_in_its_order_with_notes() {
    let bytes = deck();
    let mut package = Package::open(&bytes).unwrap();
    let (main, kind) = package.main().unwrap().unwrap();
    assert_eq!(kind, Kind::PowerPoint);
    assert_eq!(
        read(&mut package, &main, "deck.pptx").unwrap(),
        "deck.pptx: 2 slides\n\
         \n## Slide 1: Quarterly review\n\
         Q3 2026\n\
         \n## Slide 2 (hidden): Results\n\
         - North led\n\
         \x20 - Up 4%\n\
         Source: ledger\n\
         - Line\nbreak\n\
         Draft\n\
         | Region | Revenue |\n\
         |---|---|\n\
         | All |  |\n\
         Notes: Mention the West.\n"
    );
}
