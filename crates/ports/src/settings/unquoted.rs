//! A settings file's parser error said without the values it shows. serde names a value of the
//! wrong kind as it is written: a string quoted, escapes and all (`invalid type: string
//! "Bearer ab\"cd"`), a number or a character in backticks (`integer `12345``). A settings file's
//! values include secrets: a store's password, a server's token in its headers or environment.
//! Where and what is wrong is enough to mend it, so those go and the rest stays: names in
//! backticks (a key, a variant, what toml expected), and where it went wrong.

/// What serde puts before a value it shows in backticks. A variant's name (`unknown variant
/// `…``) stays: a name the file chose, not a value it holds.
const SHOWN: [&str; 3] = ["integer ", "floating point ", "character "];

/// `message` with the values it shows left out: `string "…"`, `integer `…``.
pub fn unquoted(message: &str) -> String {
    let mut said = String::with_capacity(message.len());
    let mut rest = message;
    while let Some(at) = rest.find(['`', '"']) {
        let (before, from) = rest.split_at(at);
        said.push_str(before);
        if let Some(quoted) = from.strip_prefix('"') {
            said.push_str("\"…\"");
            rest = after_quote(quoted);
            continue;
        }
        let ticked = from.get(1..).unwrap_or_default();
        match ticked.split_once('`') {
            Some((_, after)) if SHOWN.iter().any(|shown| said.ends_with(shown)) => {
                said.push_str("`…`");
                rest = after;
            }
            Some((name, after)) => {
                said.push('`');
                said.push_str(name);
                said.push('`');
                rest = after;
            }
            // A backtick that never closes is only a character.
            None => {
                said.push('`');
                rest = ticked;
            }
        }
    }
    said.push_str(rest);
    said
}

/// What follows the quoted string whose text, from just after its opening quote, is `quoted`:
/// past its closing quote, a backslash escaping the character after it. Nothing, when it never
/// closes: the rest may be the value.
fn after_quote(quoted: &str) -> &str {
    let mut escaped = false;
    for (at, character) in quoted.char_indices() {
        if escaped {
            escaped = false;
        } else if character == '\\' {
            escaped = true;
        } else if character == '"' {
            return quoted.get(at + 1..).unwrap_or_default();
        }
    }
    ""
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;

    const WHERE: &str = "at line 1 column 5";

    #[test]
    fn values_go_and_names_and_places_stay() {
        assert_eq!(
            unquoted("invalid type: string \"x\", expected `a` or `b` at line 2 column 3"),
            "invalid type: string \"…\", expected `a` or `b` at line 2 column 3"
        );
        assert_eq!(
            unquoted(
                r#"invalid type: string "Bearer ab\"cd\\", expected a map at line 1 column 5"#
            ),
            "invalid type: string \"…\", expected a map at line 1 column 5"
        );
        assert_eq!(
            unquoted("invalid type: integer `12345`, expected a string in `env`"),
            "invalid type: integer `…`, expected a string in `env`"
        );
        assert_eq!(
            unquoted("unknown variant `fast`, expected `auto` or `ask`"),
            "unknown variant `fast`, expected `auto` or `ask`"
        );
        assert_eq!(unquoted("no values"), "no values");
    }

    #[test]
    fn what_toml_expected_stays_whole() {
        assert_eq!(
            unquoted("invalid basic string, expected `\"`"),
            "invalid basic string, expected `\"`"
        );
    }

    #[test]
    fn a_quote_that_never_closes_takes_the_rest_with_it() {
        assert_eq!(unquoted("string \"abc, at line 1"), "string \"…\"");
        assert_eq!(unquoted("a ` alone"), "a ` alone");
    }

    // Each says exactly what is left: no part of the value, the rest whole.
    proptest! {
        #[test]
        fn a_string_shown_is_never_said(value in any::<String>()) {
            let said = unquoted(&format!(
                "invalid type: string {value:?}, expected a map {WHERE}"
            ));
            prop_assert_eq!(said, format!("invalid type: string \"…\", expected a map {WHERE}"));
        }

        #[test]
        fn a_string_of_quotes_backslashes_and_backticks_is_never_said(
            value in "[\"\\\\`a-z]{0,24}"
        ) {
            let said = unquoted(&format!(
                "invalid value: string {value:?}, expected `a` {WHERE}"
            ));
            prop_assert_eq!(said, format!("invalid value: string \"…\", expected `a` {WHERE}"));
        }

        #[test]
        fn a_number_shown_is_never_said(value in any::<i64>()) {
            let said = unquoted(&format!(
                "invalid type: integer `{value}`, expected a string {WHERE}"
            ));
            prop_assert_eq!(said, format!("invalid type: integer `…`, expected a string {WHERE}"));
        }

        #[test]
        fn a_fraction_shown_is_never_said(value in any::<f64>()) {
            let said = unquoted(&format!(
                "invalid type: floating point `{value}`, expected a string {WHERE}"
            ));
            prop_assert_eq!(
                said,
                format!("invalid type: floating point `…`, expected a string {WHERE}")
            );
        }

        // serde shows a character raw between backticks: any but a backtick itself, which
        // can't end the span it opens.
        #[test]
        fn a_character_shown_is_never_said(value in any::<char>().prop_filter("a backtick", |c| *c != '`')) {
            let said = unquoted(&format!(
                "invalid type: character `{value}`, expected a string {WHERE}"
            ));
            prop_assert_eq!(said, format!("invalid type: character `…`, expected a string {WHERE}"));
        }
    }
}
