//! Searching the web end to end: the real binary serves the person in a fresh home, the search
//! glave suggests at a stand-in for Exa's hosted server (the MCP test server's, never Exa
//! itself). It is off until the person turns it on, from the terminal, which puts it in their
//! own `mcp.json`; from the next turn the model searches with it, the step says what it
//! searched, the results reach the model labelled untrusted, and the call is counted. Removed,
//! it is off again.

use glave_ports::{Clock as _, SystemClock};
use glave_provider_openai::fake::{FakeServer, Reply};
use serde_json::{Value, json};

use crate::connected::{Api, serving};
use crate::http::{only_the_server_answers, serve_http_with};
use crate::support::*;

/// What every search finds.
const RESULTS: &str = "1. Tokio, an asynchronous runtime for Rust: https://tokio.rs";

/// The suggestion as `connection/list` answers it.
fn search(api: &Api) -> Value {
    let listed = api.call("connection/list", json!({}));
    let connections = listed["result"]["connections"].as_array().unwrap();
    let found = connections.iter().find(|it| it["name"] == "search");
    found.unwrap_or_else(|| panic!("{listed}")).clone()
}

#[test]
fn the_suggested_search_is_off_until_turned_on_then_searches_untrusted_and_is_counted() {
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

    let off = search(&api);
    assert_eq!(
        (&off["status"], &off["url"], &off["suggested"]["title"]),
        (&json!("off"), &json!(address), &json!("Search the web")),
        "{off}"
    );
    assert!(off.get("usage").is_none(), "{off}");
    assert!(
        !place.home.join("mcp.json").exists(),
        "nothing is written while it is off"
    );
    let config = place.home.join("config.toml");
    let own = std::fs::read_to_string(&config).unwrap();
    only_the_server_answers(&place);
    let listed = text(&glave(&place, &model, &["connections"], ""));
    assert!(
        listed.contains("off: Search the web") && listed.contains("glave connect search"),
        "{listed}"
    );

    let turned = glave(&place, &model, &["connect", "search"], "");
    let said = text(&turned);
    assert!(
        turned.status.success() && said.contains("Turned on search"),
        "{said}"
    );
    let settings = std::fs::read_to_string(place.home.join("mcp.json")).unwrap();
    let settings: Value = serde_json::from_str(&settings).unwrap();
    assert_eq!(
        settings["mcpServers"]["search"],
        json!({"type": "http", "url": address})
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

    let on = search(&api);
    assert_eq!(on["status"], "on", "{on}");
    // Counted this month, unless the month turned meanwhile, which starts the count again.
    let counted = if month() == searched_in {
        json!({"calls": 1, "sentence": "One call this month"})
    } else {
        json!({"calls": 0, "sentence": "No calls this month"})
    };
    assert_eq!(on["usage"], counted);

    let removed = glave(&place, &model, &["connections", "remove", "search"], "");
    assert!(removed.status.success(), "{}", text(&removed));
    assert_eq!(search(&api)["status"], "off");
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
