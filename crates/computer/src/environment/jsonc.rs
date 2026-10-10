//! JSON with comments, as devcontainer.json is written: `//` and `/* */` comments and
//! trailing commas are removed, leaving plain JSON. Strings are left exactly as they are.

/// `text` without comments or trailing commas. An unclosed block comment runs to the end.
pub(crate) fn strip(text: &str) -> String {
    let mut plain = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '"' => {
                plain.push(c);
                let mut escaped = false;
                for c in chars.by_ref() {
                    plain.push(c);
                    if escaped {
                        escaped = false;
                    } else if c == '\\' {
                        escaped = true;
                    } else if c == '"' {
                        break;
                    }
                }
            }
            '/' if chars.peek() == Some(&'/') => {
                for c in chars.by_ref() {
                    if c == '\n' {
                        plain.push('\n');
                        break;
                    }
                }
            }
            '/' if chars.peek() == Some(&'*') => {
                chars.next();
                let mut star = false;
                for c in chars.by_ref() {
                    if star && c == '/' {
                        break;
                    }
                    star = c == '*';
                }
                plain.push(' ');
            }
            '}' | ']' => {
                drop_trailing_comma(&mut plain);
                plain.push(c);
            }
            _ => plain.push(c),
        }
    }
    plain
}

/// Removes a comma that only whitespace separates from the end of `plain`.
fn drop_trailing_comma(plain: &mut String) {
    let kept = plain.trim_end().len();
    if plain
        .get(..kept)
        .is_some_and(|before| before.ends_with(','))
    {
        plain.truncate(kept.saturating_sub(1));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;
    use serde_json::Value;

    #[test]
    fn comments_and_trailing_commas_go() {
        let text = r#"{
            // the image
            "image": "rust:1", /* inline */
            "url": "http://x//y /* not a comment */",
            "list": [1, 2,],
        }"#;
        let value: Value = serde_json::from_str(&strip(text)).unwrap();
        assert_eq!(value["image"], "rust:1");
        assert_eq!(value["url"], "http://x//y /* not a comment */");
        assert_eq!(value["list"], serde_json::json!([1, 2]));
    }

    fn json() -> impl Strategy<Value = Value> {
        let leaf = prop_oneof![
            Just(Value::Null),
            any::<bool>().prop_map(Value::Bool),
            any::<i32>().prop_map(Value::from),
            ".{0,12}".prop_map(Value::String),
        ];
        leaf.prop_recursive(3, 24, 4, |inner| {
            prop_oneof![
                proptest::collection::vec(inner.clone(), 0..4).prop_map(Value::Array),
                proptest::collection::btree_map(".{0,6}", inner, 0..4)
                    .prop_map(|map| Value::Object(map.into_iter().collect())),
            ]
        })
    }

    proptest! {
        /// Plain JSON, compact or pretty, reads the same after stripping.
        #[test]
        fn plain_json_is_unchanged(value in json()) {
            for text in [value.to_string(), serde_json::to_string_pretty(&value).unwrap()] {
                let back: Value = serde_json::from_str(&strip(&text)).unwrap();
                prop_assert_eq!(&back, &value);
            }
        }

        /// Comments after every line change nothing.
        #[test]
        fn commented_json_reads_the_same(value in json()) {
            let pretty = serde_json::to_string_pretty(&value).unwrap();
            let commented = pretty.lines().fold(String::new(), |mut all, line| {
                all.push_str(line);
                all.push_str(" /* note */ // more\n");
                all
            });
            let back: Value = serde_json::from_str(&strip(&commented)).unwrap();
            prop_assert_eq!(back, value);
        }

        #[test]
        fn any_text_is_stripped_without_crashing(text in "\\PC{0,80}") {
            let _ = strip(&text);
        }
    }
}
