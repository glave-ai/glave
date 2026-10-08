use super::*;

const FILE: &str = "fn main() {\n    let name = \"glave\";\n    println!(\"hi {name}\");\n}\n";

#[test]
fn exact_unique_match_replaces() {
    let edited = apply(FILE, "\"glave\"", "\"hope\"", false).unwrap();
    assert_eq!(edited.stage, Stage::Exact);
    assert!(edited.text.contains("let name = \"hope\";"));
    assert_eq!(
        (edited.first_line, edited.last_line, edited.replacements),
        (2, 2, 1)
    );
}

#[test]
fn ambiguous_matches_name_their_lines() {
    let text = "a = 1\nb = 1\na = 1\n";
    assert_eq!(
        apply(text, "a = 1", "a = 2", false),
        Err(EditError::Ambiguous { lines: vec![1, 3] })
    );
    let all = apply(text, "a = 1", "a = 2", true).unwrap();
    assert_eq!(all.text, "a = 2\nb = 1\na = 2\n");
    assert_eq!(all.replacements, 2);
}

#[test]
fn empty_and_identical_are_refused() {
    assert_eq!(apply(FILE, "", "x", false), Err(EditError::Empty));
    assert_eq!(
        apply(FILE, "same", "same", false),
        Err(EditError::Identical)
    );
}

#[test]
fn copied_line_numbers_are_removed() {
    let old = "2\t    let name = \"glave\";\n3\t    println!(\"hi {name}\");";
    let new = "2\t    let name = \"hope\";\n3\t    println!(\"hello {name}\");";
    let edited = apply(FILE, old, new, false).unwrap();
    assert_eq!(edited.stage, Stage::LineNumbers);
    assert!(
        edited
            .text
            .contains("    let name = \"hope\";\n    println!(\"hello {name}\");")
    );
}

#[test]
fn trailing_whitespace_is_forgiven() {
    let old = "    let name = \"glave\";   \n    println!(\"hi {name}\");";
    let edited = apply(FILE, old, "    let name = \"hope\";", false).unwrap();
    assert_eq!(edited.stage, Stage::TrailingSpace);
    assert_eq!(edited.text, "fn main() {\n    let name = \"hope\";\n}\n");
}

#[test]
fn indentation_is_forgiven_and_the_new_text_reindented() {
    let old = "let name = \"glave\";\nprintln!(\"hi {name}\");";
    let new = "let name = \"hope\";\nif true {\n    println!(\"hi {name}\");\n}";
    let edited = apply(FILE, old, new, false).unwrap();
    assert_eq!(edited.stage, Stage::Indentation);
    assert_eq!(
        edited.text,
        "fn main() {\n    let name = \"hope\";\n    if true {\n        println!(\"hi {name}\");\n    }\n}\n"
    );
}

#[test]
fn typographic_quotes_are_forgiven() {
    let old = "let name = \u{201C}glave\u{201D};";
    let edited = apply(FILE, old, "let name = \"hope\";", false).unwrap();
    assert_eq!(edited.stage, Stage::Typography);
    assert!(edited.text.contains("    let name = \"hope\";"));
}

#[test]
fn not_found_points_at_the_closest_line() {
    let result = apply(FILE, "println!(\"hi {name}\"); // nope", "x", false);
    assert_eq!(
        result,
        Err(EditError::NotFound {
            hint: Some((3, "    println!(\"hi {name}\");".into()))
        })
    );
}

#[test]
fn similar_but_different_text_is_not_matched() {
    assert!(matches!(
        apply(FILE, "let nam = \"glave\";", "x", false),
        Err(EditError::NotFound { .. })
    ));
}

proptest::proptest! {
    /// A unique exact match is replaced once, in place, and the edit can be undone exactly.
    #[test]
    fn a_unique_match_is_replaced_and_can_be_undone(
        prefix in "[a-z \n]{0,40}",
        needle in "[A-Z]{3,8}",
        suffix in "[a-z \n]{0,40}",
        new in "[0-9]{1,8}",
    ) {
        let text = format!("{prefix}{needle}{suffix}");
        let edited = apply(&text, &needle, &new, false).unwrap();
        proptest::prop_assert_eq!(&edited.text, &format!("{prefix}{new}{suffix}"));
        proptest::prop_assert_eq!(edited.replacements, 1);
        let undone = apply(&edited.text, &new, &needle, false).unwrap();
        proptest::prop_assert_eq!(undone.text, text);
    }

    /// Whatever the model sends, the matcher answers or refuses; it never panics.
    #[test]
    fn arbitrary_edits_never_panic(
        text in "\\PC{0,200}",
        old in "\\PC{0,20}",
        new in "\\PC{0,20}",
        all in proptest::prelude::any::<bool>(),
    ) {
        let _ = apply(&text, &old, &new, all);
    }
}
