use std::fmt::Write as _;

use serde_json::json;

use super::*;
use crate::sse::SseDecoder;

fn run(events: &[Value]) -> (Result<Completion, ProviderError>, Vec<Delta>) {
    let mut assembler = Assembler::new("anthropic", "claude-haiku-5-5");
    let mut deltas = Vec::new();
    let mut on_delta = |delta: Delta| deltas.push(delta);
    for event in events {
        if let Err(e) = assembler.apply(&event.to_string(), &mut on_delta) {
            return (Err(e), deltas);
        }
    }
    (assembler.finish(), deltas)
}

fn start(usage: Value) -> Value {
    let mut event = json!({"type": "message_start", "message": {"id": "msg_1", "type": "message", "role": "assistant",
        "model": "claude-haiku-5-5-20261001", "content": []}});
    event["message"]["usage"] = usage;
    event
}

fn block(index: u32, block: Value) -> Value {
    let mut event = json!({"type": "content_block_start", "index": index});
    event["content_block"] = block;
    event
}

fn delta(index: u32, delta: Value) -> Value {
    let mut event = json!({"type": "content_block_delta", "index": index});
    event["delta"] = delta;
    event
}

fn text(index: u32, text: &str) -> Value {
    delta(index, json!({"type": "text_delta", "text": text}))
}

fn stop(index: u32) -> Value {
    json!({"type": "content_block_stop", "index": index})
}

fn end(reason: &str, output: u64) -> Vec<Value> {
    vec![
        json!({"type": "message_delta", "delta": {"stop_reason": reason, "stop_sequence": null}, "usage": {"output_tokens": output}}),
        json!({"type": "message_stop"}),
    ]
}

fn usage() -> Value {
    json!({"input_tokens": 12, "cache_read_input_tokens": 0, "cache_creation_input_tokens": 0, "output_tokens": 1})
}

/// A response with thinking, text, two tool calls and every kind of event.
fn full_response() -> Vec<Value> {
    let mut events = vec![
        start(usage()),
        block(
            0,
            json!({"type": "thinking", "thinking": "", "signature": ""}),
        ),
        delta(
            0,
            json!({"type": "thinking_delta", "thinking": "Check the "}),
        ),
        json!({"type": "ping"}),
        delta(0, json!({"type": "thinking_delta", "thinking": "files."})),
        delta(0, json!({"type": "signature_delta", "signature": "sig-1"})),
        stop(0),
        block(
            1,
            json!({"type": "redacted_thinking", "data": "redacted-1"}),
        ),
        stop(1),
        block(2, json!({"type": "text", "text": ""})),
        text(2, "Looking. "),
        text(2, "Héllo 漢🙂"),
        stop(2),
        block(
            3,
            json!({"type": "tool_use", "id": "toolu_a", "name": "bash", "input": {}}),
        ),
        delta(
            3,
            json!({"type": "input_json_delta", "partial_json": "{\"comm"}),
        ),
        delta(
            3,
            json!({"type": "input_json_delta", "partial_json": "and\": \"ls\"}"}),
        ),
        stop(3),
        block(
            4,
            json!({"type": "tool_use", "id": "toolu_b", "name": "read", "input": {}}),
        ),
        delta(4, json!({"type": "input_json_delta", "partial_json": ""})),
        stop(4),
    ];
    events.extend(end("tool_use", 40));
    events
}

#[test]
fn text_streams_live_and_assembles() {
    let mut events = vec![
        start(usage()),
        block(0, json!({"type": "text", "text": ""})),
    ];
    events.extend([text(0, "Hel"), text(0, "lo."), stop(0)]);
    events.extend(end("end_turn", 3));
    let (completion, deltas) = run(&events);
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
    assert_eq!(completion.model.provider, "anthropic");
    assert_eq!(completion.model.name, "claude-haiku-5-5-20261001");
    assert_eq!(completion.response_id.as_deref(), Some("msg_1"));
    assert_eq!(completion.stop, StopReason::EndTurn);
    assert_eq!(completion.provider_stop.as_deref(), Some("end_turn"));
    assert_eq!(completion.usage.counts().input, 12);
    assert_eq!(completion.usage.counts().output, 3);
    assert!(!completion.usage.estimated());
    assert_eq!(completion.cost, None, "Anthropic reports no cost");
}

