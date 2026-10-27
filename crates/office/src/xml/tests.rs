use super::*;

const W: &str = "http://schemas.openxmlformats.org/wordprocessingml/2006/main";
const R: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";

/// Where `text` breaks, as reading it and checking it both say.
fn broken_by(text: &str) -> Broken {
    let broken = parse(text).unwrap_err();
    assert_eq!(check(text), Err(broken.clone()), "{text:?}");
    broken
}

#[test]
fn names_take_office_s_own_prefixes_whatever_the_file_declared() {
    // A Strict file, with prefixes of its own choosing.
    let text = format!(
        "<?xml version=\"1.0\"?>\n<x:document xmlns:x=\"http://purl.oclc.org/ooxml/wordprocessingml/main\" \
         xmlns:rel=\"{R}\" xmlns:o=\"urn:other\"><x:body><x:p rel:id=\"rId1\" x:val=\"a\" \
         o:extra=\"b\" plain=\"c\"/><o:thing/></x:body></x:document>"
    );
    let root = parse(&text).unwrap();
    assert_eq!(root.name, "w:document");
    let paragraph = root.at(&["w:body", "w:p"]).unwrap();
    assert_eq!(paragraph.attribute("r:id"), Some("rId1"));
    assert_eq!(paragraph.attribute("w:val"), Some("a"));
    assert_eq!(
        paragraph.attribute("o:extra"),
        Some("b"),
        "unknown ones as written"
    );
    assert_eq!(paragraph.attribute("plain"), Some("c"));
    assert!(root.at(&["w:body", "o:thing"]).is_some());
    // The package's own namespaces are unprefixed, as Office writes them.
    let rels = parse(
        "<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">\
         <Relationship Id=\"rId1\" Target=\"a.xml\"/></Relationships>",
    )
    .unwrap();
    assert_eq!(
        rels.child("Relationship").unwrap().attribute("Id"),
        Some("rId1")
    );
}

#[test]
fn text_reads_with_its_references_and_cdata() {
    let text = format!(
        "<w:t xmlns:w=\"{W}\" a=\"x &amp; y\">Tom &amp; Jerry &#x41;&#66; <![CDATA[<raw>]]></w:t>"
    );
    let root = parse(&text).unwrap();
    assert_eq!(root.text(), "Tom & Jerry AB <raw>");
    assert_eq!(root.attribute("a"), Some("x & y"));
    let paragraph = parse(&format!(
        "<w:p xmlns:w=\"{W}\"><w:r><w:t>a</w:t></w:r><w:t>b</w:t></w:p>"
    ))
    .unwrap();
    let mut found = Vec::new();
    paragraph.every("w:t", &mut found);
    let texts: Vec<String> = found.iter().map(|t| t.text()).collect();
    assert_eq!(texts, ["a", "b"]);
}

#[test]
fn a_broken_part_says_where_and_why() {
    let mismatched = broken_by("<a>\n  <b>text</a>");
    assert_eq!((mismatched.line, mismatched.column), (2, 10));
    assert!(mismatched.why.contains("expected </b>"), "{mismatched:?}");
    assert_eq!(mismatched.near, "<a> <b>text</a>");
    assert_eq!(
        mismatched.to_string(),
        format!(
            "at line 2, column 10, {}. Near: <a> <b>text</a>",
            mismatched.why
        )
    );
    for (text, why) in [
        ("<a><b>", "<b> is never closed"),
        ("<w:p/>", "the prefix w: in <w:p> is not declared"),
        ("<a>&nbsp;</a>", "&nbsp; is not an entity XML knows"),
        ("<a/><b/>", "a second root element"),
        ("<a/>x", "text outside the root element"),
        ("<a>\u{b}</a>", "the control character U+000B"),
        ("<a x=\"1\" x=\"2\"/>", "an attribute is given twice"),
        ("<a x=\"&nbsp;\"/>", "&nbsp; is not an entity XML knows"),
        ("</a>", "does not match any open tag"),
        ("<a><!-- a -- b --></a>", "--"),
        ("", "there is no root element"),
    ] {
        let broken = broken_by(text);
        assert!(broken.why.contains(why), "{text:?}: {broken:?}");
    }
    // Where in a tag an attribute goes wrong.
    let unquoted = broken_by("<root>\n<a x=1/></root>");
    assert_eq!((unquoted.line, unquoted.column), (2, 6));
    assert_eq!(unquoted.why, "an attribute's value must be in quotes");
    let well_formed = "\u{feff}<?xml version=\"1.0\"?>\r\n<a b='1'>x</a>\n";
    let fine = parse(well_formed).unwrap();
    assert_eq!((fine.name.as_str(), fine.text().as_str()), ("a", "x"));
    assert_eq!(check(well_formed), Ok(()));
}

#[test]
fn where_a_long_line_breaks_is_shown_around_it_on_any_character() {
    let text = format!("<a>{}<b></a>", "é".repeat(100));
    let broken = broken_by(&text);
    assert_eq!(broken.line, 1);
    assert!(broken.near.ends_with("é<b></a>"), "{broken:?}");
    assert!(broken.near.chars().count() < 60, "{broken:?}");
}
