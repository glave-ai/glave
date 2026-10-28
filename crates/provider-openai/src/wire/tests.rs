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
        purpose: glave_ports::Purpose::Main,
        effort: None,
        files: Vec::new(),
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
        purpose: glave_ports::Purpose::Main,
        effort: None,
        files: Vec::new(),
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
fn no_request_turns_on_a_provider_s_own_web_search() {
    // Every search is a call of a connection the person turned on (specs/session.md): never
    // OpenRouter's `:online` or `web` plugin, nor OpenAI's own search.
    let request = ModelRequest {
        model: "openai/gpt-6-luna".into(),
        system: String::new(),
        messages: Vec::new(),
        tools: vec![ToolSpec {
            name: "web_fetch".into(),
            description: "Fetch a web page.".into(),
            parameters: json!({"type": "object"}),
        }],
        tool_choice: ToolChoice::Auto,
        max_output_tokens: None,
        purpose: glave_ports::Purpose::Main,
        effort: Some("high".into()),
        files: Vec::new(),
    };
    for provider in ["openrouter", "openai", "ollama"] {
        let sent = serde_json::to_value(chat_request(&request, provider)).unwrap();
        assert_eq!(sent["model"], "openai/gpt-6-luna", "no :online");
        let tools = sent["tools"].as_array().unwrap();
        assert!(
            tools.iter().all(|tool| tool["type"] == "function"),
            "{sent}"
        );
        let plugins = sent["plugins"].as_array().cloned().unwrap_or_default();
        assert!(plugins.iter().all(|plugin| plugin["id"] != "web"), "{sent}");
        let text = sent.to_string();
        assert!(
            !text.contains("web_search") && !text.contains(":online"),
            "{text}"
        );
    }
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
        purpose: glave_ports::Purpose::Main,
        effort: None,
        files: Vec::new(),
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
        purpose: glave_ports::Purpose::Main,
        effort: None,
        files: Vec::new(),
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
        purpose: glave_ports::Purpose::Main,
        effort: None,
        files: Vec::new(),
    };
    let routed = serde_json::to_value(chat_request(&request, "openrouter")).unwrap();
    assert_eq!(
        routed["plugins"],
        json!([{"id": "context-compression", "enabled": false}])
    );
    let plain = serde_json::to_value(chat_request(&request, "openai")).unwrap();
    assert!(plain.get("plugins").is_none());
}

#[test]
fn openrouter_keeps_prompts_private_and_every_parameter_used() {
    let request = ModelRequest {
        model: "openai/gpt-6-luna".into(),
        system: String::new(),
        messages: Vec::new(),
        tools: Vec::new(),
        tool_choice: ToolChoice::Auto,
        max_output_tokens: None,
        purpose: glave_ports::Purpose::Main,
        effort: None,
        files: Vec::new(),
    };
    let routed = serde_json::to_value(chat_request(&request, "openrouter")).unwrap();
    assert_eq!(
        routed["provider"],
        json!({"data_collection": "deny", "require_parameters": true})
    );
    assert!(
        routed.get("reasoning").is_none(),
        "no level: the model's own"
    );
    let plain = serde_json::to_value(chat_request(&request, "openai")).unwrap();
    assert!(plain.get("provider").is_none());
}

#[test]
fn a_reasoning_level_is_sent_as_each_service_takes_it() {
    let request = ModelRequest {
        model: "m".into(),
        system: String::new(),
        messages: Vec::new(),
        tools: Vec::new(),
        tool_choice: ToolChoice::Auto,
        max_output_tokens: None,
        purpose: glave_ports::Purpose::Main,
        effort: Some("high".into()),
        files: Vec::new(),
    };
    let routed = serde_json::to_value(chat_request(&request, "openrouter")).unwrap();
    assert_eq!(routed["reasoning"], json!({"effort": "high"}));
    assert!(routed.get("reasoning_effort").is_none());
    let openai = serde_json::to_value(chat_request(&request, "openai")).unwrap();
    assert_eq!(openai["reasoning_effort"], "high");
    assert!(openai.get("reasoning").is_none());
}

/// A file part for `blob`, named `name`.
fn file(blob: &glave_event::BlobRef, name: &str) -> Part {
    Part::File {
        blob: blob.clone(),
        name: Some(name.into()),
        resized: None,
    }
}

/// A blob of `media_type` holding `bytes`, and its content inline: `data`, in base64.
fn inline(bytes: &[u8], media_type: &str, data: &str) -> (glave_event::BlobRef, InlineFile) {
    let blob = glave_event::BlobRef {
        sha256: glave_ports::digest(bytes),
        bytes: bytes.len() as u64,
        media_type: media_type.into(),
    };
    let file = InlineFile {
        sha256: blob.sha256.clone(),
        media_type: media_type.into(),
        data: data.into(),
    };
    (blob, file)
}

fn asking(messages: Vec<Message>, files: Vec<InlineFile>) -> ModelRequest {
    ModelRequest {
        model: "gpt".into(),
        system: String::new(),
        messages,
        tools: Vec::new(),
        tool_choice: ToolChoice::Auto,
        max_output_tokens: None,
        purpose: glave_ports::Purpose::Main,
        effort: None,
        files,
    }
}