#[test]
fn thinking_is_kept_with_what_it_takes_to_send_it_back() {
    let (completion, deltas) = run(&full_response());
    let completion = completion.unwrap();
    assert_eq!(
        deltas[..2],
        [
            Delta::Reasoning("Check the ".into()),
            Delta::Reasoning("files.".into())
        ]
    );
    let opaque = |kind: &str, data: &str| {
        Some(Opaque {
            provider: "anthropic".into(),
            // The model as asked for, which the next request names the same way.
            model: "claude-haiku-5-5".into(),
            kind: kind.into(),
            data: data.into(),
        })
    };
    assert_eq!(
        completion.parts,
        [
            Part::Reasoning {
                text: Some("Check the files.".into()),
                opaque: opaque(SIGNATURE, "sig-1"),
            },
            Part::Reasoning {
                text: None,
                opaque: opaque(REDACTED_THINKING, "redacted-1"),
            },
            Part::Text {
                text: "Looking. Héllo 漢🙂".into()
            },
        ]
    );
}

#[test]
fn tool_calls_assemble_with_their_ids() {
    let (completion, _) = run(&full_response());
    let completion = completion.unwrap();
    assert_eq!(completion.stop, StopReason::ToolUse);
    let calls: Vec<_> = completion
        .tool_calls
        .iter()
        .map(|call| (call.provider_call_id.as_deref(), call.tool.as_str()))
        .collect();
    assert_eq!(
        calls,
        [(Some("toolu_a"), "bash"), (Some("toolu_b"), "read")]
    );
    let Arguments::Json(args) = &completion.tool_calls[0].arguments else {
        panic!("arguments should parse");
    };
    assert_eq!(args.get("command"), Some(&Value::from("ls")));
    assert_eq!(
        completion.tool_calls[1].arguments,
        Arguments::Json(Map::new()),
        "no input streamed is an empty object"
    );
}

#[test]
fn tool_input_split_anywhere_assembles_the_same() {
    let input = r#"{"path": "notes/é漢.md", "lines": [1, 2], "deep": {"x": null}}"#;
    let expected: Map<String, Value> = serde_json::from_str(input).unwrap();
    let cuts: Vec<usize> = (0..=input.len())
        .filter(|cut| input.is_char_boundary(*cut))
        .collect();
    for cut in cuts {
        let (head, tail) = input.split_at(cut);
        let mut events = vec![
            start(usage()),
            block(
                0,
                json!({"type": "tool_use", "id": "toolu_1", "name": "write", "input": {}}),
            ),
        ];
        for piece in [head, tail] {
            events.push(delta(
                0,
                json!({"type": "input_json_delta", "partial_json": piece}),
            ));
        }
        events.extend(end("tool_use", 5));
        let completion = run(&events).0.unwrap();
        assert_eq!(
            completion.tool_calls[0].arguments,
            Arguments::Json(expected.clone()),
            "cut at {cut}"
        );
    }
}

#[test]
fn input_that_is_not_an_object_is_kept_raw() {
    let mut events = vec![
        start(usage()),
        block(
            0,
            json!({"type": "tool_use", "id": "toolu_1", "name": "bash", "input": {}}),
        ),
        delta(
            0,
            json!({"type": "input_json_delta", "partial_json": "{\"command\": ls"}),
        ),
    ];
    events.extend(end("max_tokens", 5));
    let completion = run(&events).0.unwrap();
    assert_eq!(
        completion.tool_calls[0].arguments,
        Arguments::Malformed("{\"command\": ls".into())
    );
    assert_eq!(completion.stop, StopReason::MaxTokens);
}

#[test]
fn usage_is_normalized_to_inclusive_totals() {
    let mut events = vec![start(json!({
        "input_tokens": 3, "cache_read_input_tokens": 40_000,
        "cache_creation_input_tokens": 3_000, "output_tokens": 1
    }))];
    events.extend(end("end_turn", 120));
    let completion = run(&events).0.unwrap();
    let counts = completion.usage.counts();
    assert_eq!(
        (
            counts.input,
            counts.cache_read,
            counts.cache_write,
            counts.output,
            counts.reasoning
        ),
        (43_003, 40_000, 3_000, 120, 0)
    );
    let raw = completion.usage.raw().unwrap();
    assert_eq!(
        raw["output_tokens"], 120,
        "the final count replaces the first"
    );
    assert_eq!(raw["cache_read_input_tokens"], 40_000);
}

