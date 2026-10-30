//! Credits, the real binary through the person's server, with `[credits]` in its settings: a
//! turn with nothing added is refused with a sentence before the model is asked; whoever runs
//! the server adds credits; a turn is charged what the model said it cost; and once they are
//! spent, the next turn is refused again.

use glave_provider_openai::fake::{FakeServer, Reply};
use serde_json::json;

use crate::http::serve_http;
use crate::support::*;

/// An answer whose service says it cost a quarter, as OpenRouter's `usage.cost` does.
fn costly() -> Reply {
    Reply::Stream(vec![
        json!({"id": "gen-1", "model": "m", "choices": [{"index": 0, "delta": {"content": "Hi."}}]}),
        json!({"choices": [{"index": 0, "delta": {}, "finish_reason": "stop"}],
               "usage": {"prompt_tokens": 100, "completion_tokens": 10, "cost": 0.25}}),
    ])
}

#[test]
fn a_turn_spends_the_credits_added_and_once_they_are_spent_the_next_is_refused() {
    let place = place("credits");
    std::fs::create_dir_all(&place.home).unwrap();
    let settings = "[credits]\nmonthly_limit_usd = 10\n";
    std::fs::write(place.home.join("config.toml"), settings).unwrap();
    let model = FakeServer::start(vec![costly()]).unwrap();
    let (_served, _) = serve_http(&place, &model, &["--model", "m"]);
    let run = |words: &[&str]| {
        let output = glave(&place, &model, words, "");
        (output.status.success(), text(&output))
    };
    let (sent, said) = run(&["--model", "m", "hello"]);
    assert!(!sent, "{said}");
    assert!(said.contains("you're out of credits"), "{said}");
    assert!(model.received().is_empty(), "the model was never asked");
    let (added, said) = run(&["credits", "add", "0.25", "--reference", "order-1"]);
    assert!(added, "{said}");
    assert!(said.contains("Left: $0.25"), "{said}");
    let (again, said) = run(&["credits", "add", "0.25", "--reference", "order-1"]);
    assert!(
        again && said.contains("Left: $0.25"),
        "a payment adds once: {said}"
    );
    let (sent, said) = run(&["--model", "m", "hello"]);
    assert!(sent, "{said}");
    let (_, said) = run(&["credits"]);
    assert!(said.contains("Left: $0.00"), "{said}");
    assert!(said.contains("$0.25 spent of $10.00"), "{said}");
    assert!(said.contains("$0.25  you"), "{said}");
    let (sent, said) = run(&["--model", "m", "hello again"]);
    assert!(!sent && said.contains("you're out of credits"), "{said}");
    assert_eq!(model.received().len(), 1);
}

#[test]
fn a_glave_that_keeps_no_credits_says_so_and_charges_nothing() {
    let place = place("no-credits");
    let model = FakeServer::start(vec![Reply::text("Hi.")]).unwrap();
    let (_served, _) = serve_http(&place, &model, &["--model", "m"]);
    let output = glave(&place, &model, &["credits"], "");
    assert!(!output.status.success());
    assert!(
        text(&output).contains("keeps no credits"),
        "{}",
        text(&output)
    );
    let turn = glave(&place, &model, &["--model", "m", "hello"], "");
    assert!(turn.status.success(), "{}", text(&turn));
}
