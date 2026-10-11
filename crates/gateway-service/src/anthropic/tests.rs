use serde_json::json;

use super::*;

fn parsed(body: &Value) -> Result<Parsed, Refusal> {
    parse(body.to_string().as_bytes())
}

#[test]
fn a_conversation_with_tools_reads_into_the_neutral_request() {
    let body = json!({
        "model": "claude-x",
        "max_tokens": 1024,
        "system": [{"type": "text", "text": "Be brief.", "cache_control": {"type": "ephemeral"}}],
        "messages": [
            {"role": "user", "content": "List the files."},
            {"role": "assistant", "content": [
                {"type": "thinking", "thinking": "hmm", "signature": "sig"},
                {"type": "text", "text": "Looking."},
                {"type": "tool_use", "id": "toolu_1", "name": "ls", "input": {"path": "."}},
            ]},
            {"role": "user", "content": [
                {"type": "tool_result", "tool_use_id": "toolu_1",
                 "content": [{"type": "text", "text": "a.txt"}], "is_error": true},
                {"type": "text", "text": "And now?"},
            ]},
        ],
        "tools": [{"name": "ls", "description": "List.", "input_schema": {"type": "object"}}],
        "tool_choice": {"type": "auto"},
        "stream": true,
        "metadata": {"user_id": "u"},
        "cache_control": {"type": "ephemeral"},
    });
    let parsed = parsed(&body).unwrap();
    assert_eq!(parsed.model, "claude-x");
    assert!(parsed.stream);
    let request = parsed.request;
    assert_eq!(request.system, "Be brief.");
    assert_eq!(request.max_output_tokens, Some(1024));
    assert_eq!(request.messages.len(), 4);
    let Message::Assistant { parts, tool_calls } = &request.messages[1] else {
        panic!("{:?}", request.messages[1]);
    };
    assert_eq!(
        parts,
        &[Part::Text {
            text: "Looking.".into()
        }],
        "thinking a client sends back is left out"
    );
    assert_eq!(tool_calls[0].tool, "ls");
    assert!(matches!(
        &request.messages[2],
        Message::Tool { call_id, is_error: true, .. } if call_id.as_str() == "toolu_1"
    ));
    assert!(matches!(&request.messages[3], Message::User { .. }));
}

#[test]
fn what_the_gateway_cannot_pass_on_is_refused_by_name() {
    let refused = |extra: Value| {
        let mut body = json!({"model": "m", "max_tokens": 10,
                              "messages": [{"role": "user", "content": "hi"}]});
        for (key, value) in extra.as_object().unwrap() {
            body[key] = value.clone();
        }
        parsed(&body).unwrap_err().message
    };
    assert!(refused(json!({"temperature": 1})).starts_with("temperature is not supported"));
    assert!(refused(json!({"thinking": {"type": "enabled"}})).starts_with("thinking"));
    assert!(refused(json!({"tool_choice": {"type": "any"}})).starts_with("tool_choice"));
    let server_tool = json!({"tools": [{"type": "web_search_20250305", "name": "web_search"}]});
    assert!(refused(server_tool).contains("/mcp"));
    let image = json!({"messages": [{"role": "user", "content": [
        {"type": "image", "source": {"type": "base64", "media_type": "image/png", "data": ""}}
    ]}]});
    assert!(refused(image).contains("blocks of type image"));
}
