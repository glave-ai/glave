use glave_event::{Arguments, ToolCallRequested, ToolKind};
use serde_json::json;

use super::*;
use crate::plan::tests::event;

fn requested(call: &str, tool: &str, path: &str) -> Body {
    let serde_json::Value::Object(map) = json!({"path": path, "content": "x"}) else {
        panic!("an object");
    };
    Body::ToolCallRequested(ToolCallRequested {
        call_id: CallId::new(call).unwrap(),
        provider_call_id: None,
        tool: tool.to_owned(),
        arguments: Arguments::Json(map),
        summary: String::new(),
        kind: ToolKind::Other,
    })
}

fn completed(call: &str, is_error: bool) -> Body {
    Body::ToolCallCompleted(ToolCallCompleted {
        call_id: CallId::new(call).unwrap(),
        is_error,
        output: Vec::new(),
        structured: None,
        untrusted: false,
    })
}

#[test]
fn a_turn_read_from_its_log_changed_the_files_its_writes_and_edits_changed_once_each() {
    let events: Vec<Event> = [
        requested("a", "write", "notes.md"),
        completed("a", false),
        requested("b", "edit", "main.rs"),
        completed("b", true),
        requested("c", "bash", "made.txt"),
        completed("c", false),
        requested("d", "edit", "notes.md"),
        completed("d", false),
        requested("e", "edit", "lib.rs"),
    ]
    .into_iter()
    .zip(1..)
    .map(|(body, seq)| event(seq, body))
    .collect();
    assert_eq!(
        own_tallies(&events).changed,
        ["notes.md"],
        "not an edit that failed, a command, or a call with no result yet"
    );
}
