use super::*;
use crate::package::tests::{rels, types, zipped};
use crate::package::{CONTENT_TYPES, Kind};

const MAIN: &str =
    "application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml";
const NAMESPACES: &str = "xmlns:w=\"http://schemas.openxmlformats.org/wordprocessingml/2006/main\" \
    xmlns:mc=\"http://schemas.openxmlformats.org/markup-compatibility/2006\" \
    xmlns:wps=\"http://schemas.microsoft.com/office/word/2010/wordprocessingShape\" \
    xmlns:v=\"urn:schemas-microsoft-com:vml\"";

/// A part of a Word document: its root element named `root`, holding `inner`.
fn part(root: &str, inner: &str) -> String {
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\r\n\
         <w:{root} {NAMESPACES}>{inner}</w:{root}>"
    )
}

/// A paragraph of the style `style` (none if empty), saying `text` in one run.
fn paragraph(style: &str, text: &str) -> String {
    let style = if style.is_empty() {
        String::new()
    } else {
        format!("<w:pPr><w:pStyle w:val=\"{style}\"/></w:pPr>")
    };
    format!("<w:p>{style}<w:r><w:t>{text}</w:t></w:r></w:p>")
}

/// A list item at `level` of the list `list`.
fn item(list: u32, level: u32, text: &str) -> String {
    format!(
        "<w:p><w:pPr><w:numPr><w:ilvl w:val=\"{level}\"/><w:numId w:val=\"{list}\"/></w:numPr>\
         </w:pPr><w:r><w:t>{text}</w:t></w:r></w:p>"
    )
}

/// A document whose body is `body`, written as Word writes one: styles, numbering, comments
/// and a header beside it.
fn document(body: &str) -> Vec<u8> {
    let styles = part(
        "styles",
        "<w:style w:type=\"paragraph\" w:styleId=\"Title\"><w:name w:val=\"Title\"/></w:style>\
         <w:style w:type=\"paragraph\" w:styleId=\"Heading1\"><w:name w:val=\"heading 1\"/>\
         </w:style><w:style w:type=\"paragraph\" w:styleId=\"Mine\"><w:name w:val=\"Mine\"/>\
         <w:basedOn w:val=\"Heading2\"/></w:style><w:style w:type=\"paragraph\" \
         w:styleId=\"Heading2\"><w:name w:val=\"heading 2\"/></w:style>\
         <w:style w:type=\"paragraph\" w:styleId=\"ListBullet\"><w:name w:val=\"List Bullet\"/>\
         <w:pPr><w:numPr><w:numId w:val=\"1\"/></w:numPr></w:pPr></w:style>",
    );
    let numbering = part(
        "numbering",
        "<w:abstractNum w:abstractNumId=\"0\"><w:lvl w:ilvl=\"0\"><w:numFmt w:val=\"bullet\"/>\
         </w:lvl><w:lvl w:ilvl=\"1\"><w:numFmt w:val=\"bullet\"/></w:lvl></w:abstractNum>\
         <w:abstractNum w:abstractNumId=\"1\"><w:lvl w:ilvl=\"0\"><w:start w:val=\"1\"/>\
         <w:numFmt w:val=\"decimal\"/></w:lvl><w:lvl w:ilvl=\"1\"><w:start w:val=\"1\"/>\
         <w:numFmt w:val=\"lowerLetter\"/></w:lvl></w:abstractNum>\
         <w:num w:numId=\"1\"><w:abstractNumId w:val=\"0\"/></w:num>\
         <w:num w:numId=\"2\"><w:abstractNumId w:val=\"1\"/></w:num>",
    );
    let comments = part(
        "comments",
        "<w:comment w:id=\"0\" w:author=\"Ana Lee\"><w:p><w:r><w:annotationRef/></w:r><w:r>\
         <w:t>Check against the ledger.</w:t></w:r></w:p></w:comment>",
    );
    let header = part("hdr", &paragraph("", "Acme Ltd, confidential"));
    let doc = "word/document.xml";
    zipped(&[
        (CONTENT_TYPES, &types(doc, MAIN)),
        ("_rels/.rels", &rels(&[("rId1", "officeDocument", doc)])),
        (
            doc,
            &part("document", &format!("<w:body>{body}<w:sectPr/></w:body>")),
        ),
        (
            "word/_rels/document.xml.rels",
            &rels(&[
                ("rId1", "styles", "styles.xml"),
                ("rId2", "numbering", "numbering.xml"),
                ("rId3", "comments", "comments.xml"),
                ("rId4", "header", "header1.xml"),
                ("rId5", "header", "header1.xml"),
            ]),
        ),
        ("word/styles.xml", &styles),
        ("word/numbering.xml", &numbering),
        ("word/comments.xml", &comments),
        ("word/header1.xml", &header),
    ])
}

fn read_text(bytes: &[u8]) -> String {
    let mut package = Package::open(bytes).unwrap();
    let (main, kind) = package.main().unwrap().unwrap();
    assert_eq!(kind, Kind::Word);
    read(&mut package, &main, "report.docx").unwrap()
}

