use glave_ports::Hit;
use serde_json::json;

use super::*;

fn arguments(value: Value) -> Arguments {
    match value {
        Value::Object(map) => Arguments::Json(map),
        other => Arguments::Malformed(other.to_string()),
    }
}

#[test]
fn a_call_asks_a_query_and_at_most_twenty_results() {
    let ask = asked(&arguments(json!({"query": "  rust async  "}))).unwrap();
    assert_eq!((ask.query.as_str(), ask.count), ("rust async", RESULTS));
    let ask = asked(&arguments(json!({"query": "q", "count": 3}))).unwrap();
    assert_eq!(ask.count, 3);
    for wrong in [
        json!({}),
        json!({"query": "  "}),
        json!({"query": "q", "count": 0}),
        json!({"query": "q", "count": 21}),
        json!({"query": "q", "count": "five"}),
        json!({"query": "x".repeat(LONGEST_QUERY + 1)}),
    ] {
        assert!(asked(&arguments(wrong.clone())).is_err(), "{wrong}");
    }
}

#[test]
fn a_call_s_line_says_what_was_searched() {
    let line = describe(&arguments(json!({"query": "tokio\n  runtime"})));
    assert_eq!(line, "Searched the web: tokio runtime");
    let long = describe(&arguments(json!({"query": "a ".repeat(200)})));
    assert!(long.ends_with('…') && long.chars().count() < 160, "{long}");
}

#[test]
fn what_was_found_is_numbered_with_each_page_s_address() {
    let found = SearchResults {
        hits: vec![
            Hit {
                title: "Tokio".into(),
                url: "https://tokio.rs/".into(),
                snippet: "A runtime.".into(),
            },
            Hit {
                title: "smol".into(),
                url: "https://github.com/smol-rs/smol".into(),
                snippet: String::new(),
            },
        ],
        cost: None,
    };
    assert_eq!(
        found_text(&found),
        "1. Tokio\n   https://tokio.rs/\n   A runtime.\n2. smol\n   https://github.com/smol-rs/smol"
    );
    assert_eq!(found_text(&SearchResults::default()), "No results.");
}

#[test]
fn a_refusal_is_told_in_the_search_s_terms() {
    let limited = ProviderError::Http {
        status: 429,
        body: "{}".into(),
    };
    assert!(not_searched(&limited).contains("Try again"));
    let out = ProviderError::Unpaid(glave_ports::Unpaid::Spent);
    assert!(
        not_searched(&out).contains("balance is used up"),
        "{}",
        not_searched(&out)
    );
}
