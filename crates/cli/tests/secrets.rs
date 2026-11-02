//! Secrets, through the real binary over stdio with a fresh home: the person adds one by name;
//! it is kept sealed, never answered back, logged or written in the clear; the next turn's
//! commands see its value, and the value, where a command's output shows it, is masked before
//! the model sees it. Replacing and removing it destroy the old value.

use glave_ports::testing::any_secret;
use glave_provider_openai::fake::{FakeServer, Reply};
use serde_json::json;

use crate::served::*;
use crate::support::*;

#[test]
fn a_secret_is_kept_sealed_commands_see_its_value_and_output_masks_it() {
    let place = place("secrets");
    let (value, replaced) = (any_secret(), any_secret());
    let echoed = place.project.join("echoed.txt");
    std::fs::write(&echoed, format!("sent {value}\n")).unwrap();
    // The value compared where the model can't see it, then printed, as an API would echo it.
    let command = format!(
        r#"[ "sent $STRIPE_KEY" = "$(cat '{}')" ] && echo same-value; printf 'key=%s\n' "$STRIPE_KEY""#,
        echoed.display()
    );
    let model = FakeServer::start(vec![
        Reply::tool_call("call_1", "bash", &json!({"command": command})),
        Reply::text("Called."),
    ])
    .unwrap();
    let args = ["--model", "m", "--permissions", "full-access"];
    let mut served = Served::start(&place, &model, &args);

    let stripe = json!({"name": "STRIPE_KEY", "value": value});
    let added = served.call("secret/add", &stripe);
    let shown = json!({"name": "STRIPE_KEY"});
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
        results.contains("same-value"),
        "the command saw the value: {results}"
    );
    assert!(results.contains("key=[redacted]"), "masked: {results}");
    let events = served.call("thread/events", &json!({"thread": thread}));

    let renewed = json!({"name": "STRIPE_KEY", "value": replaced});
    let renewed = served.call("secret/replace", &renewed);
    assert_eq!(renewed["result"]["secret"], shown, "{renewed}");
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
