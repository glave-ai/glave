//! Web search end to end: the real binary serves the person in a fresh home, its server a
//! stand-in for Exa's hosted one (the MCP test server's, never Exa itself). It is off until the
//! person turns it on, from the terminal, which puts it in their own `mcp.json`; it is no
//! connection of theirs to list. From the next turn the model searches with it, the step says
//! what it searched, the results reach the model labelled untrusted, and the search is
//! counted. Turned off, it is out of their settings again.

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
    api.call("search/get", json!({}))["result"]["search"].clone()
}

#[test]
fn web_search_is_off_until_turned_on_then_searches_untrusted_and_is_counted() {
    let place = place("search");
    let (_exa, stand_in) = serving(&["--results", RESULTS]);
    let address = format!("{stand_in}?tools=web_search_exa");
    choose(&place, "memory", "\"off\"");
    choose(&place, "permissions", "\"full-access\"");
    let query = json!({"query": "rust async runtimes"});
    let model = FakeServer::start(vec![
        Reply::tool_call("call_1", "mcp__search__web_search_exa", &query),
        Reply::text("Tokio."),
    ])
    .unwrap();
    let env = [("GLAVE_SEARCH_URL", address.as_str())];
    let (_served, url, _) = serve_http_with(&place, &model, &["--model", "m"], &env);
    let api = Api::new(&place, &url);

    let off = json!({"kind": "chosen", "on": false, "detail": "Uses Exa, free. Your searches go to Exa."});
    assert_eq!(search(&api), off);
    assert!(
        !place.home.join("mcp.json").exists(),
        "nothing written while it is off"
    );
    let config = place.home.join("config.toml");
    let own = std::fs::read_to_string(&config).unwrap();
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
    let settings = std::fs::read_to_string(place.home.join("mcp.json")).unwrap();
    let settings: Value = serde_json::from_str(&settings).unwrap();
    assert_eq!(
        settings["mcpServers"]["search"],
        json!({"type": "http", "url": address})
    );
    let listed = text(&glave(&place, &model, &["connections"], ""));
    assert!(
        !listed.contains("search") && !listed.contains(&stand_in),
        "{listed}"
    );
    // The person's own settings read again, so the next turn reads `mcp.json` again too.
    std::fs::write(&config, own).unwrap();

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

    let turned = glave(&place, &model, &["search", "off"], "");
    assert!(turned.status.success(), "{}", text(&turned));
    assert_eq!(search(&api), off);
    let settings = std::fs::read_to_string(place.home.join("mcp.json")).unwrap();
    assert!(!settings.contains("web_search_exa"), "{settings}");
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
