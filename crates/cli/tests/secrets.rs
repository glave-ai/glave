//! Secrets, through the real binary over stdio with a fresh home: the person adds one by name
//! with the hosts it may reach; it is kept sealed, never answered back, logged or written in
//! the clear; the next turn's commands see only its placeholder, and its value, where a
//! command's output shows it anyway, is masked before the model sees it. Replacing and
//! removing it destroy the old value.

use glave_ports::testing::any_secret;
use glave_provider_openai::fake::{FakeServer, Reply};
use serde_json::json;

use crate::served::*;
use crate::support::*;

#[test]
fn a_secret_is_kept_sealed_commands_see_its_placeholder_and_output_masks_it() {
    let place = place("secrets");
    let (value, replaced) = (any_secret(), any_secret());
    // As an API that echoes the key it was sent would.
    let echoed = place.project.join("echoed.txt");
    std::fs::write(&echoed, format!("sent {value}\n")).unwrap();
    let command = format!(
        r#"printf 'key=%s\n' "$STRIPE_KEY" && cat '{}'"#,
        echoed.display()
    );
    let model = FakeServer::start(vec![
        Reply::tool_call("call_1", "bash", &json!({"command": command})),
        Reply::text("Called."),
    ])
    .unwrap();
    let args = ["--model", "m", "--permissions", "full-access"];
    let mut served = Served::start(&place, &model, &args);

    let stripe = json!({"name": "STRIPE_KEY", "hosts": ["API.stripe.com"], "value": value});
    let added = served.call("secret/add", &stripe);
    let shown = json!({"name": "STRIPE_KEY", "hosts": ["api.stripe.com"]});
    assert_eq!(added["result"]["secret"], shown, "{added}");
    let again = served.call("secret/add", &stripe);
    assert_eq!(
        again["error"]["code"], -32602,
        "a name kept already: {again}"
    );
    let listed = served.call("secret/list", &json!({}));
    assert_eq!(listed["result"]["secrets"], json!([shown]), "{listed}");

    let thread = served.call("thread/start", &json!({}))["result"]["thread"]["id"].clone();
    let turn = served.call(
        "turn/start",
        &json!({"thread": thread, "prompt": "call it"}),
    );
    assert_eq!(turn["result"]["turn"]["outcome"], "completed", "{turn}");
    let received = model.received();
    let results = said(&received[1].body, "tool").concat();
    assert!(
        results.contains("key=glave-secret-"),
        "a placeholder: {results}"
    );
    assert!(results.contains("sent [redacted]"), "masked: {results}");
    let events = served.call("thread/events", &json!({"thread": thread}));

    let renewed = json!({"name": "STRIPE_KEY", "value": replaced});
    let renewed = served.call("secret/replace", &renewed);
    assert_eq!(renewed["result"]["secret"], shown, "hosts kept: {renewed}");
    let removed = served.call("secret/remove", &json!({"name": "STRIPE_KEY"}));
    assert_eq!(removed["result"], json!({}), "{removed}");
    let gone = served.call("secret/remove", &json!({"name": "STRIPE_KEY"}));
    assert_eq!(gone["error"]["code"], -32024, "{gone}");
    let listed = served.call("secret/list", &json!({}));
    assert_eq!(listed["result"]["secrets"], json!([]));

    let said = served.end();
    for secret in [&value, &replaced] {
        assert!(
            !said.contains(secret.as_str()),
            "never answered or logged: {said}"
        );
        assert!(!events.to_string().contains(secret.as_str()), "{events}");
        assert!(!holds(&place.home, secret), "sealed at rest, and in no log");
        for request in model.received() {
            assert!(!request.body.to_string().contains(secret.as_str()));
        }
    }
}
