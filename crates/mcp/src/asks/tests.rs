use rmcp::model::ServerResult;
use serde_json::json;

use super::*;

fn form_of(schema: &Value, message: &str) -> Option<Form> {
    let params = json!({"mode": "form", "message": message, "requestedSchema": schema});
    form("k", serde_json::from_value(params).unwrap()).ok()
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
        ("passWord", "x", "x"),
        ("accesstoken", "x", "x"),
        ("authtoken", "x", "x"),
        ("clientsecret", "x", "x"),
        ("sshkey", "x", "x"),
        ("password1", "x", "x"),
        ("pass", "x", "x"),
        ("pin", "x", "x"),
        ("cvv", "x", "x"),
        ("value", "Verification code", "x"),
        ("Passwort", "x", "x"),
        ("contraseña", "x", "x"),
        ("contrasen\u{303}a", "x", "x"),
        ("pass\u{200b}word", "x", "x"),
        ("openai_key", "x", "x"),
        ("value", "Stripe key", "x"),
        ("value", "GitHub key", "x"),
        ("openaikey", "x", "x"),
        ("githubtoken", "x", "x"),
        ("TOKEN1", "x", "x"),
        ("PIN1", "x", "x"),
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
        ("spinner", "Spinner", "x"),
        ("issueKey", "x", "x"),
        ("sort_key", "x", "x"),
        ("value", "Primary key", "x"),
        ("broken", "Broken", "x"),
        ("compass", "Compass", "x"),
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
fn a_yes_or_no_may_say_what_only_typing_would_give_away() {
    let yes_no = |name: &str| json!({"type": "object", "properties": {name: {"type": "boolean"}}});
    assert!(form_of(&yes_no("pass"), "Did the tests go well?").is_some());
    assert!(form_of(&yes_no("pin"), "Keep it at the top?").is_some());
    assert!(form_of(&yes_no("password"), "Rotate it?").is_none());
}

#[test]
fn what_could_rewrite_a_terminal_or_hide_letters_is_gone_before_anyone_shows_it() {
    assert_eq!(clean("a\u{1b}[2Kb\nc\u{200b}d\u{202e}e"), "a[2Kb cde");
    assert_eq!(clean("  two\r\n\tlines\u{2028}here "), "two lines here");
    let schema = json!({"type": "object", "properties": {
        "one": {"type": "string", "title": "\u{1b}[1mBold", "description": "x\ny",
            "oneOf": [{"const": "a\u{7}", "title": "A\u{7}y"}], "default": "a\u{7}"},
        "words": {"type": "string", "default": "d\u{1b}e\nf"},
    }});
    let form = form_of(&schema, "Ask\nglave: all is well\u{1b}[2K").unwrap();
    assert_eq!(form.message, "Ask glave: all is well[2K");
    let one = form
        .fields
        .iter()
        .find(|field| field.name == "one")
        .unwrap();
    assert_eq!(one.label, "[1mBold");
    assert_eq!(one.description.as_deref(), Some("x y"));
    assert_eq!(
        one.kind,
        ServiceFieldKind::Choice {
            // What goes back is as the server sent it.
            options: vec![ServiceOption {
                value: "a\u{7}".to_owned(),
                label: "Ay".to_owned(),
            }],
            multiple: false,
            default: vec!["a\u{7}".to_owned()],
        }
    );
    let words = form
        .fields
        .iter()
        .find(|field| field.name == "words")
        .unwrap();
    assert_eq!(
        words.kind,
        ServiceFieldKind::Text {
            default: Some("de f".to_owned())
        }
    );
    assert!(
        form_of(&schema, "\u{1b}\u{7}").is_none(),
        "nothing left to show"
    );
}

#[test]
fn a_field_name_with_anything_unseen_in_it_is_refused() {
    let schema = json!({"type": "object", "required": ["a\u{1b}[2Kb"], "properties": {
        "a\u{1b}[2Kb": {"type": "string", "title": "Fine", "default": "\n"},
    }});
    assert!(form_of(&schema, "m").is_none());
}

#[test]
fn a_default_with_nothing_left_in_it_is_none() {
    let schema = json!({"type": "object", "required": ["words"], "properties": {
        "words": {"type": "string", "default": "\n\u{1b}"},
    }});
    let form = form_of(&schema, "m").unwrap();
    assert_eq!(
        form.fields.first().map(|field| &field.kind),
        Some(&ServiceFieldKind::Text { default: None })
    );
}

#[test]
fn a_note_is_told_even_where_the_output_opens_with_no_text() {
    let asking = Asking {
        notes: BTreeSet::from([Note::Declined]),
        ..Asking::default()
    };
    let told = asking.told(
        "github",
        ToolOutput {
            parts: Vec::new(),
            ..ToolOutput::text(false, "")
        },
    );
    let [Part::Text { text }] = told.parts.as_slice() else {
        panic!("not one text part");
    };
    assert!(text.contains("they declined"), "{text}");
}

#[test]
fn a_default_is_one_only_if_it_fits() {
    let schema = json!({"type": "object", "properties": {
        "one": {"type": "string", "enum": ["a", "b"], "default": "c"},
        "many": {"type": "array", "items": {"type": "string", "enum": ["x"]},
            "default": ["x", "y"]},
    }});
    let form = form_of(&schema, "m").unwrap();
    let default_of = |name: &str| {
        let field = form.fields.iter().find(|field| field.name == name).unwrap();
        match &field.kind {
            ServiceFieldKind::Choice { default, .. } => default.clone(),
            other => panic!("{other:?}"),
        }
    };
    assert_eq!(default_of("one"), Vec::<String>::new());
    assert_eq!(default_of("many"), ["x"]);
    let long = "y".repeat(LONGEST_WORDS + 1);
    let schema =
        json!({"type": "object", "properties": {"t": {"type": "string", "default": long}}});
    assert!(form_of(&schema, "m").is_none());
}

#[test]
fn a_service_named_as_glave_or_the_model_does_not_ask() {
    for name in [
        "glave",
        "Glave",
        "the model",
        "Model",
        "GLAVE ",
        "gla\u{200b}ve",
        "a\u{1b}b",
        "",
    ] {
        assert!(!may_ask(name), "{name:?}");
    }
    for name in ["GitHub", "tests", "My tracker", "glave-docs"] {
        assert!(may_ask(name), "{name:?}");
    }
}

#[test]
fn each_refusal_says_its_own_reason() {
    assert!(Note::Refused.said("x").contains("such as a password"));
    assert!(!Note::TooMuch.said("x").contains("password"));
    assert!(!Note::Page.said("x").contains("password"));
    assert!(!Note::Name.said("x").contains("password"));
}

#[test]
fn a_page_to_open_is_refused() {
    let params = json!({
        "mode": "url", "message": "Open this", "url": "https://example.com/a",
        "elicitationId": "e1"
    });
    let refused = form("k", serde_json::from_value(params).unwrap()).err();
    assert_eq!(refused, Some(Note::Page));
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
    // Compared without printing: a failure must not show what was typed.
    let same = asking.responses.get("k") == Some(&json!({"action": "accept", "content": values}));
    assert!(same);
    let told = asking.told("github", ToolOutput::text(false, "done"));
    let [Part::Text { text }] = told.parts.as_slice() else {
        panic!("not one text part");
    };
    assert!(!text.contains("what they typed"));
    assert!(text.starts_with("[github asked the person"), "{text}");
    assert!(text.ends_with("\ndone"), "{text}");
}
