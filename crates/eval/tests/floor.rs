//! An attempt's calls keep to Glave Cloud's floor, through the gateway as a server's do, with
//! who serves the model read from OpenRouter's lists; the attempt says which provider served
//! each call, and a model no provider within the floor serves is refused, never sent.

use glave_eval::{Floor, attempt};
use glave_ports::block_on;
use glave_ports::testing::{FakeHttp, any_secret};
use glave_provider_openai::Config;
use serde_json::{Value, json};

use crate::attempt::{folder, keyed_at, task};

const MODEL: &str = "vendor/own";

/// A stand-in for OpenRouter: its providers (one in the US at 8 bits, one in China, one in the
/// US at 4 bits), every endpoint keeping nothing, `endpoints` of [`MODEL`], and a chat that
/// answers from Stateside.
fn openrouter(endpoints: &[(&str, &str, &str)]) -> FakeHttp {
    let providers = json!({"data": [
        {"slug": "stateside", "name": "Stateside", "headquarters": "US"},
        {"slug": "overseas", "name": "Overseas", "headquarters": "CN"},
        {"slug": "lowbit", "name": "Lowbit", "headquarters": "US"},
    ]});
    let zdr: Vec<Value> = (endpoints.iter())
        .map(|(_, tag, _)| json!({"model_id": MODEL, "tag": tag}))
        .collect();
    let zdr = json!({"data": zdr});
    let listed: Vec<Value> = (endpoints.iter())
        .map(|(name, tag, bits)| json!({"provider_name": name, "tag": tag, "quantization": bits}))
        .collect();
    let listed = json!({"data": {"id": MODEL, "endpoints": listed}});
    let said = json!({"id": "gen-1", "model": MODEL, "provider": "Stateside",
        "choices": [{"index": 0, "delta": {"content": "Hi."}}]});
    let ended = json!({"choices": [{"index": 0, "delta": {}, "finish_reason": "stop"}],
        "usage": {"prompt_tokens": 100, "completion_tokens": 10, "cost": 0.001}});
    let chat = format!("data: {said}\n\ndata: {ended}\n\ndata: [DONE]\n\n");
    FakeHttp::answering(move |method, path| match (method, path) {
        ("GET", "/api/v1/providers") => (200, providers.to_string()),
        ("GET", "/api/v1/endpoints/zdr") => (200, zdr.to_string()),
        ("GET", "/api/v1/models/vendor/own/endpoints") => (200, listed.to_string()),
        ("GET", _) => (404, "{}".to_owned()),
        _ => (200, chat.clone()),
    })
    .unwrap()
}

/// The stand-in's endpoint, named as OpenRouter is.
fn endpoint(router: &FakeHttp) -> Config {
    let mut config = Config::for_url(format!("{}/api/v1", router.url), Some(any_secret())).unwrap();
    "openrouter".clone_into(&mut config.provider);
    config
}

/// One attempt of a task the model passes by answering, at `router` under Cloud's floor: the
/// attempt, and the chat requests the stand-in received.
fn attempted(name: &str, router: FakeHttp) -> (glave_eval::Attempt, Vec<Value>) {
    let dir = folder(name);
    let task = task(&dir, "", "exit 0\n");
    let config = endpoint(&router);
    let floor = block_on!(Floor::cloud(&config, &[MODEL.to_owned()])).unwrap();
    let mut setup = keyed_at(config, &any_secret());
    setup.floor = Some(floor);
    let done = block_on!(attempt(&setup, &task, MODEL, 1, &dir.join("run"))).unwrap();
    let chats = (router.received().into_iter())
        .filter(|request| request.method == "POST")
        .map(|request| serde_json::from_str(&request.body).unwrap())
        .collect();
    (done, chats)
}

#[test]
fn an_attempt_keeps_to_cloud_s_floor_and_says_who_served_it() {
    let router = openrouter(&[
        ("Stateside", "stateside/fp8", "fp8"),
        ("Overseas", "overseas/fp8", "fp8"),
        ("Lowbit", "lowbit/fp4", "fp4"),
    ]);
    let (done, chats) = attempted("floor-kept", router);
    assert!(done.passed(), "{:?}", done.failures);
    assert_eq!(done.served_by.get("Stateside"), Some(&1), "{done:?}");
    let asked = &chats.first().unwrap()["provider"];
    assert_eq!(asked["only"], json!(["stateside"]), "{asked}");
    assert_eq!(asked["zdr"], json!(true), "{asked}");
    let quantizations = asked["quantizations"].as_array().unwrap();
    assert!(quantizations.contains(&json!("fp8")), "{asked}");
    assert!(!quantizations.contains(&json!("fp4")), "{asked}");
    assert!(quantizations.contains(&json!("unknown")), "{asked}");
}

#[test]
fn a_model_served_only_below_the_floor_is_refused_and_never_sent() {
    let router = openrouter(&[("Lowbit", "lowbit/fp4", "fp4")]);
    let (done, chats) = attempted("floor-refused", router);
    assert!(!done.passed());
    assert!(chats.is_empty(), "never sent: {chats:?}");
    let said = done.failures.join("; ");
    assert!(said.contains("this server allows no provider"), "{said}");
}

#[test]
fn the_floor_needs_openrouter_and_says_how_to_run_without_it() {
    let config = Config::for_url("http://127.0.0.1:9/v1", None).unwrap();
    let refused = block_on!(Floor::cloud(&config, &[MODEL.to_owned()])).unwrap_err();
    assert!(refused.contains("--no-floor"), "{refused}");
}
