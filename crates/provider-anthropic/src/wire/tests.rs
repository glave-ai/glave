//! The request's shape, checked as JSON.

use super::*;
use glave_event::{CallId, Opaque};
use glave_ports::ToolCall;
use serde_json::json;

fn user(text: &str) -> Message {
    Message::User {
        parts: vec![Part::Text { text: text.into() }],
    }
}

fn call(id: &str, arguments: Arguments) -> ToolCall {
    ToolCall {
        call_id: CallId::new(id).unwrap(),
        tool: "bash".into(),
        arguments,
    }
}

fn result(id: &str, text: &str, is_error: bool) -> Message {
    Message::Tool {
        call_id: CallId::new(id).unwrap(),
        is_error,
        output: vec![Part::Text { text: text.into() }],
    }
}

fn bash() -> ToolSpec {
    ToolSpec {
        name: "bash".into(),
        description: "Run a command.".into(),
        parameters: json!({"type": "object"}),
    }
}

fn request(messages: Vec<Message>) -> ModelRequest {
    ModelRequest {
        model: "claude-haiku-5-5".into(),
        system: String::new(),
        messages,
        tools: Vec::new(),
        tool_choice: ToolChoice::Auto,
        max_output_tokens: None,
        purpose: glave_ports::Purpose::Main,
        threads: Vec::new(),
    }
}

fn body(request: &ModelRequest) -> Value {
    serde_json::to_value(messages_request(request, "anthropic", Api::Messages)).unwrap()
}

fn thinking(provider: &str, model: &str, kind: &str, text: Option<&str>) -> Part {
    Part::Reasoning {
        text: text.map(str::to_owned),
        opaque: Some(Opaque {
            provider: provider.into(),
            model: model.into(),
            kind: kind.into(),
            data: format!("{kind}-data"),
        }),
    }
}

#[test]
fn a_request_has_the_messages_shape() {
    let mut request = request(vec![
        user("list files"),
        Message::Assistant {
            parts: vec![Part::Text {
                text: "Listing.".into(),
            }],
            tool_calls: vec![call(
                "call_4",
                Arguments::Json(json!({"command": "ls"}).as_object().unwrap().clone()),
            )],
        },
        result("call_4", "README.md", false),
    ]);
    request.system = "Be brief.".into();
    request.tools = vec![bash()];
    request.max_output_tokens = Some(1024);
    assert_eq!(
        body(&request),
        json!({
            "model": "claude-haiku-5-5",
            "max_tokens": 1024,
            "system": "Be brief.",
            "messages": [
                {"role": "user", "content": [{"type": "text", "text": "list files"}]},
                {"role": "assistant", "content": [
                    {"type": "text", "text": "Listing."},
                    {"type": "tool_use", "id": "call_4", "name": "bash", "input": {"command": "ls"}}
                ]},
                {"role": "user", "content": [
                    {"type": "tool_result", "tool_use_id": "call_4", "content": "README.md", "is_error": false}
                ]}
            ],
            "tools": [{"name": "bash", "description": "Run a command.", "input_schema": {"type": "object"}}],
            "cache_control": {"type": "ephemeral"},
            "stream": true
        })
    );
}

#[test]
fn defaults_fill_what_the_api_requires_and_leave_out_the_rest() {
    let body = body(&request(vec![user("hi")]));
    assert_eq!(body["max_tokens"], DEFAULT_MAX_TOKENS);
    for absent in ["system", "tools", "tool_choice", "thinking", "temperature"] {
        assert!(body.get(absent).is_none(), "{absent} is not sent");
    }
    assert_eq!(body["cache_control"], json!({"type": "ephemeral"}));
}

#[test]
fn all_results_of_a_turn_go_in_one_user_message_after_it() {
    let body = body(&request(vec![
        user("check"),
        Message::Assistant {
            parts: Vec::new(),
            tool_calls: vec![
                call("call_1", Arguments::Json(Map::new())),
                call("call_2", Arguments::Malformed("{\"command\": ls".into())),
            ],
        },
        result("call_1", "ok", false),
        result("call_2", "not JSON", true),
        user("[cancelled]"),
    ]));
    assert_eq!(
        body["messages"],
        json!([
            {"role": "user", "content": [{"type": "text", "text": "check"}]},
            {"role": "assistant", "content": [
                {"type": "tool_use", "id": "call_1", "name": "bash", "input": {}},
                {"type": "tool_use", "id": "call_2", "name": "bash", "input": {}}
            ]},
            {"role": "user", "content": [
                {"type": "tool_result", "tool_use_id": "call_1", "content": "ok", "is_error": false},
                {"type": "tool_result", "tool_use_id": "call_2", "content": "not JSON", "is_error": true},
                {"type": "text", "text": "[cancelled]"}
            ]}
        ])
    );
}