#[test]
fn stop_reasons_map_and_keep_their_raw_value() {
    for (raw, stop) in [
        ("end_turn", StopReason::EndTurn),
        ("tool_use", StopReason::ToolUse),
        ("max_tokens", StopReason::MaxTokens),
        ("stop_sequence", StopReason::StopSequence),
        ("refusal", StopReason::Refusal),
        ("pause_turn", StopReason::Pause),
        ("model_context_window_exceeded", StopReason::ContextExceeded),
        ("something_new", StopReason::Error),
    ] {
        let mut events = vec![start(usage())];
        events.extend(end(raw, 1));
        let completion = run(&events).0.unwrap();
        assert_eq!(completion.stop, stop, "{raw}");
        assert_eq!(completion.provider_stop.as_deref(), Some(raw));
    }
}

#[test]
fn a_stream_without_a_stop_is_interrupted_and_estimated() {
    let events = [
        start(usage()),
        block(0, json!({"type": "text", "text": ""})),
        text(0, "partial"),
    ];
    let completion = run(&events).0.unwrap();
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
    let events = [
        start(usage()),
        json!({"type": "error", "error": {"type": "overloaded_error", "message": "Overloaded"}}),
    ];
    let error = run(&events).0.unwrap_err();
    assert_eq!(
        error,
        ProviderError::Http {
            status: 529,
            body: "Overloaded".into()
        }
    );
    assert!(error.is_transient());
}

#[test]
fn unknown_events_and_blocks_are_skipped_but_broken_ones_fail() {
    let mut events = vec![
        start(usage()),
        json!({"type": "brand_new_event", "x": 1}),
        block(
            0,
            json!({"type": "server_tool_use", "id": "srvtoolu_1", "name": "web_search"}),
        ),
        delta(0, json!({"type": "input_json_delta", "partial_json": "{}"})),
        block(1, json!({"type": "text", "text": ""})),
        delta(1, json!({"type": "citations_delta", "citation": {}})),
        text(1, "ok"),
    ];
    events.extend(end("end_turn", 2));
    let completion = run(&events).0.unwrap();
    assert_eq!(completion.parts, [Part::Text { text: "ok".into() }]);
    assert_eq!(completion.tool_calls, []);

    let never_started = run(&[start(usage()), text(5, "x")]).0;
    assert!(matches!(never_started, Err(ProviderError::Protocol(_))));
    let wrong_kind = run(&[
        start(usage()),
        block(0, json!({"type": "text", "text": ""})),
        delta(0, json!({"type": "input_json_delta", "partial_json": "{}"})),
    ])
    .0;
    assert!(matches!(wrong_kind, Err(ProviderError::Protocol(_))));
    let unreadable = Assembler::new("anthropic", "m").apply("not json", &mut |_| {});
    assert!(matches!(unreadable, Err(ProviderError::Protocol(_))));
}

/// The response as the wire carries it, with event names.
fn wire_bytes(events: &[Value]) -> Vec<u8> {
    let mut stream = String::new();
    for event in events {
        let name = event["type"].as_str().unwrap();
        let _ = write!(stream, "event: {name}\ndata: {event}\n\n");
    }
    stream.into_bytes()
}

/// Decodes `chunks` and assembles them, as the client does.
fn assemble_chunks(chunks: &[&[u8]]) -> (Completion, Vec<Delta>) {
    let mut decoder = SseDecoder::default();
    let mut assembler = Assembler::new("anthropic", "claude-haiku-5-5");
    let mut deltas = Vec::new();
    let mut on_delta = |delta: Delta| deltas.push(delta);
    for chunk in chunks {
        for data in decoder.push(chunk).unwrap() {
            assembler.apply(&data, &mut on_delta).unwrap();
        }
    }
    if let Some(data) = decoder.finish() {
        assembler.apply(&data, &mut on_delta).unwrap();
    }
    (assembler.finish().unwrap(), deltas)
}

proptest::proptest! {
    /// However the network splits a response into chunks, including inside a multibyte
    /// character, it assembles to the same answer.
    #[test]
    fn any_chunking_assembles_the_same(
        cuts in proptest::collection::vec(proptest::prelude::any::<usize>(), 0..16),
    ) {
        let bytes = wire_bytes(&full_response());
        let whole = assemble_chunks(&[&bytes]);
        let mut points: Vec<usize> = cuts.iter().map(|cut| cut % (bytes.len() + 1)).collect();
        points.sort_unstable();
        let mut chunks = Vec::new();
        let mut from = 0;
        for point in points {
            chunks.push(&bytes[from..point]);
            from = point;
        }
        chunks.push(&bytes[from..]);
        let (completion, deltas) = assemble_chunks(&chunks);
        proptest::prop_assert_eq!(&completion, &whole.0);
        proptest::prop_assert_eq!(&deltas, &whole.1);
    }
}
