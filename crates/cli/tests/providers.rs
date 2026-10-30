//! Model accounts, through the real binary over stdio with a fresh home: a first run with no
//! model connects an account, whose key is checked against the server's model list, kept
//! sealed, never shown, logged or answered back, and used for the next turn at once; it is
//! still there after a restart; settings that choose a model stay the default; and removing
//! the account takes its models away.

use glave_ports::testing::any_secret;
use glave_provider_openai::fake::{FakeServer, Reply};
use serde_json::{Value, json};

use crate::served::*;
use crate::support::*;

/// The key the person pastes, which ends `wxyz`: new at each call.
fn key() -> String {
    any_secret() + "-wxyz"
}

/// The fake server's model list: two models, the newer second.
fn listing() -> Reply {
    Reply::Status(
        200,
        json!({"object": "list", "data": [
            {"id": "fake-old", "created": 1},
            {"id": "fake-new", "created": 2},
        ]})
        .to_string(),
    )
}

fn connect(url: &str, key: &str) -> Value {
    json!({"kind": "compatible", "url": url, "key": key})
}

#[test]
fn a_first_run_connects_an_account_and_its_models_answer_at_once() {
    let key = key();
    let place = place("accounts-first-run");
    let fake = FakeServer::start(vec![listing(), Reply::text("Hello from the account.")]).unwrap();
    let mut served = Served::start(&place, &fake, &[]);
    assert!(served.models().is_empty(), "no model is set up yet");

    let connected = served.call("provider/connect", &connect(fake.url(), &key));
    let account = &connected["result"]["provider"];
    assert_eq!(account["kind"], "compatible", "{connected}");
    assert!(account.get("hint").is_none(), "no key's ending");
    assert_eq!(account["models"], json!(["fake-new", "fake-old"]));
    assert_eq!(served.models(), ["fake-new", "fake-old"]);

    let thread = served.call("thread/start", &json!({}))["result"]["thread"]["id"].clone();
    let turn = served.call("turn/start", &json!({"thread": thread, "prompt": "hi"}));
    assert_eq!(turn["result"]["turn"]["outcome"], "completed", "{turn}");
    let received = fake.received();
    assert_eq!(received[0].line, "GET /v1/models");
    assert_eq!(received[1].body["model"], "fake-new");
    let bearer = format!("Bearer {key}");
    assert!(
        received
            .iter()
            .all(|r| r.authorization.as_deref() == Some(bearer.as_str()))
    );

    let listed = served.call("provider/list", &json!({}));
    assert!(listed["result"]["providers"][0].get("hint").is_none());
    let said = served.end();
    assert!(
        !said.contains(&key),
        "the key is never answered back or logged: {said}"
    );
    assert!(
        !holds(&place.home, &key),
        "the key is sealed at rest, and in no log"
    );

    // A restart finds the account.
    let mut served = Served::start(&place, &fake, &[]);
    assert_eq!(served.models(), ["fake-new", "fake-old"]);
    let listed = served.call("provider/list", &json!({}));
    let id = listed["result"]["providers"][0]["id"].clone();
    let removed = served.call("provider/remove", &json!({"provider": id}));
    assert_eq!(removed["result"], json!({}), "{removed}");
    assert!(served.models().is_empty(), "its models went with it");
    let again = served.call("provider/remove", &json!({"provider": id}));
    assert_eq!(again["error"]["code"], -32023);
    served.end();
}

#[test]
fn a_key_the_server_refuses_is_not_kept_and_says_what_to_do() {
    let key = key();
    let place = place("accounts-refused");
    let refused = r#"{"error":{"message":"Incorrect API key provided"}}"#;
    let fake = FakeServer::start(vec![Reply::Status(401, refused.into())]).unwrap();
    let mut served = Served::start(&place, &fake, &[]);
    let answered = served.call("provider/connect", &connect(fake.url(), &key));
    assert_eq!(answered["error"]["code"], -32020, "{answered}");
    assert_eq!(answered["error"]["data"]["failure"], "refused");
    let message = answered["error"]["message"].as_str().unwrap();
    assert!(message.contains("did not accept that key"), "{message}");
    let listed = served.call("provider/list", &json!({}));
    assert_eq!(listed["result"]["providers"], json!([]));
    let far = served.call(
        "provider/connect",
        &connect("http://models.example.com/v1", &key),
    );
    assert_eq!(
        far["error"]["code"], -32602,
        "plain http only on this computer"
    );
    let said = served.end();
    assert!(!said.contains(&key), "{said}");
}

#[test]
fn a_model_the_settings_choose_stays_the_default_beside_an_account() {
    let key = key();
    let place = place("accounts-precedence");
    let fake = FakeServer::start(vec![listing()]).unwrap();
    let mut served = Served::start(&place, &fake, &["--model", "chosen"]);
    assert_eq!(served.models(), ["chosen"]);
    let connected = served.call("provider/connect", &connect(fake.url(), &key));
    assert!(connected.get("result").is_some(), "{connected}");
    assert_eq!(served.models(), ["chosen", "fake-new", "fake-old"]);
    let listed = served.call("model/list", &json!({}));
    assert_eq!(listed["result"]["models"][0]["default"], true);
    served.end();
}

