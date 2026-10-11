use serde_json::json;

use super::*;

fn parsed(body: &Value) -> Result<Parsed, Refusal> {
    parse(body.to_string().as_bytes())
}

#[test]
fn a_conversation_with_tools_reads_into_the_neutral_request() {
    let body = json!({
        "model": "main",
        "messages": [
            {"role": "system", "content": "Be brief."},
            {"role": "developer", "content": [{"type": "text", "text": "Use tools."}]},
            {"role": "user", "content": "List the files."},
            {"role": "assistant", "content": null, "refusal": null, "tool_calls": [
                {"id": "call_1", "type": "function",
                 "function": {"name": "ls", "arguments": "{\"path\":\".\"}"}}
            ]},
            {"role": "tool", "tool_call_id": "call_1", "content": "a.txt"},
        ],
        "tools": [{"type": "function", "function": {
            "name": "ls", "description": "List.", "parameters": {"type": "object"}}}],
        "tool_choice": "auto",
        "max_completion_tokens": 512,
        "stream": true,
        "stream_options": {"include_usage": true},
        "user": "someone", "metadata": {"k": "v"}, "store": false, "n": 1,
        "parallel_tool_calls": true,
    });
    let parsed = parsed(&body).unwrap();
    assert_eq!(parsed.model, "main");
    assert!(parsed.stream && parsed.usage);
    let request = parsed.request;
    assert_eq!(request.system, "Be brief.\n\nUse tools.");
    assert_eq!(request.max_output_tokens, Some(512));
    assert_eq!(request.tools[0].name, "ls");
    assert_eq!(request.messages.len(), 3);
    let Message::Assistant { tool_calls, parts } = &request.messages[1] else {
        panic!("{:?}", request.messages[1]);
    };
    assert_eq!(parts.as_slice(), []);
    assert_eq!(tool_calls[0].call_id.as_str(), "call_1");
    assert!(matches!(&tool_calls[0].arguments, Arguments::Json(map) if map["path"] == "."));
    assert!(
        matches!(&request.messages[2], Message::Tool { call_id, .. } if call_id.as_str() == "call_1")
    );
}

#[test]
fn what_the_gateway_cannot_pass_on_is_refused_by_name() {
    let base = || json!({"model": "m", "messages": [{"role": "user", "content": "hi"}]});
    let refused = |extra: Value| {
        let mut body = base();
        for (key, value) in extra.as_object().unwrap() {
            body[key] = value.clone();
        }
        parsed(&body).unwrap_err().message
    };
    assert!(refused(json!({"temperature": 0.2})).starts_with("temperature is not supported"));
    assert!(refused(json!({"n": 2})).starts_with("n is not supported"));
    assert!(refused(json!({"tool_choice": "required"})).starts_with("tool_choice"));
    assert!(
        refused(json!({"response_format": {"type": "json_object"}})).contains("response_format")
    );
    let image = json!({"messages": [{"role": "user", "content": [
        {"type": "image_url", "image_url": {"url": "https://example.com/a.png"}}
    ]}]});
    assert!(refused(image).contains("image_url"));
    assert!(
        parsed(&json!({"messages": []}))
            .unwrap_err()
            .message
            .contains("model")
    );
    assert!(parsed(&base()).is_ok());
}
