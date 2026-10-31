//! The model a throwaway glave talks to: a stand-in on this computer, at an address the system
//! gives it (glave-ports' fake HTTP server), that answers every chat with one short sentence
//! and says each answer cost a dime, as OpenRouter's `usage.cost` says what a call cost. So
//! chats answer, and receipts show a cost, with no account, network or cost. Under
//! [`OPENROUTER`] it answers as OpenRouter too (`router.rs`), for an OpenRouter account
//! connected with any key.

use glave_ports::testing::FakeHttp;
use serde_json::json;

/// The model glave is told to use.
pub(crate) const MODEL: &str = "try-org";

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
    FakeHttp::answering(move |method, path| {
        let listed = if method == "GET" && path.starts_with(OPENROUTER) {
            super::router::answer(path)
        } else {
            None
        };
        match listed {
            Some(listed) => (200, listed.to_string()),
            None if method == "GET" => (404, r#"{"error":{"message":"not found"}}"#.to_owned()),
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
}
