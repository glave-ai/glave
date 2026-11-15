//! The wire format's shape, checked as JSON.

use super::*;
use glave_event::{CallId, MaxPrice, Nanos};
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
        payer: None,
        route: None,
        session: None,
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
fn the_longest_answer_is_named_as_each_service_takes_it() {
    let mut request = ModelRequest {
        model: "m".into(),
        system: String::new(),
        messages: Vec::new(),
        tools: Vec::new(),
        tool_choice: ToolChoice::Auto,
        max_output_tokens: Some(32_000),
        purpose: glave_ports::Purpose::Main,
        effort: None,
        files: Vec::new(),
        payer: None,
        route: None,
    };
    let field = |request: &ModelRequest, provider: &str| {
        let body = serde_json::to_value(chat_request(request, provider)).unwrap();
        let named = |name: &str| body.get(name).and_then(Value::as_u64);
        (named("max_tokens"), named("max_completion_tokens"))
    };
    // OpenAI's reasoning models refuse `max_tokens`; Azure OpenAI's too.
    assert_eq!(field(&request, "openai"), (None, Some(32_000)));
    let azure = "acme.openai.azure.com";
    assert_eq!(field(&request, azure), (None, Some(32_000)));
    // OpenRouter and the servers that copy OpenAI take `max_tokens`.
    for provider in ["openrouter", "local", "ollama", "api.deepseek.com"] {
        assert_eq!(
            field(&request, provider),
            (Some(32_000), None),
            "{provider}"
        );
    }
    request.max_output_tokens = None;
    assert_eq!(field(&request, "openai"), (None, None));
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
        payer: None,
        route: None,
        session: None,
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
        tools: vec![ToolSpec {
            name: "web_fetch".into(),
            description: "Fetch a web page.".into(),
            parameters: json!({"type": "object"}),
        }],
        effort: Some("high".into()),
        ..ask("openai/gpt-6-luna")
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
        tools: vec![ToolSpec {
            name: "bash".into(),
            description: "Run a command.".into(),
            parameters: json!({"type": "object"}),
        }],
        tool_choice: ToolChoice::None,
        ..ask("m")
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
    let request = ask("anthropic/claude-haiku-5.5");
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
    let request = ask("openai/gpt-6-luna");
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
    let request = ask("openai/gpt-6-luna");
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
fn a_person_s_filters_go_to_openrouter_alone_with_what_every_request_asks() {
    let route = Route {
        only: Some(vec!["groq".into(), "fireworks".into()]),
        ignore: vec!["deepseek".into()],
        quantizations: vec!["fp8".into(), "bf16".into()],
        sort: Some(Sort::Speed),
        zdr: true,
        max_price: Some(MaxPrice {
            input: Some(Nanos::new(150_000_000)),
            output: Some(Nanos::new(2_000_000_000)),
        }),
    };
    let request = ModelRequest {
        route: Some(route),
        ..ask("deepseek/deepseek-v4.1-flash")
    };
    let routed = serde_json::to_value(chat_request(&request, "openrouter")).unwrap();
    assert_eq!(
        routed["provider"],
        json!({
            "data_collection": "deny",
            "require_parameters": true,
            "only": ["groq", "fireworks"],
            "ignore": ["deepseek"],
            "quantizations": ["fp8", "bf16"],
            "sort": "throughput",
            "zdr": true,
            "max_price": {"prompt": 0.15, "completion": 2.0},
        })
    );
    let plain = serde_json::to_value(chat_request(&request, "openai")).unwrap();
    assert!(
        plain.get("provider").is_none(),
        "only a router is sent them"
    );
}

#[test]
fn openrouter_alone_is_told_the_thread_and_the_prompt_stays_as_it_was() {
    let thread = "019a8f2e-1c3b-7d4e-8f5a-1b2c3d4e5f60";
    let mut request = asking(vec![user("hello")], Vec::new());
    request.system = "Be brief.".into();
    let alone = serde_json::to_value(chat_request(&request, "openrouter")).unwrap();
    assert!(alone.get("session_id").is_none(), "no thread, no session");
    request.session = Some(serde_json::from_value(json!(thread)).unwrap());
    let mut routed = serde_json::to_value(chat_request(&request, "openrouter")).unwrap();
    assert_eq!(routed["session_id"], json!(thread));
    assert!(routed.get("provider").unwrap().get("order").is_none());
    let body = routed.as_object_mut().unwrap();
    body.remove("session_id");
    assert_eq!(
        routed, alone,
        "everything else, the prompt above all, as it was"
    );
    let plain = serde_json::to_value(chat_request(&request, "openai")).unwrap();
    assert!(plain.get("session_id").is_none(), "only a router is told");
}

#[test]
fn a_reasoning_level_is_sent_as_each_service_takes_it() {
    let request = ModelRequest {
        effort: Some("high".into()),
        ..ask("m")
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

/// A request to `model` with nothing in it.
fn ask(model: &str) -> ModelRequest {
    ModelRequest {
        model: model.into(),
        ..asking(Vec::new(), Vec::new())
    }
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
        payer: None,
        route: None,
        session: None,
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

fn user(text: &str) -> Message {
    Message::User {
        parts: vec![Part::Text { text: text.into() }],
    }
}

/// A tool call's assistant message with no reasoning of its own.
fn unreasoned(call_id: &str) -> Message {
    Message::Assistant {
        parts: Vec::new(),
        tool_calls: vec![ToolCall {
            call_id: CallId::new(call_id).unwrap(),
            tool: "bash".into(),
            arguments: Arguments::Json(json!({"command": "ls"}).as_object().unwrap().clone()),
        }],
    }
}

fn result_of(call_id: &str) -> Message {
    Message::Tool {
        call_id: CallId::new(call_id).unwrap(),
        is_error: false,
        output: vec![Part::Text { text: "ok".into() }],
    }
}

/// The body sent to `provider` for `model` with `messages`.
fn body_for(provider: &str, model: &str, messages: Vec<Message>) -> Value {
    let request = ModelRequest {
        model: model.into(),
        ..asking(messages, Vec::new())
    };
    serde_json::to_value(chat_request(&request, provider)).unwrap()
}

#[test]
fn reasoning_goes_back_within_the_turn_and_every_turn_where_the_maker_demands() {
    let carried = |provider: &str, model: &str| {
        let body = body_for(
            provider,
            model,
            vec![
                user("first"),
                reasoned(provider, model, "reasoning_content", ""),
                user("second"),
                reasoned(provider, model, "reasoning_content", ""),
                reasoned(provider, model, "reasoning_content", ""),
            ],
        );
        body["messages"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|message| message["role"] == "assistant")
            .map(|message| message.get("reasoning_content").is_some())
            .collect::<Vec<_>>()
    };
    assert_eq!(carried("openrouter", "gpt"), [false, true, true]);
    for (provider, model) in [
        ("api.deepseek.com", "deepseek-reasoner"),
        ("openrouter", "deepseek/deepseek-v4.1-flash"),
        ("api.deepinfra.com", "deepseek-ai/DeepSeek-V4.1-Flash"),
    ] {
        assert_eq!(carried(provider, model), [true, true, true], "{model}");
    }
}

#[test]
fn every_reply_to_deepseek_carries_reasoning_empty_where_it_had_none() {
    let model = "deepseek/deepseek-v4.1-flash";
    let body = body_for(
        "openrouter",
        model,
        vec![
            user("first"),
            unreasoned("call_1"),
            result_of("call_1"),
            reasoned("anthropic", "claude", "reasoning_content", ""),
            user("second"),
            reasoned("openrouter", model, "reasoning_content", ""),
        ],
    );
    let sent: Vec<&Value> = body["messages"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|message| message["role"] == "assistant")
        .map(|message| &message["reasoning_content"])
        .collect();
    assert_eq!(sent, [&json!(""), &json!(""), &json!("Look first.")]);
    let other = body_for("openrouter", "gpt", vec![unreasoned("call_1")]);
    assert!(other["messages"][0].get("reasoning_content").is_none());
}

#[test]
fn deepseek_is_sent_its_makers_sampler_and_no_other_model_any() {
    for (provider, model) in [
        ("openrouter", "deepseek/deepseek-v4.1-flash"),
        ("api.deepinfra.com", "deepseek-ai/DeepSeek-V4.1-Flash"),
    ] {
        let body = body_for(provider, model, Vec::new());
        assert_eq!(body["temperature"], json!(1.0), "{model}");
        assert_eq!(body["top_p"], json!(0.95), "{model}");
        let text = serde_json::to_string(&body).unwrap();
        assert!(!text.contains("penalty"), "{text}");
    }
    for (provider, model) in [
        ("openrouter", "anthropic/claude-haiku-5.5"),
        ("openai", "gpt-6-luna"),
        ("openrouter", "openai/gpt-6-luna"),
        ("ollama", "gemma-4-12b"),
    ] {
        let body = body_for(provider, model, Vec::new());
        assert!(body.get("temperature").is_none(), "{model}");
        assert!(body.get("top_p").is_none(), "{model}");
    }
}

#[test]
fn a_deepseek_conversation_is_sent_again_byte_for_byte_as_it_grows() {
    let model = "deepseek/deepseek-v4.1-flash";
    let first = vec![
        user("first"),
        reasoned("openrouter", model, "reasoning_content", ""),
        unreasoned("call_1"),
        result_of("call_1"),
    ];
    let mut later = first.clone();
    later.extend([
        reasoned("openrouter", model, "reasoning_content", ""),
        user("second"),
        unreasoned("call_2"),
        result_of("call_2"),
    ]);
    let sent = |messages: Vec<Message>| {
        let body = body_for("openrouter", model, messages);
        let messages = body["messages"].as_array().unwrap().clone();
        let without = |key: &str| {
            let mut body = body.clone();
            body.as_object_mut().unwrap().remove(key);
            body
        };
        (messages, without("messages"))
    };
    let (before, rest_before) = sent(first);
    let (after, rest_after) = sent(later);
    let prefix: Vec<String> = before.iter().map(Value::to_string).collect();
    let grown: Vec<String> = after.iter().map(Value::to_string).collect();
    assert_eq!(grown.get(..prefix.len()), Some(prefix.as_slice()));
    assert_eq!(rest_before, rest_after, "the same parameters on every call");
}

#[test]
fn the_most_a_provider_may_cost_is_sent_exact_to_the_last_place() {
    let route = Route {
        max_price: Some(MaxPrice {
            input: Some(Nanos::new(1)),
            output: None,
        }),
        ..Route::default()
    };
    let sent = serde_json::to_string(&Preferences::of(Some(&route)).max_price).unwrap();
    assert_eq!(sent, r#"{"prompt":0.000000001}"#);
}