#[test]
fn an_image_and_a_pdf_are_sent_inline_beside_the_text_and_a_file_not_inline_by_name() {
    let (image, image_inline) = inline(b"png", "image/png", "cG5n");
    let (pdf, pdf_inline) = inline(b"%PDF", "application/pdf", "JVBERg==");
    let (other, _) = inline(b"old", "application/pdf", "b2xk");
    let request = asking(
        vec![
            Message::User {
                parts: vec![Part::Text {
                    text: "earlier".into(),
                }],
            },
            Message::User {
                parts: vec![
                    Part::Text {
                        text: "what is this?".into(),
                    },
                    file(&image, "shot.png"),
                    file(&pdf, "terms.pdf"),
                    file(&other, "old.pdf"),
                ],
            },
        ],
        vec![image_inline, pdf_inline],
    );
    let body = serde_json::to_value(chat_request(&request, "openai")).unwrap();
    assert_eq!(
        body["messages"],
        json!([{"role": "user", "content": [
            {"type": "text", "text": "earlier"},
            {"type": "text", "text": "what is this?"},
            {"type": "image_url", "image_url": {"url": "data:image/png;base64,cG5n"}},
            {"type": "file", "file": {"filename": "terms.pdf", "file_data": "data:application/pdf;base64,JVBERg=="}},
            {"type": "text", "text": "old.pdf"}
        ]}])
    );
}

#[test]
fn what_tool_results_hand_the_model_follows_them_as_the_persons() {
    let (image, image_inline) = inline(b"png", "image/png", "cG5n");
    let (pdf, pdf_inline) = inline(b"%PDF", "application/pdf", "JVBERg==");
    let call = |id: &str| ToolCall {
        call_id: CallId::new(id).unwrap(),
        tool: "read".into(),
        arguments: Arguments::Json(serde_json::Map::new()),
    };
    let result = |id: &str, output: Vec<Part>| Message::Tool {
        call_id: CallId::new(id).unwrap(),
        is_error: false,
        output,
    };
    let said = Part::Text {
        text: "shot.png is an image of 4x3 pixels.".into(),
    };
    let request = asking(
        vec![
            Message::Assistant {
                parts: Vec::new(),
                tool_calls: vec![call("call_1"), call("call_2")],
            },
            result("call_1", vec![said.clone(), file(&image, "shot.png")]),
            result(
                "call_2",
                vec![
                    Part::Text {
                        text: "notes".into(),
                    },
                    file(&pdf, "terms.pdf"),
                ],
            ),
        ],
        vec![image_inline, pdf_inline],
    );
    let body = serde_json::to_value(chat_request(&request, "openai")).unwrap();
    let messages = body["messages"].as_array().unwrap();
    assert_eq!(
        messages[1],
        json!({"role": "tool", "tool_call_id": "call_1",
            "content": "shot.png is an image of 4x3 pixels.\nshot.png"})
    );
    assert_eq!(messages[2]["role"], "tool", "every result comes first");
    assert_eq!(
        messages[3],
        json!({"role": "user", "content": [
            {"type": "text", "text": "shot.png, from tool result call_1:"},
            {"type": "image_url", "image_url": {"url": "data:image/png;base64,cG5n"}},
            {"type": "text", "text": "terms.pdf, from tool result call_2:"},
            {"type": "file", "file": {"filename": "terms.pdf", "file_data": "data:application/pdf;base64,JVBERg=="}}
        ]})
    );
    assert_eq!(messages.len(), 4);
}

/// A tool call's assistant message whose reasoning came from `provider` and `model` in `kind`.
fn reasoned(provider: &str, model: &str, kind: &str, data: &str) -> Message {
    Message::Assistant {
        parts: vec![
            Part::Reasoning {
                text: Some("Look first.".into()),
                opaque: Some(glave_event::Opaque {
                    provider: provider.into(),
                    model: model.into(),
                    kind: kind.into(),
                    data: data.into(),
                }),
            },
            Part::Text {
                text: "Listing.".into(),
            },
        ],
        tool_calls: Vec::new(),
    }
}

fn assistant_sent(message: Message, provider: &str) -> Value {
    let request = asking(vec![message], Vec::new());
    let body = serde_json::to_value(chat_request(&request, provider)).unwrap();
    body["messages"][0].clone()
}

#[test]
fn reasoning_goes_back_in_the_field_it_came_in() {
    let content = assistant_sent(
        reasoned("deepseek", "gpt", "reasoning_content", ""),
        "deepseek",
    );
    assert_eq!(
        content,
        json!({"role": "assistant", "content": "Listing.", "reasoning_content": "Look first."})
    );
    let plain = assistant_sent(reasoned("ollama", "gpt", "reasoning", ""), "ollama");
    assert_eq!(plain["reasoning"], "Look first.");
    assert!(plain.get("reasoning_content").is_none());
}

#[test]
fn reasoning_details_go_back_exactly_as_kept() {
    let kept = r#"[{"type":"reasoning.text","text":"Look first.","signature":"c2ln","index":0},{"type":"reasoning.encrypted","data":"ZW5j","id":null}]"#;
    let request = asking(
        vec![reasoned("openrouter", "gpt", "reasoning_details", kept)],
        Vec::new(),
    );
    let body = serde_json::to_string(&chat_request(&request, "openrouter")).unwrap();
    assert!(
        body.contains(&format!(r#""reasoning_details":{kept}"#)),
        "{body}"
    );
    assert!(!body.contains(r#""reasoning":"Look first.""#), "{body}");
}

#[test]
fn reasoning_from_another_provider_or_model_is_not_sent() {
    for (provider, model) in [("openrouter", "gpt"), ("deepseek", "other")] {
        let sent = assistant_sent(
            reasoned(provider, model, "reasoning_content", ""),
            "deepseek",
        );
        assert_eq!(
            sent,
            json!({"role": "assistant", "content": "Listing."}),
            "from {provider} {model}"
        );
    }
}
