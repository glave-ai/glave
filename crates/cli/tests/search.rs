//! Web search end to end: the real binary serves the person in a fresh home with a search key
//! in `GLAVE_SEARCH_KEY`, the search service a stand-in for Brave's (never Brave itself). It is
//! on by default. The model searches with glave's own `web_search`, the step says what it
//! searched, the results reach the model labelled untrusted, and the search is counted; the key
//! goes to the search service alone, never to the model, the person's client, the log or a file
//! in glave's folder. Turned off, it stays off when glave starts again; turned on, it is back.
//! With no key, it is not the person's to choose.

use std::path::Path;

use glave_ports::testing::{FakeHttp, any_secret};
use glave_ports::{Clock as _, SystemClock};
use glave_provider_openai::fake::{FakeServer, Reply};
use serde_json::{Value, json};

use crate::connected::Api;
use crate::http::{only_the_server_answers, serve_http_with};
use crate::support::*;

/// What the stand-in finds, as Brave's web search answers.
fn brave_answer() -> String {
    json!({"type": "search", "web": {"type": "search", "results": [{
        "title": "<strong>Tokio</strong>, an asynchronous runtime for Rust",
        "url": "https://tokio.rs/",
        "description": "Build reliable network applications.",
    }]}})
    .to_string()
}

/// Web search as `search/get` answers it.
fn search(api: &Api) -> Value {
    api.call("search/get", json!({"usage": true}))["result"]["search"].clone()
}

/// Whether any file under `folder` holds `text`.
fn held_in(folder: &Path, text: &str) -> bool {
    std::fs::read_dir(folder).unwrap().any(|entry| {
        let path = entry.unwrap().path();
        if path.is_dir() {
            return held_in(&path, text);
        }
        std::fs::read(&path).is_ok_and(|bytes| {
            bytes
                .windows(text.len())
                .any(|window| window == text.as_bytes())
        })
    })
}

#[test]
fn web_search_is_on_by_default_searches_untrusted_is_counted_and_stays_off_once_turned_off() {
    let place = place("search");
    let stand_in = FakeHttp::always(200, &brave_answer()).unwrap();
    let key = any_secret();
    let at = format!("{}/res/v1/web/search", stand_in.url);
    choose(&place, "project_memory", "\"off\"");
    choose(&place, "permissions", "\"full-access\"");
    let query = json!({"query": "rust async runtimes"});
    let model = FakeServer::start(vec![
        Reply::tool_call("call_1", "web_search", &query),
        Reply::text("Tokio."),
    ])
    .unwrap();
    let env = [
        ("GLAVE_SEARCH_KEY", key.as_str()),
        ("GLAVE_SEARCH_URL", at.as_str()),
    ];
    let (served, url, _) = serve_http_with(&place, &model, &["--model", "m"], &env);
    let api = Api::new(&place, &url);
    let config = place.home.join("config.toml");
    let own = std::fs::read_to_string(&config).unwrap();

    let thread = api.call("thread/start", json!({}))["result"]["thread"]["id"].clone();
    let month = || SystemClock.now().strftime("%Y-%m").to_string();
    let searched_in = month();
    let sent = api
        .turn(&thread, "which async runtimes are there for rust?")
        .concat();
    assert!(
        sent.contains("Searched the web: rust async runtimes"),
        "{sent}"
    );
    assert!(!sent.contains(&key), "the key never reaches the client");
    searched_untrusted(&model, &key);
    // Counted this month, unless the month turned meanwhile, which starts the count again.
    let counted = if month() == searched_in {
        json!({"calls": 1, "sentence": "One search this month"})
    } else {
        json!({"calls": 0, "sentence": "No searches this month"})
    };
    let on = search(&api);
    assert_eq!((&on["on"], &on["usage"]), (&json!(true), &counted), "{on}");

    only_the_server_answers(&place);
    let turned = glave(&place, &model, &["search", "off"], "");
    assert!(turned.status.success(), "{}", text(&turned));
    let off = json!({"kind": "chosen", "on": false, "detail": "Uses Brave Search. Your searches go to Brave Search."});
    assert_eq!(search(&api), off);

    // Off stays off when glave starts again: it is kept in the person's settings.
    std::fs::write(&config, &own).unwrap();
    drop(served);
    let (served, url, _) = serve_http_with(&place, &model, &["--model", "m"], &env);
    let api = Api::new(&place, &url);
    assert_eq!(search(&api), off);
    only_the_server_answers(&place);
    let said = text(&glave(&place, &model, &["search"], ""));
    assert!(
        said.contains("Web search is off. Uses Brave Search."),
        "{said}"
    );
    let turned = glave(&place, &model, &["search", "on"], "");
    let said = text(&turned);
    assert!(
        turned.status.success() && said.contains("Web search is on."),
        "{said}"
    );
    drop(served);

    let asked = stand_in.received();
    let first = asked.first().unwrap();
    assert!(
        first
            .path
            .starts_with("/res/v1/web/search?q=rust+async+runtimes")
    );
    assert_eq!(first.headers.get("x-subscription-token"), Some(&key));
    assert!(
        !held_in(&place.home, &key),
        "the key is in no file of glave's"
    );
}

#[test]
fn with_no_search_key_web_search_is_not_the_person_s_to_choose() {
    let place = place("search-none");
    let model = FakeServer::start(Vec::new()).unwrap();
    let (_served, url, _) = serve_http_with(&place, &model, &["--model", "m"], &[]);
    let api = Api::new(&place, &url);
    let never = search(&api);
    assert_eq!(never["kind"], "never", "{never}");
    let refused = api.call("search/set", json!({"on": true}));
    assert!(refused["error"].is_object(), "{refused}");
}

/// Asserts the model was offered `web_search`, that what it found reached the model labelled
/// untrusted, and that the key never did.
fn searched_untrusted(model: &FakeServer, key: &str) {
    let asked = model.received();
    let offered = asked[0].body["tools"]
        .as_array()
        .unwrap()
        .iter()
        .any(|tool| tool["function"]["name"] == "web_search");
    assert!(offered, "offered from the thread's first call");
    let results = said(&asked[1].body, "tool").concat();
    assert!(
        results.contains("<untrusted>\\n1. Tokio, an asynchronous runtime for Rust")
            && results.contains("https://tokio.rs/"),
        "{results}"
    );
    assert!(
        asked
            .iter()
            .all(|request| !request.body.to_string().contains(key)),
        "the key never reaches the model"
    );
}
