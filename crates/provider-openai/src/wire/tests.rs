//! The wire format's shape, checked as JSON.

use super::*;
use glave_event::CallId;
use glave_ports::ToolCall;
use serde_json::json;

#[test]
fn a_request_has_the_chat_completions_shape() {
    let request = ModelRequest {
        model: "gemma-4-12b".into(),
        system: "Be brief.".into(),
        messages: vec![
            Message::User {
                parts: vec![Part::Text {
                    text: "list files".into(),
                }],
            },
            Message::Assistant {
                parts: vec![Part::Reasoning {
                    text: Some("think".into()),
                    opaque: None,
                }],
                tool_calls: vec![ToolCall {
                    call_id: CallId::new("call_4").unwrap(),
                    tool: "bash".into(),
                    arguments: Arguments::Json(
                        json!({"command": "ls"}).as_object().unwrap().clone(),
                    ),
                }],
            },
            Message::Tool {
                call_id: CallId::new("call_4").unwrap(),
                is_error: false,
                output: vec![Part::Text {
                    text: "README.md".into(),
                }],
            },
        ],
        tools: vec![ToolSpec {
            name: "bash".into(),
            description: "Run a command.".into(),
            parameters: json!({"type": "object"}),
        }],
        tool_choice: ToolChoice::Auto,
        max_output_tokens: Some(1024),
    };
    let body = serde_json::to_value(chat_request(&request, "ollama")).unwrap();
    assert_eq!(
        body,
        json!({
            "model": "gemma-4-12b",
            "messages": [
                {"role": "system", "content": "Be brief."},
                {"role": "user", "content": "list files"},
                {"role": "assistant", "content": null, "tool_calls": [
                    {"id": "call_4", "type": "function",
                     "function": {"name": "bash", "arguments": "{\"command\":\"ls\"}"}}
                ]},
                {"role": "tool", "tool_call_id": "call_4", "content": "README.md"}
            ],
            "tools": [{"type": "function", "function": {
                "name": "bash", "description": "Run a command.", "parameters": {"type": "object"}
            }}],
            "stream": true,
            "stream_options": {"include_usage": true},
            "max_tokens": 1024
        })
    );
}

#[test]
fn consecutive_user_messages_are_sent_as_one() {
    let user = |text: &str| Message::User {
        parts: vec![Part::Text { text: text.into() }],
    };
    let request = ModelRequest {
        model: "gemma-4-12b".into(),
        system: String::new(),
        messages: vec![
            user("summary"),
            user("[cancelled]"),
            user("next"),
            Message::Assistant {
                parts: vec![Part::Text { text: "ok".into() }],
                tool_calls: Vec::new(),
            },
            user("last"),
        ],
        tools: Vec::new(),
        tool_choice: ToolChoice::Auto,
        max_output_tokens: None,
    };
    let body = serde_json::to_value(chat_request(&request, "ollama")).unwrap();
    assert_eq!(
        body["messages"],
        json!([
            {"role": "user", "content": "summary\n\n[cancelled]\n\nnext"},
            {"role": "assistant", "content": "ok"},
            {"role": "user", "content": "last"}
        ])
    );
}

#[test]
fn text_only_requests_keep_their_tools_and_say_none() {
    let mut request = ModelRequest {
        model: "m".into(),
        system: String::new(),
        messages: Vec::new(),
        tools: vec![ToolSpec {
            name: "bash".into(),
            description: "Run a command.".into(),
            parameters: json!({"type": "object"}),
        }],
        tool_choice: ToolChoice::None,
        max_output_tokens: None,
    };
    let body = serde_json::to_value(chat_request(&request, "ollama")).unwrap();
    assert_eq!(body["tool_choice"], "none");
    assert_eq!(body["tools"].as_array().map(Vec::len), Some(1));
    request.tool_choice = ToolChoice::Auto;
    let body = serde_json::to_value(chat_request(&request, "ollama")).unwrap();
    assert!(
        body.get("tool_choice").is_none(),
        "the default stays unsaid"
    );
}

#[test]
fn automatic_caching_is_asked_for_only_where_it_works() {
    assert!(caches_automatically(
        "openrouter",
        "anthropic/claude-haiku-5.5"
    ));
    assert!(!caches_automatically("openrouter", "openai/gpt-6.1-sol"));
    assert!(!caches_automatically("openrouter", "google/gemma-4-31b-it"));
    assert!(!caches_automatically(
        "anthropic",
        "anthropic/claude-haiku-5.5"
    ));
    assert!(!caches_automatically("ollama", "gemma4:12b"));
    let request = ModelRequest {
        model: "anthropic/claude-haiku-5.5".into(),
        system: String::new(),
        messages: Vec::new(),
        tools: Vec::new(),
        tool_choice: ToolChoice::Auto,
        max_output_tokens: None,
    };
    let cached = serde_json::to_value(chat_request(&request, "openrouter")).unwrap();
    assert_eq!(cached["cache_control"], json!({"type": "ephemeral"}));
    let plain = serde_json::to_value(chat_request(&request, "ollama")).unwrap();
    assert!(
        plain.get("cache_control").is_none(),
        "other servers get today's request"
    );
}

#[test]
fn openrouter_is_told_not_to_cut_a_long_prompt() {
    let request = ModelRequest {
        model: "openai/gpt-6-luna".into(),
        system: String::new(),
        messages: Vec::new(),
        tools: Vec::new(),
        tool_choice: ToolChoice::Auto,
        max_output_tokens: None,
    };
    let routed = serde_json::to_value(chat_request(&request, "openrouter")).unwrap();
    assert_eq!(
        routed["plugins"],
        json!([{"id": "context-compression", "enabled": false}])
    );
    let plain = serde_json::to_value(chat_request(&request, "openai")).unwrap();
    assert!(plain.get("plugins").is_none());
}