#[test]
fn each_model_is_listed_once_as_its_services_describe_it_and_falls_back_between_them() {
    let key = key();
    let place = place("accounts-described");
    let described = |price: &str| {
        Reply::Status(
            200,
            json!({"object": "list", "data": [{
                "id": "acme/rocket-1.5",
                "name": "Acme: Rocket 1.5",
                "context_length": 200_000,
                "architecture": {"input_modalities": ["text", "image"]},
                "pricing": {"prompt": price, "completion": "0.000004"},
                "top_provider": {"max_completion_tokens": 16_000},
                "reasoning": {"supported_efforts": ["high", "low", "none"], "default_effort": "low",
                              "default_enabled": true},
            }]})
            .to_string(),
        )
    };
    let overloaded = Reply::Status(503, r#"{"error":{"message":"overloaded"}}"#.into());
    let first = FakeServer::start(vec![described("0.000001"), overloaded]).unwrap();
    let second = FakeServer::start(vec![
        Reply::Status(200, json!({"data": [{"id": "rocket-1-5"}]}).to_string()),
        Reply::Stream(vec![
            json!({"id": "gen-2", "provider": "Fireworks", "choices": [{"index": 0, "delta": {"content": "From the second."}}]}),
            json!({"choices": [{"index": 0, "delta": {}, "finish_reason": "stop"}],
                   "usage": {"prompt_tokens": 100, "completion_tokens": 10, "cost": 0.00042}}),
        ]),
    ])
    .unwrap();
    let mut served = Served::start(&place, &first, &[]);
    served.call("provider/connect", &connect(first.url(), &key));
    served.call("provider/connect", &connect(second.url(), &key));
    let listed = served.call("model/list", &json!({}));
    let models = listed["result"]["models"].as_array().unwrap();
    assert_eq!(models.len(), 1, "one entry for the model: {listed}");
    let model = &models[0];
    assert_eq!(model["id"], "acme/rocket-1.5");
    assert_eq!(model["name"], "Rocket 1.5");
    assert_eq!(model["maker"], "Acme");
    assert_eq!(model["providers"], json!(["Local server", "Local server"]));
    assert_eq!(model["context"], 200_000);
    assert_eq!(model["max_output"], 16_000);
    assert_eq!(model["images"], true);
    assert_eq!(
        model["price"],
        json!({"input": 1_000_000, "output": 4_000_000})
    );
    // The levels, and the one the first service is sent unasked.
    let reasoning = json!({"efforts": ["high", "low", "none"], "default": "low"});
    assert_eq!(model["reasoning"], reasoning);

    let thread = served.call("thread/start", &json!({}))["result"]["thread"]["id"].clone();
    let turn = served.call("turn/start", &json!({"thread": thread, "prompt": "hi"}));
    assert_eq!(turn["result"]["turn"]["outcome"], "completed", "{turn}");
    let tried = second.received();
    assert_eq!(
        tried[1].body["model"], "rocket-1-5",
        "the second, once the first failed"
    );
    // Each service is sent its own listing's default: the second's names none.
    let sent = |server: &FakeServer| server.received()[1].body.get("reasoning_effort").cloned();
    assert_eq!((sent(&first), sent(&second)), (Some(json!("low")), None));
    // The record says where the turn ran, and what it cost by the service's own numbers.
    let events = served.call("thread/events", &json!({"thread": thread}));
    let responded = events["result"]["events"]
        .as_array()
        .unwrap()
        .iter()
        .find(|event| event["type"] == "model.responded")
        .unwrap();
    assert_eq!(responded["data"]["service"], "Local server", "{responded}");
    assert_eq!(responded["data"]["upstream"], "Fireworks");
    assert_eq!(responded["data"]["cost"]["amount"]["micros"], 420);
    assert!(responded["data"].get("effort").is_none(), "none was sent");
    served.end();
}

#[test]
fn refreshing_lists_the_models_again_and_keeps_them_when_the_server_cannot_say() {
    let key = key();
    let place = place("accounts-refresh");
    let newer = Reply::Status(
        200,
        json!({"object": "list", "data": [
            {"id": "fake-old", "created": 1},
            {"id": "fake-new", "created": 2},
            {"id": "fake-newest", "created": 3, "max_model_len": 32_768},
        ]})
        .to_string(),
    );
    let down = Reply::Status(503, r#"{"error":{"message":"overloaded"}}"#.into());
    let fake = FakeServer::start(vec![listing(), newer, down]).unwrap();
    let mut served = Served::start(&place, &fake, &[]);
    served.call("provider/connect", &connect(fake.url(), &key));
    let refreshed = served.call("provider/refresh", &json!({}));
    assert_eq!(
        refreshed["result"]["providers"][0]["models"],
        json!(["fake-newest", "fake-new", "fake-old"]),
        "{refreshed}"
    );
    assert_eq!(served.models(), ["fake-newest", "fake-new", "fake-old"]);
    let failed = served.call("provider/refresh", &json!({}));
    assert_eq!(failed["error"]["code"], -32020, "{failed}");
    let message = failed["error"]["message"].as_str().unwrap();
    assert!(message.contains("busy right now"), "{message}");
    served.end();

    // The catalog kept is the one the next start runs; a fresh one is not listed again.
    let mut served = Served::start(&place, &fake, &[]);
    assert_eq!(served.models(), ["fake-newest", "fake-new", "fake-old"]);
    let lines: Vec<String> = fake.received().iter().map(|r| r.line.clone()).collect();
    assert_eq!(lines, ["GET /v1/models"; 3]);
    served.end();
}
