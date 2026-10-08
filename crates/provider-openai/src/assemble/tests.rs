use super::*;

fn run(provider: &str, events: &[&str]) -> (Result<Completion, ProviderError>, Vec<Delta>) {
    let mut assembler = Assembler::new(provider, "requested-model");
    let mut deltas = Vec::new();
    let mut on_delta = |delta: Delta| deltas.push(delta);
    for event in events {
        if let Err(e) = assembler.apply(event, &mut on_delta) {
            return (Err(e), deltas);
        }
    }
    (assembler.finish(), deltas)
}

#[test]
fn text_streams_live_and_assembles() {
    let (completion, deltas) = run(
        "ollama",
        &[
            r#"{"id":"chatcmpl-1","model":"gemma4:12b","choices":[{"delta":{"role":"assistant","content":"Hel"}}]}"#,
            r#"{"choices":[{"delta":{"content":"lo."}}]}"#,
            r#"{"choices":[{"delta":{},"finish_reason":"stop"}]}"#,
            r#"{"choices":[],"usage":{"prompt_tokens":12,"completion_tokens":3}}"#,
        ],
    );
    let completion = completion.unwrap();
    assert_eq!(
        deltas,
        [Delta::Text("Hel".into()), Delta::Text("lo.".into())]
    );
    assert_eq!(
        completion.parts,
        [Part::Text {
            text: "Hello.".into()
        }]
    );
    assert_eq!(completion.model.name, "gemma4:12b");
    assert_eq!(completion.response_id.as_deref(), Some("chatcmpl-1"));
    assert_eq!(completion.stop, StopReason::EndTurn);
    assert_eq!(completion.usage.counts().input, 12);
    assert_eq!(completion.usage.counts().output, 3);
    assert!(!completion.usage.estimated());
    assert_eq!(completion.cost, None);
}

#[test]
fn tool_call_fragments_merge_by_index() {
    let (completion, _) = run(
        "openai",
        &[
            r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"id":"call_a","type":"function","function":{"name":"bash","arguments":""}}]}}]}"#,
            r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"arguments":"{\"comm"}}]}}]}"#,
            r#"{"choices":[{"delta":{"tool_calls":[{"index":1,"id":"call_b","function":{"name":"read","arguments":"{\"path\":\"a.md\"}"}}]}}]}"#,
            r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"arguments":"and\":\"ls\"}"}}]}}]}"#,
            r#"{"choices":[{"delta":{},"finish_reason":"tool_calls"}]}"#,
        ],
    );
    let completion = completion.unwrap();
    assert_eq!(completion.stop, StopReason::ToolUse);
    let calls: Vec<_> = completion
        .tool_calls
        .iter()
        .map(|call| (call.provider_call_id.as_deref(), call.tool.as_str()))
        .collect();
    assert_eq!(calls, [(Some("call_a"), "bash"), (Some("call_b"), "read")]);
    let Arguments::Json(args) = &completion.tool_calls[0].arguments else {
        panic!("arguments should parse");
    };
    assert_eq!(args.get("command"), Some(&Value::from("ls")));
}

#[test]
fn malformed_arguments_are_kept_raw() {
    let (completion, _) = run(
        "local",
        &[
            r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"id":"c1","function":{"name":"bash","arguments":"{\"command\": ls"}}]},"finish_reason":"tool_calls"}]}"#,
        ],
    );
    assert_eq!(
        completion.unwrap().tool_calls[0].arguments,
        Arguments::Malformed("{\"command\": ls".into())
    );
}

#[test]
fn tool_calls_win_over_a_plain_stop() {
    let (completion, _) = run(
        "local",
        &[
            r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"id":"c1","function":{"name":"bash","arguments":"{}"}}]},"finish_reason":"stop"}]}"#,
        ],
    );
    let completion = completion.unwrap();
    assert_eq!(completion.stop, StopReason::ToolUse);
    assert_eq!(completion.provider_stop.as_deref(), Some("stop"));
}

#[test]
fn reasoning_is_its_own_part_before_the_text() {
    let (completion, deltas) = run(
        "vllm",
        &[
            r#"{"choices":[{"delta":{"reasoning_content":"Think"}}]}"#,
            r#"{"choices":[{"delta":{"reasoning_content":"ing."}}]}"#,
            r#"{"choices":[{"delta":{"content":"Answer."},"finish_reason":"stop"}]}"#,
        ],
    );
    assert_eq!(deltas.first(), Some(&Delta::Reasoning("Think".into())));
    assert_eq!(
        completion.unwrap().parts,
        [
            Part::Reasoning {
                text: Some("Thinking.".into()),
                opaque: None
            },
            Part::Text {
                text: "Answer.".into()
            },
        ]
    );
}

#[test]
fn usage_normalizes_and_reported_cost_is_exact() {
    let (completion, _) = run(
        "openrouter",
        &[
            r#"{"choices":[{"delta":{"content":"ok"},"finish_reason":"stop"}]}"#,
            r#"{"choices":[],"usage":{"prompt_tokens":43003,"completion_tokens":120,"prompt_tokens_details":{"cached_tokens":40000,"cache_write_tokens":3000},"completion_tokens_details":{"reasoning_tokens":20},"cost":0.0042}}"#,
        ],
    );
    let completion = completion.unwrap();
    let counts = completion.usage.counts();
    assert_eq!(
        (
            counts.input,
            counts.cache_read,
            counts.cache_write,
            counts.output,
            counts.reasoning
        ),
        (43_003, 40_000, 3_000, 120, 20)
    );
    let cost = completion.cost.unwrap();
    assert_eq!(cost.amount.micros, 4_200);
    assert_eq!(cost.price_list, "openrouter:reported");
    assert!(completion.usage.raw().is_some());
}

#[test]
fn inconsistent_usage_is_clamped_not_rejected() {
    let (completion, _) = run(
        "odd",
        &[
            r#"{"choices":[{"delta":{"content":"x"},"finish_reason":"stop"}],"usage":{"prompt_tokens":10,"completion_tokens":2,"prompt_tokens_details":{"cached_tokens":50},"completion_tokens_details":{"reasoning_tokens":9}}}"#,
        ],
    );
    let counts = completion.unwrap().usage.counts();
    assert_eq!((counts.cache_read, counts.reasoning), (10, 2));
}

#[test]
fn a_stream_without_a_finish_is_interrupted_and_estimated() {
    let (completion, _) = run(
        "local",
        &[r#"{"choices":[{"delta":{"content":"partial"}}]}"#],
    );
    let completion = completion.unwrap();
    assert_eq!(completion.stop, StopReason::Interrupted);
    assert!(completion.usage.estimated());
    assert_eq!(
        completion.parts,
        [Part::Text {
            text: "partial".into()
        }]
    );
}

#[test]
fn an_error_mid_stream_fails_the_request() {
    let (completion, _) = run(
        "openrouter",
        &[r#"{"error":{"code":502,"message":"upstream failed"}}"#],
    );
    assert!(matches!(completion, Err(ProviderError::Protocol(_))));
}

#[test]
fn decimals_convert_to_micros_exactly() {
    for (text, micros) in [
        ("0.0042", Some(4_200)),
        ("1", Some(1_000_000)),
        ("12.5", Some(12_500_000)),
        ("0.0000005", Some(1)),
        ("0.0000004", Some(0)),
        ("-0.25", Some(-250_000)),
        ("1e-5", None),
        ("", None),
        (".5", None),
    ] {
        assert_eq!(decimal_to_micros(text), micros, "{text}");
    }
}
