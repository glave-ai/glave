//! Web search end to end: the real binary serves the person in a fresh home, its server a
//! stand-in for Exa's hosted one (the MCP test server's, never Exa itself). It is on by
//! default, in the person's own `mcp.json`, written on the first start; it is no connection of
//! theirs to list. The model searches with it, the step says what it searched, the results
//! reach the model labelled untrusted, and the search is counted. Turned off, it is out of
//! their settings, and stays off when glave starts again; turned on, it is back.

use glave_ports::{Clock as _, SystemClock};
use glave_provider_openai::fake::{FakeServer, Reply};
use serde_json::{Value, json};

use crate::connected::{Api, serving};
use crate::http::{only_the_server_answers, serve_http_with};
use crate::support::*;

/// What every search finds.
const RESULTS: &str = "1. Tokio, an asynchronous runtime for Rust: https://tokio.rs";

/// Web search as `search/get` answers it.
fn search(api: &Api) -> Value {
    api.call("search/get", json!({"usage": true}))["result"]["search"].clone()
}

#[test]
fn web_search_is_on_by_default_searches_untrusted_is_counted_and_stays_off_once_turned_off() {
    let place = place("search");
    // A home with no settings of the person's own, as on a first start.
    std::fs::remove_file(place.home.join("mcp.json")).unwrap();
    let (_exa, stand_in) = serving(&["--results", RESULTS]);
    let address = format!("{stand_in}?tools=web_search_exa");
    choose(&place, "project_memory", "\"off\"");
    choose(&place, "permissions", "\"full-access\"");
    let query = json!({"query": "rust async runtimes"});
    let model = FakeServer::start(vec![
        Reply::tool_call("call_1", "mcp__search__web_search_exa", &query),
        Reply::text("Tokio."),
    ])
    .unwrap();
    let env = [("GLAVE_SEARCH_URL", address.as_str())];
    let (served, url, _) = serve_http_with(&place, &model, &["--model", "m"], &env);
    let api = Api::new(&place, &url);

    let ours = json!({"type": "http", "url": address});
    let settings = || -> Value {
        let text = std::fs::read_to_string(place.home.join("mcp.json")).unwrap();
        serde_json::from_str(&text).unwrap()
    };
    assert_eq!(settings()["mcpServers"]["search"], ours, "on by default");
    let config = place.home.join("config.toml");
    let own = std::fs::read_to_string(&config).unwrap();
    only_the_server_answers(&place);
    let listed = text(&glave(&place, &model, &["connections"], ""));
    assert!(
        !listed.contains("search") && !listed.contains(&stand_in),
        "{listed}"
    );
    std::fs::write(&config, &own).unwrap();

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
    searched_alone_and_untrusted(&model);
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
    let off = json!({"kind": "chosen", "on": false, "detail": "Uses Exa, free. Your searches go to Exa."});
    assert_eq!(search(&api), off);
    assert!(!settings().to_string().contains("web_search_exa"));

    // Off stays off when glave starts again: its settings are the person's now.
    std::fs::write(&config, &own).unwrap();
    drop(served);
    let (_served, url, _) = serve_http_with(&place, &model, &["--model", "m"], &env);
    let api = Api::new(&place, &url);
    assert_eq!(search(&api), off);
    only_the_server_answers(&place);
    let said = text(&glave(&place, &model, &["search"], ""));
    assert!(
        said.contains("Web search is off. Uses Exa, free."),
        "{said}"
    );
    let turned = glave(&place, &model, &["search", "on"], "");
    let said = text(&turned);
    assert!(
        turned.status.success() && said.contains("Web search is on."),
        "{said}"
    );
    assert_eq!(settings()["mcpServers"]["search"], ours);
}

/// What the tools told the model in `body`.
fn said_to(body: &Value) -> String {
    said(body, "tool").concat()
}

/// Asserts the model was offered the search's one tool, and that what it found reached the
/// model labelled untrusted.
fn searched_alone_and_untrusted(model: &FakeServer) {
    let asked = model.received();
    let offered: Vec<&str> = asked[0].body["tools"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|tool| tool["function"]["name"].as_str())
        .filter(|name| name.starts_with("mcp__search__"))
        .collect();
    assert_eq!(offered, ["mcp__search__web_search_exa"], "only its search");
    let results = said_to(&asked[1].body);
    assert!(
        results.contains("<untrusted>\\n1. Tokio") && results.contains("https://tokio.rs"),
        "{results}"
    );
}
