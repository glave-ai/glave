//! The model a throwaway glave talks to: a stand-in on this computer, at an address the system
//! gives it (glave-ports' fake HTTP server), that answers every chat with one short sentence
//! and says each answer cost a dime, as OpenRouter's `usage.cost` says what a call cost. So
//! chats answer, and receipts show a cost, with no account, network or cost. Told to [`ASK`],
//! it calls the service's tool that asks the person something, so a person can try that card.
//! Under [`OPENROUTER`] it answers as OpenRouter too (`router.rs`), for an OpenRouter account
//! connected with any key.

use glave_ports::testing::{FakeHttp, Received};
use serde_json::json;

/// The model glave is told to use.
pub(crate) const MODEL: &str = "try-org";

/// What, said in a message, has it call [`ASKING_TOOL`].
pub(crate) const ASK: &str = "ask me";

/// The tool of [`SERVICE`] that asks the person something.
const ASKING_TOOL: &str = "mcp__Tracker__ask";

/// The name of the service it knows, as glave is told of it.
pub(crate) const SERVICE: &str = "Tracker";

/// The model a glave that keeps credits is told to use, one of OpenRouter's sample models, so
/// its catalog prices it (`router.rs`).
pub(crate) const PRICED_MODEL: &str = "deepseek/deepseek-v4.1-flash";

/// What every answer says.
const SAID: &str = "Hello from try-org's stand-in model: it answers every message with this.";

/// What every answer says it cost, in dollars.
const COST: &str = "0.10";

/// Where under the stand-in's address it answers as OpenRouter's API.
pub(crate) const OPENROUTER: &str = "/api/v1";

/// Starts the stand-in, answering until it is dropped: a chat anywhere, OpenRouter's lists
/// under [`OPENROUTER`].
pub(crate) fn start() -> Result<FakeHttp, String> {
    let chat = chat();
    FakeHttp::answering_requests(move |request| {
        let (method, path) = (request.method.as_str(), request.path.as_str());
        let listed = if method == "GET" && path.starts_with(OPENROUTER) {
            super::router::answer(path)
        } else {
            None
        };
        match listed {
            Some(listed) => (200, listed.to_string()),
            None if method == "GET" => (404, r#"{"error":{"message":"not found"}}"#.to_owned()),
            None if told_to_ask(request) => (200, asking()),
            None => (200, chat.clone()),
        }
    })
    .map_err(|err| format!("could not start the stand-in model: {err}"))
}

/// A streamed chat answer, as OpenAI's are, with its usage and what it cost.
fn chat() -> String {
    let said = json!({"id": "gen-try-org", "model": MODEL,
        "choices": [{"index": 0, "delta": {"content": SAID}}]});
    let ended = format!(
        r#"{{"choices":[{{"index":0,"delta":{{}},"finish_reason":"stop"}}],"usage":{{"prompt_tokens":100,"completion_tokens":20,"cost":{COST}}}}}"#
    );
    format!("data: {said}\n\ndata: {ended}\n\ndata: [DONE]\n\n")
}

/// Whether the last message of the chat in `request` is the person's, saying [`ASK`], with the
/// tool on offer.
fn told_to_ask(request: &Received) -> bool {
    let Ok(body) = serde_json::from_str::<serde_json::Value>(&request.body) else {
        return false;
    };
    let listed = |key: &str| body.get(key).and_then(serde_json::Value::as_array);
    let last = listed("messages").and_then(|all| all.last());
    let said = last.is_some_and(|last| {
        last.get("role").and_then(serde_json::Value::as_str) == Some("user")
            && last.to_string().to_lowercase().contains(ASK)
    });
    let named = |tool: &serde_json::Value| {
        let name = tool.pointer("/function/name");
        name.and_then(serde_json::Value::as_str) == Some(ASKING_TOOL)
    };
    said && listed("tools").is_some_and(|tools| tools.iter().any(named))
}

/// A streamed chat answer that calls [`ASKING_TOOL`].
fn asking() -> String {
    let call = json!({"id": "gen-try-org", "model": MODEL, "choices": [{"index": 0, "delta": {
        "tool_calls": [{"index": 0, "id": "call_ask", "type": "function",
            "function": {"name": ASKING_TOOL, "arguments": "{}"}}]
    }}]});
    let ended = format!(
        r#"{{"choices":[{{"index":0,"delta":{{}},"finish_reason":"tool_calls"}}],"usage":{{"prompt_tokens":100,"completion_tokens":20,"cost":{COST}}}}}"#
    );
    format!("data: {call}\n\ndata: {ended}\n\ndata: [DONE]\n\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_answer_says_its_sentence_and_what_it_cost() {
        let streamed = chat();
        let events: Vec<serde_json::Value> = streamed
            .split("\n\n")
            .filter_map(|event| event.strip_prefix("data: "))
            .filter(|data| *data != "[DONE]")
            .map(|data| serde_json::from_str(data).unwrap())
            .collect();
        assert_eq!(events[0]["choices"][0]["delta"]["content"], SAID);
        assert_eq!(events[1]["usage"]["cost"], 0.1);
        assert!(streamed.ends_with("data: [DONE]\n\n"));
    }

    fn sent(body: &serde_json::Value) -> Received {
        Received {
            method: "POST".to_owned(),
            path: "/v1/chat/completions".to_owned(),
            headers: std::collections::BTreeMap::new(),
            body: body.to_string(),
        }
    }

    #[test]
    fn only_a_message_that_says_so_has_it_call_the_tool_that_asks() {
        let tools = json!([{"function": {"name": ASKING_TOOL}}]);
        let ask = json!({"role": "user", "content": "Please ask me something"});
        let hello = json!({"role": "user", "content": "hello"});
        let result = json!({"role": "tool", "content": "ask me"});
        let chat = |last: &serde_json::Value| json!({"messages": [last], "tools": tools});
        assert!(told_to_ask(&sent(&chat(&ask))));
        assert!(!told_to_ask(&sent(&chat(&hello))));
        assert!(!told_to_ask(&sent(&chat(&result))));
        assert!(!told_to_ask(&sent(&json!({"messages": [ask]}))));
        assert!(asking().contains(ASKING_TOOL));
    }
}
