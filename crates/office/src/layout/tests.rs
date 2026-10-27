use proptest::prelude::*;

use super::*;

/// A paragraph as Word writes it, on one line after the declaration's.
const WORD: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\r\n\
    <w:document xmlns:w=\"w\"><w:body><w:p><w:pPr><w:pStyle w:val=\"Title\"/></w:pPr><w:r>\
    <w:t xml:space=\"preserve\"> </w:t></w:r><w:r><w:t>Hi &amp; bye</w:t></w:r></w:p>\
    <!-- a note --><w:sectPr/></w:body></w:document>";

#[test]
fn a_part_is_spread_an_element_to_a_line_and_drawn_back_as_it_was() {
    let spread = spread(WORD);
    assert_eq!(
        spread,
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\r\n\
         <w:document xmlns:w=\"w\">\n\
         <w:body>\n \
         <w:p>\n  \
         <w:pPr>\n   \
         <w:pStyle w:val=\"Title\"/></w:pPr>\n  \
         <w:r>\n   \
         <w:t xml:space=\"preserve\"> </w:t></w:r>\n  \
         <w:r>\n   \
         <w:t>Hi &amp; bye</w:t></w:r></w:p>\n \
         <!-- a note -->\n \
         <w:sectPr/></w:body></w:document>"
    );
    assert_eq!(condensed(&spread), WORD);
    // Spreading what is spread changes nothing.
    assert_eq!(super::spread(&spread), spread);
}

#[test]
fn text_an_element_holds_alone_is_kept_and_broken_xml_left_as_it_is() {
    let kept = "<a><t>\n</t><t xml:space=\"preserve\">\n  </t> <b/></a>";
    assert_eq!(condensed(kept), kept);
    // What an edit leaves, as a cell's value taken off its line.
    assert_eq!(
        condensed("<a>\n <c>\n  <f>A1</f>\n  </c>\n <b/>\n</a>"),
        "<a><c><f>A1</f></c><b/></a>"
    );
    for broken in ["<a><b></a>", "<a x=1/>"] {
        assert_eq!(spread(broken), broken);
    }
}

/// XML as Office writes it, with nothing between one tag and the next.
fn compact() -> impl Strategy<Value = String> {
    let leaf = prop_oneof![
        "[a-z &<>]{0,6}".prop_map(|text| quick_xml::escape::escape(text.as_str()).into_owned()),
        Just("<e x=\"1\"/>".to_owned()),
    ];
    leaf.prop_recursive(4, 32, 4, |inner| {
        (
            prop::sample::select(vec!["w:p", "w:r", "c"]),
            prop::collection::vec(inner, 0..4),
        )
            .prop_map(|(name, children)| format!("<{name}>{}</{name}>", children.concat()))
    })
}

proptest! {
    #[test]
    fn what_is_spread_draws_back_together_as_it_was(inner in compact()) {
        let part = format!("<?xml version=\"1.0\"?>\r\n<root>{inner}</root>");
        prop_assert_eq!(condensed(&spread(&part)), part);
    }
}
