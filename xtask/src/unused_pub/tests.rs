use super::*;

/// The names of what `source` exports that nothing outside names but `outside`'s words.
fn unused_in(source: &str, outside: &str) -> Vec<String> {
    let library = exported::read_text(source).unwrap();
    let outside: BTreeSet<&str> = words(outside).collect();
    unused(&library, |name| outside.contains(name))
        .into_iter()
        .map(|item| format!("{} {}", item.kind, item.name))
        .collect()
}

#[test]
fn an_exported_item_no_other_crate_names_fails() {
    let source = "pub fn called() {}\npub fn forgotten() {}\npub(crate) fn inner() {}\nfn own() {}";
    assert_eq!(unused_in(source, "lib::called();"), ["fn forgotten"]);
}

#[test]
fn every_kind_of_public_item_is_read() {
    let source = "\
        pub struct S { pub field: u8, private: u8 }\n\
        pub enum E { A }\n\
        pub trait T {}\n\
        pub type A = u8;\n\
        pub const C: u8 = 1;\n\
        pub static V: u8 = 1;\n\
        pub mod m { pub fn inside() {} }\n\
        mod hidden { pub fn reexported() {} }\n\
        pub use hidden::reexported as renamed;\n\
        pub struct Kept;\n\
        impl Kept { pub fn method(&self) {} pub const LIMIT: u8 = 1; fn own(&self) {} }\n\
        #[macro_export]\n\
        macro_rules! shout { () => {} }";
    assert_eq!(
        unused_in(source, "Kept"),
        [
            "struct S",
            "field field",
            "enum E",
            "trait T",
            "type A",
            "const C",
            "static V",
            "mod m",
            "fn inside",
            "fn reexported",
            "use renamed",
            "fn method",
            "const LIMIT",
            "macro shout",
        ]
    );
}

#[test]
fn what_a_used_items_interface_names_stays_public() {
    let source = "\
        pub fn parse(text: &str) -> Result<Parsed, ParseError> { todo() }\n\
        pub struct Parsed { pub kind: Kind }\n\
        pub enum Kind { One(Detail) }\n\
        pub struct Detail;\n\
        pub enum ParseError { Bad }\n\
        pub fn unused() -> Lonely { todo() }\n\
        pub struct Lonely;\n\
        pub struct Number;\n\
        impl std::str::FromStr for Number { type Err = NumberError; }\n\
        pub struct NumberError;";
    assert_eq!(
        unused_in(source, "parse(text)?.kind; Number"),
        ["fn unused", "struct Lonely"]
    );
}

#[test]
fn an_item_reexported_under_a_new_name_is_called_by_it() {
    let source = "mod hidden { pub fn original() {} }\npub use hidden::original as renamed;";
    assert_eq!(unused_in(source, "lib::renamed()"), Vec::<String>::new());
}

#[test]
fn a_macros_body_keeps_what_it_calls_public() {
    let source = "\
        #[macro_export]\n\
        macro_rules! block_on { ($f:expr) => { $crate::runtime().block_on($f) } }\n\
        pub fn runtime() {}";
    assert_eq!(unused_in(source, "block_on!(work)"), Vec::<String>::new());
}

#[test]
fn tests_and_private_items_export_nothing() {
    let source = "\
        #[cfg(test)]\n\
        mod tests { pub fn helper() {} }\n\
        #[cfg(test)]\n\
        pub fn for_tests() {}\n\
        #[cfg(feature = \"conformance\")]\n\
        pub fn suite() {}\n\
        pub(crate) struct Inner;\n\
        struct Own;";
    assert_eq!(unused_in(source, ""), ["fn suite"]);
}

#[test]
fn words_are_identifiers_wherever_they_are() {
    let text = "glave_ports::Clock::now(); // r#type 2x \"in_a_string\"";
    assert_eq!(
        words(text).collect::<Vec<_>>(),
        ["glave_ports", "Clock", "now", "r", "type", "in_a_string"]
    );
}

#[test]
fn every_allowed_item_has_its_reason() {
    assert!(
        ALLOWED
            .iter()
            .all(|(name, item, why)| !name.is_empty() && !item.is_empty() && !why.is_empty())
    );
}
