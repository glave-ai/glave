//! Credits, the real binary through the person's server, with `[credits]` in its settings: a
//! turn with nothing added is refused with a sentence before the model is asked; whoever runs
//! the server adds credits; a turn is charged what the model said it cost; and once they are
//! spent, the next turn is refused again.

use glave_ports::testing::FakeHttp;
use glave_provider_openai::fake::{FakeServer, Reply};
use serde_json::{Value, json};

use crate::connected::Api;
use crate::http::{serve_http, serve_http_at};
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

/// The model a server's own OpenRouter serves, and another it offers beside it.
const OWN_MODEL: &str = "vendor/own";

/// OpenRouter's lists, as glave reads them for a catalog: two models, the first served by a
/// provider in the US and one in China; and a chat that says it cost a quarter.
fn own_openrouter() -> FakeHttp {
    let model = |id: &str| {
        json!({"id": id, "name": id, "created": 1, "context_length": 100_000,
            "architecture": {"input_modalities": ["text"], "output_modalities": ["text"]},
            "supported_parameters": ["tools", "tool_choice", "max_tokens"],
            "pricing": {"prompt": "0.000001", "completion": "0.000002"},
            "top_provider": {"context_length": 100_000, "max_completion_tokens": 100}})
    };
    let models = json!({"data": [model(OWN_MODEL), model("vendor/other")]});
    let providers = json!({"data": [
        {"slug": "stateside", "name": "Stateside", "headquarters": "US", "datacenters": ["US"]},
        {"slug": "overseas", "name": "Overseas", "headquarters": "CN", "datacenters": ["CN"]},
    ]});
    let endpoint = |name: &str, tag: &str| {
        json!({"provider_name": name, "tag": tag, "quantization": "fp8", "context_length": 100_000,
            "max_completion_tokens": 100, "supported_parameters": ["tools"],
            "pricing": {"prompt": "0.000001", "completion": "0.000002"}})
    };
    let endpoints = json!({"data": {"id": OWN_MODEL, "endpoints": [
        endpoint("Stateside", "stateside/fp8"), endpoint("Overseas", "overseas/fp8"),
    ]}});
    let said = json!({"id": "gen-1", "model": OWN_MODEL,
        "choices": [{"index": 0, "delta": {"content": "Hi."}}]});
    let ended = json!({"choices": [{"index": 0, "delta": {}, "finish_reason": "stop"}],
        "usage": {"prompt_tokens": 100, "completion_tokens": 10, "cost": 0.25}});
    let chat = format!("data: {said}\n\ndata: {ended}\n\ndata: [DONE]\n\n");
    FakeHttp::answering(
        move |method, path| match (method, path.split('?').next().unwrap()) {
            ("GET", "/api/v1/key") => (200, json!({"data": {"label": "own"}}).to_string()),
            ("GET", "/api/v1/models") => (200, models.to_string()),
            ("GET", "/api/v1/providers") => (200, providers.to_string()),
            ("GET", "/api/v1/endpoints/zdr") => (200, json!({"data": []}).to_string()),
            ("GET", "/api/v1/models/vendor/own/endpoints") => (200, endpoints.to_string()),
            ("GET", "/api/v1/models/vendor/other/endpoints") => {
                (200, json!({"data": {"endpoints": []}}).to_string())
            }
            ("GET", _) => (404, "{}".to_owned()),
            _ => (200, chat.clone()),
        },
    )
    .unwrap()
}

/// The server's entry for [`OWN_MODEL`], from `model/list`.
fn own_entry(api: &Api) -> Value {
    let listed = api.call("model/list", json!({}));
    let models = listed["result"]["models"].as_array().unwrap();
    let entry = models.iter().find(|model| model["id"] == OWN_MODEL);
    entry.unwrap_or_else(|| panic!("{listed}")).clone()
}

