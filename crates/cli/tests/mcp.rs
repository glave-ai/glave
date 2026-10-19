//! MCP servers from `mcp.json` in glave's folder, end to end: the real binary starts the test
//! server, offers its tools to a fake model, runs the call it asks for, and says what failed.

use std::path::{Path, PathBuf};

use glave_provider_openai::fake::{FakeServer, Reply};
use serde_json::json;

use crate::support::*;

/// The test server, built beside `glave` by `cargo xtask ci`.
fn test_server() -> PathBuf {
    let path = Path::new(env!("CARGO_BIN_EXE_glave")).with_file_name("glave-mcp-test-server");
    assert!(
        path.exists(),
        "build the test server first: cargo build -p glave-mcp --features test-server"
    );
    path
}

fn settings(place: &Place, servers: &serde_json::Value) {
    std::fs::create_dir_all(&place.home).unwrap();
    let file = json!({ "mcpServers": servers });
    std::fs::write(place.home.join("mcp.json"), file.to_string()).unwrap();
}

#[test]
fn a_servers_tools_reach_the_model_and_its_calls_run() {
    let place = place("mcp-tools");
    std::fs::create_dir_all(&place.home).unwrap();
    std::fs::write(place.home.join("config.toml"), "memory = \"off\"\n").unwrap();
    let server = test_server();
    settings(&place, &json!({ "tests": { "command": server } }));
    let model = FakeServer::start(vec![
        Reply::tool_call("call_1", "mcp__tests__echo", &json!({"text": "from mcp"})),
        Reply::text("It said: from mcp."),
    ])
    .unwrap();
    // Unconfined and without asking, so the call runs wherever the sandbox cannot.
    let output = glave(
        &place,
        &model,
        &[
            "--model",
            "m",
            "--sandbox",
            "off",
            "--approval",
            "auto",
            "echo it",
        ],
        "",
    );
    let shown = text(&output);
    assert!(output.status.success(), "{shown}");
    assert!(shown.contains("It said: from mcp."), "{shown}");
    let received = model.received();
    let offered: Vec<&str> = received[0].body["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|tool| tool["function"]["name"].as_str().unwrap())
        .collect();
    assert_eq!(
        offered,
        [
            "search",
            "glob",
            "read",
            "write",
            "edit",
            "bash",
            "command_output",
            "web_fetch",
            "mcp__tests__echo",
            "mcp__tests__fail",
            "mcp__tests__slow",
            "mcp__tests__env",
            "mcp__tests__header",
            "mcp__tests__grow",
            "mcp__tests__listed",
            "mcp__tests__flood",
            "mcp__tests__lookup",
            "list_mcp_resources",
            "read_mcp_resource",
            "agent",
        ]
    );
    let system = said(&received[0].body, "system").join("\n");
    assert!(
        system.contains("# Notes from tool servers")
            && system.contains("## tests\\n\\nCall echo to repeat text; it costs nothing."),
        "the server's instructions reach the model: {system}"
    );
    let results = said(&received[1].body, "tool");
    assert!(results[0].contains("from mcp"), "{results:?}");
}

#[test]
fn a_server_that_does_not_start_is_named_and_the_turn_goes_on() {
    let place = place("mcp-broken");
    let missing = "/nonexistent/glave-mcp-server";
    settings(&place, &json!({ "broken": { "command": missing } }));
    let model = FakeServer::start(vec![Reply::text("Answered anyway.")]).unwrap();
    let output = glave(&place, &model, &["--model", "m", "hello"], "");
    let shown = text(&output);
    assert!(output.status.success(), "{shown}");
    assert!(
        shown.contains(&format!(
            "warning MCP server \"broken\" did not start: could not run {missing}"
        )),
        "{shown}"
    );
    assert!(shown.contains("Answered anyway."), "{shown}");
}

#[test]
fn bad_settings_stop_glave_before_anything_runs() {
    let place = place("mcp-bad");
    settings(&place, &json!({ "odd": { "comand": "typo" } }));
    let model = FakeServer::start(vec![Reply::text("never")]).unwrap();
    let output = glave(&place, &model, &["--model", "m", "hello"], "");
    let shown = text(&output);
    assert!(!output.status.success(), "{shown}");
    assert!(shown.contains("mcp.json"), "{shown}");
    assert!(shown.contains("comand"), "names the unknown field: {shown}");
    assert!(model.received().is_empty());
}
