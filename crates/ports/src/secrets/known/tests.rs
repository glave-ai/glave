use super::*;
use crate::testing::any_secret;

/// Output written, whole.
#[derive(Default)]
struct Written(Mutex<String>);

impl Output for Written {
    fn write<'a>(&'a self, text: &'a str) -> BoxFuture<'a, ()> {
        self.0.lock().unwrap().push_str(text);
        Box::pin(std::future::ready(()))
    }
}

fn known(values: &[(&str, &str)]) -> KnownSecrets {
    let known = KnownSecrets::default();
    let all = values
        .iter()
        .map(|(name, value)| Known::new((*name).to_owned(), Secret::new(*value)));
    known.set(all.collect());
    known
}

#[test]
fn commands_see_values_and_output_never_shows_one() {
    let stripe = any_secret();
    // A value that holds another is masked whole.
    let longer = format!("{stripe}-and-more");
    assert_eq!(
        KnownSecrets::default().mask(&stripe),
        stripe,
        "none kept yet"
    );
    let known = known(&[("STRIPE_KEY", &stripe), ("OTHER_KEY", &longer)]);
    let variables = known.variables();
    assert_eq!(
        variables.get("STRIPE_KEY").map(Secret::expose),
        Some(stripe.as_str())
    );
    assert!(!format!("{variables:?}{known:?}").contains(&stripe));
    let shown = format!("curl said {stripe} and {longer}\n");
    assert_eq!(known.mask(&shown), "curl said [redacted] and [redacted]\n");
    assert_eq!(known.mask("nothing here"), "nothing here");
}

#[test]
fn a_tool_s_texts_and_record_are_masked_whatever_the_value_holds() {
    // A quote and a backslash, which a record's JSON would escape.
    let value = format!("{}\"\\x", any_secret());
    let known = known(&[("KEY", &value)]);
    let output = ToolOutput {
        structured: Some(serde_json::json!({"page": {"said": [format!("got {value}")]}})),
        ..ToolOutput::text(false, format!("got {value}"))
    };
    let output = known.masked(output);
    assert_eq!(
        output.parts,
        [Part::Text {
            text: "got [redacted]".into()
        }]
    );
    let record = output.structured.unwrap();
    assert_eq!(record["page"]["said"][0], "got [redacted]");
}

#[test]
fn a_value_split_across_writes_is_masked_before_any_of_it_is_passed_on() {
    let value = any_secret();
    let known = known(&[("KEY", &value)]);
    let written = Written::default();
    crate::block_on!(async {
        let masking = known.masking(&written);
        let (start, end) = value.split_at(5);
        for text in ["line one\nsent ", start, end, "\nline ", "two\n"] {
            masking.write(text).await;
            assert!(
                !written.0.lock().unwrap().contains(start),
                "{start} passed on"
            );
        }
        masking.flush().await;
    });
    assert_eq!(
        *written.0.lock().unwrap(),
        "line one\nsent [redacted]\nline two\n"
    );
}