#[test]
fn a_servers_own_openrouter_shows_who_serves_its_models_and_charges_credits() {
    let place = place("credits-own-openrouter");
    std::fs::create_dir_all(&place.home).unwrap();
    let settings = "[credits]\nmonthly_limit_usd = 10\n";
    std::fs::write(place.home.join("config.toml"), settings).unwrap();
    let router = own_openrouter();
    let at = format!("{}/api/v1", router.url);
    let env = [
        ("GLAVE_PROVIDER", "openrouter"),
        ("GLAVE_OPENROUTER_URL", at.as_str()),
    ];
    let (_served, url, _) = serve_http_at(&place, &at, &["--model", OWN_MODEL], &env);
    let api = Api::new(&place, &url);

    // No one connected an account, and the server's own is read all the same.
    let refreshed = api.call("provider/refresh", json!({}));
    assert_eq!(refreshed["result"]["providers"], json!([]), "{refreshed}");
    let listed = api.call("model/list", json!({}));
    let ids: Vec<&str> = listed["result"]["models"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|model| model["id"].as_str())
        .collect();
    assert_eq!(ids, [OWN_MODEL, "vendor/other"], "{listed}");

    // Where each provider is based, and which the person's filters hide.
    let entry = own_entry(&api);
    let endpoints = entry["endpoints"].as_array().unwrap();
    let based: Vec<&str> = endpoints
        .iter()
        .filter_map(|endpoint| endpoint["based_in"].as_str())
        .collect();
    assert_eq!(based, ["US", "CN"], "{entry}");
    let kept = api.call("filters/set", json!({"filters": {"based_in": ["US"]}}));
    assert!(kept.get("result").is_some(), "{kept}");
    let entry = own_entry(&api);
    let hidden: Vec<bool> = entry["endpoints"]
        .as_array()
        .unwrap()
        .iter()
        .map(|endpoint| endpoint.get("hidden_by").is_some())
        .collect();
    assert_eq!(hidden, [false, true], "{entry}");

    // With nothing added its calls are refused; with credits, charged what the model said.
    let dummy = FakeServer::start(Vec::new()).unwrap();
    let run = |words: &[&str]| {
        let output = glave(&place, &dummy, words, "");
        (output.status.success(), text(&output))
    };
    let (sent, said) = run(&["--model", OWN_MODEL, "hello"]);
    assert!(!sent && said.contains("you're out of credits"), "{said}");
    let (added, said) = run(&["credits", "add", "1", "--reference", "order-1"]);
    assert!(added, "{said}");
    let (sent, said) = run(&["--model", OWN_MODEL, "hello"]);
    assert!(sent, "{said}");
    let (_, said) = run(&["credits"]);
    assert!(said.contains("Left: $0.75"), "{said}");

    // The call kept to the filters, by what the catalog says of who serves the model.
    let chats: Vec<_> = router
        .received()
        .into_iter()
        .filter(|request| request.body.contains(OWN_MODEL))
        .collect();
    let sent = chats.first().unwrap();
    assert!(
        sent.body.contains(r#""only":["stateside"]"#),
        "{}",
        sent.body
    );
}

#[test]
fn a_server_people_sign_in_to_with_oidc_reads_its_own_openrouter_too() {
    let place = place("oidc-own-openrouter");
    let router = own_openrouter();
    let at = format!("{}/api/v1", router.url);
    let env = [
        ("GLAVE_PROVIDER", "openrouter"),
        ("GLAVE_OPENROUTER_URL", at.as_str()),
    ];
    let args = [
        "--model",
        OWN_MODEL,
        "--issuer",
        "https://id.example.com",
        "--audience",
        "glave",
    ];
    let (_served, url, _) = serve_http_at(&place, &at, &args, &env);
    let api = Api::new(&place, &url);
    let refreshed = api.call("provider/refresh", json!({}));
    assert_eq!(refreshed["result"]["providers"], json!([]), "{refreshed}");
    let entry = own_entry(&api);
    assert_eq!(entry["endpoints"][0]["based_in"], "US", "{entry}");
}

#[test]
fn a_person_s_filters_can_t_reach_a_provider_outside_the_server_s_floor() {
    let place = place("router-floor");
    std::fs::create_dir_all(&place.home).unwrap();
    let settings = "[credits]\nmonthly_limit_usd = 10\n\n[router_floor]\nbased_in = [\"US\"]\n";
    std::fs::write(place.home.join("config.toml"), settings).unwrap();
    let router = own_openrouter();
    let at = format!("{}/api/v1", router.url);
    let env = [
        ("GLAVE_PROVIDER", "openrouter"),
        ("GLAVE_OPENROUTER_URL", at.as_str()),
    ];
    let (_served, url, _) = serve_http_at(&place, &at, &["--model", OWN_MODEL], &env);
    let api = Api::new(&place, &url);
    let refreshed = api.call("provider/refresh", json!({}));
    assert_eq!(refreshed["result"]["providers"], json!([]), "{refreshed}");

    // The picker hides what the floor rules out, and says so.
    let entry = own_entry(&api);
    assert_eq!(entry["endpoints"][0].get("hidden_by"), None, "{entry}");
    assert_eq!(
        entry["endpoints"][1]["hidden_by"],
        json!(["server"]),
        "{entry}"
    );

    let dummy = FakeServer::start(Vec::new()).unwrap();
    let run = |words: &[&str]| {
        let output = glave(&place, &dummy, words, "");
        (output.status.success(), text(&output))
    };
    let (added, said) = run(&["credits", "add", "1", "--reference", "order-1"]);
    assert!(added, "{said}");

    // A person who chose the country the floor rules out is refused with a sentence.
    let kept = api.call("filters/set", json!({"filters": {"based_in": ["CN"]}}));
    assert!(kept.get("result").is_some(), "{kept}");
    let (sent, said) = run(&["--model", OWN_MODEL, "hello"]);
    assert!(!sent, "{said}");
    assert!(
        said.contains("both your filters and what this server allows"),
        "{said}"
    );

    // Choosing the provider outside it by name reaches it no better.
    let overseas = json!({"filters": {"serves": {OWN_MODEL: {"provider": "overseas"}}}});
    let kept = api.call("filters/set", overseas);
    assert!(kept.get("result").is_some(), "{kept}");
    let (sent, said) = run(&["--model", OWN_MODEL, "hello"]);
    assert!(!sent, "{said}");

    // With none of their own, the floor alone decides who may serve it.
    let kept = api.call("filters/set", json!({"filters": {}}));
    assert!(kept.get("result").is_some(), "{kept}");
    let (sent, said) = run(&["--model", OWN_MODEL, "hello"]);
    assert!(sent, "{said}");

    let chats: Vec<_> = router
        .received()
        .into_iter()
        .filter(|request| request.body.contains(OWN_MODEL))
        .collect();
    assert_eq!(chats.len(), 1, "only the call the floor allows was sent");
    let sent = &chats[0].body;
    assert!(sent.contains(r#""only":["stateside"]"#), "{sent}");
    assert!(sent.contains(r#""data_collection":"deny""#), "{sent}");
}