#[test]
fn consecutive_messages_of_one_role_merge_and_empty_ones_vanish() {
    let body = body(&request(vec![
        user("summary"),
        user(""),
        user("next"),
        Message::Assistant {
            parts: vec![Part::Text { text: "ok".into() }],
            tool_calls: Vec::new(),
        },
        Message::Assistant {
            parts: Vec::new(),
            tool_calls: Vec::new(),
        },
        user("last"),
    ]));
    assert_eq!(
        body["messages"],
        json!([
            {"role": "user", "content": [{"type": "text", "text": "summary"}, {"type": "text", "text": "next"}]},
            {"role": "assistant", "content": [{"type": "text", "text": "ok"}]},
            {"role": "user", "content": [{"type": "text", "text": "last"}]}
        ])
    );
}

#[test]
fn thinking_from_this_provider_and_model_goes_back_first_and_unchanged() {
    let body = body(&request(vec![
        user("why?"),
        Message::Assistant {
            parts: vec![
                thinking(
                    "anthropic",
                    "claude-haiku-5-5",
                    SIGNATURE,
                    Some("Let me see."),
                ),
                Part::Text {
                    text: "Because.".into(),
                },
                thinking("anthropic", "claude-haiku-5-5", REDACTED_THINKING, None),
                thinking("anthropic", "claude-haiku-5-5", SIGNATURE, None),
            ],
            tool_calls: vec![call("call_1", Arguments::Json(Map::new()))],
        },
    ]));
    assert_eq!(
        body["messages"][1]["content"],
        json!([
            {"type": "thinking", "thinking": "Let me see.", "signature": "signature-data"},
            {"type": "redacted_thinking", "data": "redacted_thinking-data"},
            {"type": "thinking", "thinking": "", "signature": "signature-data"},
            {"type": "text", "text": "Because."},
            {"type": "tool_use", "id": "call_1", "name": "bash", "input": {}}
        ])
    );
}

#[test]
fn thinking_from_elsewhere_is_left_out() {
    let body = body(&request(vec![
        user("why?"),
        Message::Assistant {
            parts: vec![
                thinking("openrouter", "claude-haiku-5-5", SIGNATURE, Some("a")),
                thinking("anthropic", "claude-opus-5-5", SIGNATURE, Some("b")),
                thinking("anthropic", "claude-haiku-5-5", "encrypted", Some("c")),
                Part::Reasoning {
                    text: Some("plain".into()),
                    opaque: None,
                },
                Part::Text {
                    text: "Because.".into(),
                },
            ],
            tool_calls: Vec::new(),
        },
    ]));
    assert_eq!(
        body["messages"][1]["content"],
        json!([{"type": "text", "text": "Because."}])
    );
}

#[test]
fn text_only_requests_keep_their_tools_and_say_none() {
    let mut request = request(vec![user("sum up")]);
    request.tools = vec![bash()];
    request.tool_choice = ToolChoice::None;
    let none = body(&request);
    assert_eq!(none["tool_choice"], json!({"type": "none"}));
    assert_eq!(none["tools"].as_array().map(Vec::len), Some(1));
    request.tool_choice = ToolChoice::Auto;
    assert!(
        body(&request).get("tool_choice").is_none(),
        "the default stays unsaid"
    );
}

#[test]
fn vertex_takes_its_version_in_the_body_and_the_model_in_the_url() {
    let request = request(vec![user("hi")]);
    let vertex = serde_json::to_value(messages_request(&request, "vertex", Api::Vertex)).unwrap();
    assert_eq!(vertex["anthropic_version"], "vertex-2023-10-16");
    assert!(vertex.get("model").is_none(), "{vertex}");
    let mut messages = vertex.clone();
    if let Some(fields) = messages.as_object_mut() {
        fields.remove("anthropic_version");
        fields.insert("model".into(), json!("claude-haiku-5-5"));
    }
    assert_eq!(messages, body(&request), "otherwise the same request");
    assert!(body(&request).get("anthropic_version").is_none());
}
