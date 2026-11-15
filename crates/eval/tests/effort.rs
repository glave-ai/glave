//! `--effort` through the gateway: a level asked goes out as OpenRouter's `reasoning.effort` to
//! a model whose listing offers it, a model that offers none is sent none, and the attempt counts
//! the level each call went at and the reasoning it reported. Without it, nothing is sent.

use glave_eval::{Attempt, Effort, attempt};
use glave_ports::block_on;
use glave_ports::testing::{FakeHttp, any_secret};
use glave_provider_openai::Config;
use serde_json::{Value, json};

use crate::attempt::{folder, keyed_at, task};

/// A model whose listing offers three levels.
const THINKS: &str = "vendor/thinks";

/// A model whose listing offers no `high`, reasoning at `medium` unasked.
const MILD: &str = "vendor/mild";

/// A model whose listing says nothing of reasoning.
const PLAIN: &str = "vendor/plain";

/// A stand-in for OpenRouter: its model list, and a chat that answers with 420 reasoning
/// tokens.
fn openrouter() -> FakeHttp {
    let models = json!({"data": [
        {"id": THINKS, "reasoning": {"mandatory": false, "default_enabled": true,
            "supported_efforts": ["high", "medium", "low"], "default_effort": "medium"}},
        {"id": MILD, "reasoning": {"mandatory": false, "default_enabled": true,
            "supported_efforts": ["medium", "low"], "default_effort": "medium"}},
        {"id": PLAIN},
    ]});
    let said = json!({"id": "gen-1", "model": THINKS,
        "choices": [{"index": 0, "delta": {"content": "Hi."}}]});
    let ended = json!({"choices": [{"index": 0, "delta": {}, "finish_reason": "stop"}],
        "usage": {"prompt_tokens": 100, "completion_tokens": 500,
            "completion_tokens_details": {"reasoning_tokens": 420}}});
    let chat = format!("data: {said}\n\ndata: {ended}\n\ndata: [DONE]\n\n");
    FakeHttp::answering(move |method, path| match (method, path) {
        ("GET", "/api/v1/models") => (200, models.to_string()),
        ("GET", _) => (404, "{}".to_owned()),
        _ => (200, chat.clone()),
    })
    .unwrap()
}

/// One attempt of `model` at the stand-in, asking `effort`: the attempt, and the first chat
/// request it sent.
fn attempted(model: &str, effort: Option<&str>) -> (Attempt, Value) {
    let dir = folder(&format!("effort-{}", effort.unwrap_or("unasked")));
    let task = task(&dir, "", "exit 0\n");
    let router = openrouter();
    let mut config = Config::for_url(format!("{}/api/v1", router.url), Some(any_secret())).unwrap();
    "openrouter".clone_into(&mut config.provider);
    let models = [model.to_owned()];
    let effort = effort.map(|level| block_on!(Effort::listed(&config, level, &models)).unwrap());
    let mut setup = keyed_at(config, &any_secret());
    setup.effort = effort;
    let done = block_on!(attempt(&setup, &task, model, 1, &dir.join("run"))).unwrap();
    assert!(done.passed(), "{:?}", done.failures);
    let chat = (router.received().into_iter())
        .find(|request| request.method == "POST")
        .map(|request| serde_json::from_str(&request.body).unwrap())
        .unwrap();
    (done, chat)
}

#[test]
fn a_level_asked_goes_out_as_reasoning_effort_through_the_gateway() {
    let (done, chat) = attempted(THINKS, Some("high"));
    assert_eq!(chat["reasoning"], json!({"effort": "high"}));
    assert_eq!((done.model_calls, done.reasoning_tokens), (1, 420));
    assert_eq!(done.calls_without_reasoning, 0);
    assert_eq!(done.sent_at, [("high".to_owned(), 1)].into());
}

#[test]
fn a_model_without_the_level_is_sent_its_own_default_and_the_attempt_says_so() {
    let (done, chat) = attempted(MILD, Some("high"));
    assert_eq!(chat["reasoning"], json!({"effort": "medium"}));
    assert_eq!(done.sent_at, [("medium".to_owned(), 1)].into());
}

#[test]
fn a_model_that_offers_no_level_is_sent_none_and_the_attempt_says_so() {
    let (done, chat) = attempted(PLAIN, Some("high"));
    assert!(chat.get("reasoning").is_none(), "{chat}");
    assert_eq!(done.sent_at, [("default".to_owned(), 1)].into());
}

#[test]
fn without_a_level_nothing_is_sent() {
    let (done, chat) = attempted(THINKS, None);
    assert!(chat.get("reasoning").is_none(), "{chat}");
    assert_eq!(done.sent_at, [("default".to_owned(), 1)].into());
}
