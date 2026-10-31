use rmcp::model::ServerResult;
use serde_json::json;

use super::*;

fn form_of(schema: &Value, message: &str) -> Option<Form> {
    let params = json!({"mode": "form", "message": message, "requestedSchema": schema});
    form("k", serde_json::from_value(params).unwrap())
}

fn text_field(name: &str, title: &str, description: &str) -> Value {
    json!({"type": "object", "properties": {
        name: {"type": "string", "title": title, "description": description}
    }})
}

#[test]
fn a_field_that_asks_for_a_secret_is_refused() {
    for (name, title, description) in [
        ("password", "x", "x"),
        ("pass", "Password", "x"),
        ("value", "x", "Your GitHub password"),
        ("apiKey", "x", "x"),
        ("api_key", "x", "x"),
        ("key", "x", "x"),
        ("value", "API key", "x"),
        ("value", "x", "the SSH key to use"),
        ("clientSecret", "x", "x"),
        ("value", "Access token", "x"),
        ("value", "x", "your credentials"),
    ] {
        assert!(
            form_of(&text_field(name, title, description), "Which?").is_none(),
            "{name} {title} {description}"
        );
    }
}

#[test]
fn a_field_that_only_sounds_near_a_secret_is_not() {
    for (name, title, description) in [
        ("repository", "Repository", "Where it goes"),
        ("issue", "Issue key", "Such as ABC-12"),
        ("keyword", "Keyword", "What to search for"),
        ("tokenizer", "Tokenizer", "Which to use"),
        ("monkey", "Monkey", "x"),
    ] {
        assert!(
            form_of(&text_field(name, title, description), "Which?").is_some(),
            "{name} {title} {description}"
        );
    }
}

#[test]
fn a_message_that_asks_for_a_secret_is_refused_where_it_could_be_typed() {
    let typed = text_field("value", "Value", "x");
    assert!(form_of(&typed, "Please enter your password").is_none());
    assert!(form_of(&typed, "Which label?").is_some());
    let yes_no = json!({"type": "object", "properties": {"go": {"type": "boolean"}}});
    assert!(
        form_of(&yes_no, "Rotate the token?").is_some(),
        "a yes or no gives nothing away"
    );
}

#[test]
fn a_page_to_open_is_refused() {
    let params = json!({
        "mode": "url", "message": "Open this", "url": "https://example.com/a",
        "elicitationId": "e1"
    });
    assert!(form("k", serde_json::from_value(params).unwrap()).is_none());
}

#[test]
fn too_much_to_show_is_refused() {
    assert!(form_of(&text_field("a", "A", "x"), &"y".repeat(LONGEST_MESSAGE + 1)).is_none());
    assert!(form_of(&text_field("a", &"y".repeat(LONGEST_WORDS + 1), "x"), "m").is_none());
    let many: serde_json::Map<String, Value> = (0..=MOST_FIELDS)
        .map(|at| (format!("f{at}"), json!({"type": "boolean"})))
        .collect();
    let schema = json!({"type": "object", "properties": many});
    assert!(form_of(&schema, "m").is_none());
}

#[test]
fn every_kind_of_field_reads_as_glaves() {
    let schema = json!({"type": "object", "required": ["one"], "properties": {
        "one": {"type": "string", "title": "One", "oneOf": [
            {"const": "a", "title": "Ay"}, {"const": "b", "title": "Bee"}
        ], "default": "b"},
        "many": {"type": "array", "items": {"type": "string", "enum": ["x", "y"]}},
        "words": {"type": "string", "default": "hi"},
        "whole": {"type": "integer", "default": 2},
        "part": {"type": "number"},
        "sure": {"type": "boolean", "default": true},
    }});
    let form = form_of(&schema, "m").unwrap();
    let kinds: Vec<(&str, &ServiceFieldKind, bool)> = form
        .fields
        .iter()
        .map(|field| (field.name.as_str(), &field.kind, field.required))
        .collect();
    // A map read back from `json!` is sorted; a server's own order is kept from the wire.
    assert_eq!(
        kinds,
        [
            (
                "many",
                &ServiceFieldKind::Choice {
                    options: vec![option("x", "x"), option("y", "y")],
                    multiple: true,
                    default: Vec::new(),
                },
                false
            ),
            (
                "one",
                &ServiceFieldKind::Choice {
                    options: vec![option("a", "Ay"), option("b", "Bee")],
                    multiple: false,
                    default: vec!["b".to_owned()],
                },
                true
            ),
            (
                "part",
                &ServiceFieldKind::Number {
                    integer: false,
                    default: None,
                },
                false
            ),
            (
                "sure",
                &ServiceFieldKind::YesNo {
                    default: Some(true)
                },
                false
            ),
            (
                "whole",
                &ServiceFieldKind::Number {
                    integer: true,
                    default: Some(Number::from(2)),
                },
                false
            ),
            (
                "words",
                &ServiceFieldKind::Text {
                    default: Some("hi".to_owned())
                },
                false
            ),
        ]
    );
    let one = form
        .fields
        .iter()
        .find(|field| field.name == "one")
        .unwrap();
    assert_eq!(one.label, "One");
}

/// A round with a field of a format MCP does not know, as rmcp reads the whole result.
fn with_a_password_format() -> Value {
    json!({
        "resultType": "input_required",
        "requestState": "kept",
        "inputRequests": {
            "secret": {"method": "elicitation/create", "params": {
                "mode": "form", "message": "Your password", "requestedSchema": {
                    "type": "object",
                    "properties": {"p": {"type": "string", "format": "password"}}
                }
            }},
            "fine": {"method": "elicitation/create", "params": {
                "mode": "form", "message": "Which?", "requestedSchema": {
                    "type": "object", "properties": {"go": {"type": "boolean"}}
                }
            }}
        }
    })
}

#[test]
fn a_form_rmcp_cannot_read_is_refused_and_the_others_still_asked() {
    let result: ServerResult = serde_json::from_value(with_a_password_format()).unwrap();
    let ServerResult::CustomResult(custom) = result else {
        panic!("rmcp read it as {result:?}");
    };
    let round = InputRound::unread(&custom.0).unwrap();
    assert!(matches!(
        round.requests.get("secret"),
        Some(Asked::Unreadable)
    ));
    assert!(matches!(
        round.requests.get("fine"),
        Some(Asked::Readable(_))
    ));
    assert_eq!(round.state.as_deref(), Some("kept"));
    assert!(InputRound::unread(&json!({"resultType": "complete"})).is_none());
    assert!(InputRound::unread(&json!({"anything": 1})).is_none());
}

#[test]
fn the_model_is_told_what_came_of_it_never_what_was_answered() {
    let mut asking = Asking {
        forms: VecDeque::from([Form {
            key: "k".to_owned(),
            message: "Which?".to_owned(),
            fields: Vec::new(),
        }]),
        ..Asking::default()
    };
    let values = json!({"label": "what they typed"});
    asking
        .given(ServiceAnswer::Answered {
            values: values.as_object().unwrap().clone(),
        })
        .unwrap();
    assert!(asking.forms.is_empty());
    assert_eq!(
        asking.responses.get("k"),
        Some(&json!({"action": "accept", "content": values}))
    );
    let told = asking.told("github", ToolOutput::text(false, "done"));
    let [Part::Text { text }] = told.parts.as_slice() else {
        panic!("{told:?}");
    };
    assert!(text.starts_with("[github asked the person"), "{text}");
    assert!(text.ends_with("\ndone"), "{text}");
    assert!(!text.contains("what they typed"), "{text}");
}