#[test]
fn a_document_reads_in_order_with_headings_lists_tables_and_comments() {
    let body = [
        paragraph("Title", "Quarterly report"),
        // Word splits words across runs, as edits happened.
        "<w:p><w:pPr><w:pStyle w:val=\"Heading1\"/></w:pPr><w:r><w:t>Res</w:t></w:r>\
         <w:r w:rsidR=\"00AB12CD\"><w:t>ults</w:t></w:r></w:p>"
            .to_owned(),
        paragraph("Mine", "By region"),
        "<w:p><w:r><w:t xml:space=\"preserve\">Revenue rose to </w:t></w:r>\
         <w:commentRangeStart w:id=\"0\"/><w:r><w:rPr><w:b/></w:rPr><w:t>1,250</w:t></w:r>\
         <w:commentRangeEnd w:id=\"0\"/><w:r><w:commentReference w:id=\"0\"/></w:r>\
         <w:r><w:t xml:space=\"preserve\"> this quarter.</w:t></w:r></w:p>"
            .to_owned(),
        paragraph("ListBullet", "North led"),
        item(1, 1, "West grew"),
        item(2, 0, "Hire two people"),
        item(2, 1, "One in sales"),
        item(2, 0, "Open the office"),
        item(2, 1, "By March"),
        paragraph("", ""),
        "<w:tbl><w:tr><w:tc><w:p><w:r><w:t>Region</w:t></w:r></w:p></w:tc><w:tc><w:p><w:r>\
         <w:t>Revenue</w:t></w:r></w:p></w:tc></w:tr><w:tr><w:tc><w:tcPr><w:gridSpan \
         w:val=\"2\"/></w:tcPr><w:p><w:r><w:t>All | regions</w:t></w:r></w:p><w:p><w:r>\
         <w:t>together</w:t></w:r></w:p></w:tc></w:tr></w:tbl>"
            .to_owned(),
        "<w:p><w:r><w:t>Line one</w:t><w:br/><w:t>line two</w:t><w:br w:type=\"page\"/></w:r>\
         <w:r><w:fldChar w:fldCharType=\"begin\"/></w:r><w:r><w:instrText> PAGE </w:instrText>\
         </w:r><w:r><w:fldChar w:fldCharType=\"separate\"/></w:r><w:r><w:t>, page 3</w:t></w:r>\
         <w:r><w:fldChar w:fldCharType=\"end\"/></w:r></w:p>"
            .to_owned(),
        // A text box, with the copy Word keeps for apps that can't draw it.
        "<w:p><w:r><mc:AlternateContent><mc:Choice Requires=\"wps\"><w:drawing><wps:txbx>\
         <w:txbxContent><w:p><w:r><w:t>In a box</w:t></w:r></w:p></w:txbxContent></wps:txbx>\
         </w:drawing></mc:Choice><mc:Fallback><w:pict><v:textbox><w:txbxContent><w:p><w:r>\
         <w:t>In a box</w:t></w:r></w:p></w:txbxContent></v:textbox></w:pict></mc:Fallback>\
         </mc:AlternateContent></w:r></w:p>"
            .to_owned(),
        "<w:sdt><w:sdtPr/><w:sdtContent><w:p><w:pPr><w:outlineLvl w:val=\"2\"/></w:pPr><w:r>\
         <w:t>In a control</w:t></w:r></w:p></w:sdtContent></w:sdt>"
            .to_owned(),
    ]
    .concat();
    assert_eq!(
        read_text(&document(&body)),
        "report.docx: a Word document\n\
         Header: Acme Ltd, confidential\n\
         # Quarterly report\n\
         # Results\n\
         ## By region\n\
         Revenue rose to 1,250 this quarter.\n\
         [comment by Ana Lee on \"1,250\": Check against the ledger.]\n\
         - North led\n\
         \x20 - West grew\n\
         1. Hire two people\n\
         \x20 1. One in sales\n\
         2. Open the office\n\
         \x20 1. By March\n\
         | Region | Revenue |\n\
         |---|---|\n\
         | All \\| regions\\ntogether |  |\n\
         Line one\nline two, page 3\n\
         In a box\n\
         ### In a control\n"
    );
}

#[test]
fn tracked_changes_read_as_word_shows_them_with_markup_off() {
    let body = "<w:p><w:r><w:t xml:space=\"preserve\">Sales rose </w:t></w:r>\
        <w:del w:id=\"1\" w:author=\"Ana\"><w:r><w:delText>4%</w:delText></w:r></w:del>\
        <w:ins w:id=\"2\" w:author=\"Ana\"><w:r><w:t>5%</w:t></w:r></w:ins>\
        <w:moveFrom w:id=\"3\"><w:r><w:t> moved</w:t></w:r></w:moveFrom><w:r><w:t>.</w:t></w:r></w:p>";
    assert_eq!(
        read_text(&document(body)),
        "report.docx: a Word document with tracked changes not yet accepted: insertions read as \
         text here, and deletions are left out\n\
         Header: Acme Ltd, confidential\n\
         Sales rose 5%.\n"
    );
}

#[test]
fn a_document_with_only_its_body_still_reads() {
    let doc = "word/document.xml";
    let body = format!(
        "<w:body>{}{}</w:body>",
        paragraph("Heading2", "Plain"),
        paragraph("", "Words")
    );
    let bytes = zipped(&[
        (CONTENT_TYPES, &types(doc, MAIN)),
        ("_rels/.rels", &rels(&[("rId1", "officeDocument", doc)])),
        (doc, &part("document", &body)),
    ]);
    assert_eq!(
        read_text(&bytes),
        "report.docx: a Word document\n## Plain\nWords\n"
    );
}

#[test]
fn a_long_comment_quotes_the_start_of_what_it_is_on() {
    assert_eq!(shortened("short"), "short");
    let long = "word ".repeat(30);
    let cut = shortened(long.trim());
    assert!(
        cut.ends_with("word…") && cut.chars().count() <= QUOTED,
        "{cut}"
    );
}
