//! Reasoning details merged from their deltas, and only one field sent back.

use super::*;
use serde_json::json;

fn kept(chunks: Vec<Value>) -> Value {
    let mut kept = Kept::default();
    for chunk in chunks {
        let Value::Array(deltas) = chunk else {
            panic!("a chunk's details are a list");
        };
        kept.add_details(deltas);
    }
    let opaque = kept.into_opaque("openrouter", "m").unwrap();
    serde_json::from_str(&opaque.data).unwrap()
}

#[test]
fn text_and_summary_pieces_of_one_entry_merge() {
    let details = kept(vec![
        json!([{"type": "reasoning.summary", "summary": "Plan", "id": "r1", "index": 0}]),
        json!([{"type": "reasoning.summary", "summary": " it.", "id": null, "index": 0}]),
        json!([{"type": "reasoning.text", "text": "a", "index": 1}]),
        json!([{"type": "reasoning.text", "text": "b", "index": 1, "signature": "s"}]),
    ]);
    assert_eq!(
        details,
        json!([
            {"type": "reasoning.summary", "summary": "Plan it.", "id": "r1", "index": 0},
            {"type": "reasoning.text", "text": "ab", "index": 1, "signature": "s"}
        ])
    );
}

#[test]
fn encrypted_entries_and_different_entries_never_merge() {
    let details = kept(vec![
        json!([{"type": "reasoning.encrypted", "data": "x", "index": 0}]),
        json!([{"type": "reasoning.encrypted", "data": "y", "index": 0}]),
        json!([{"type": "reasoning.text", "text": "a", "index": 1, "signature": "s1"}]),
        json!([{"type": "reasoning.text", "text": "b", "index": 1, "signature": "s2"}]),
        json!([{"type": "reasoning.text", "text": "c", "index": 2}]),
        json!(["not an entry"]),
    ]);
    assert_eq!(details.as_array().map(Vec::len), Some(5), "{details}");
    assert_eq!(details[1]["data"], "y");
}

#[test]
fn details_win_over_the_field_and_nothing_kept_is_nothing() {
    let mut kept = Kept::default();
    kept.came_in(REASONING);
    kept.add_details(vec![json!({"type": "reasoning.text", "text": "a"})]);
    let opaque = kept.into_opaque("openrouter", "m").unwrap();
    assert_eq!(opaque.kind, DETAILS);
    assert!(Kept::default().into_opaque("openrouter", "m").is_none());
}
