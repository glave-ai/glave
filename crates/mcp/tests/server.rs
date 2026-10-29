//! A real MCP server process as a tool source: started with only the environment it should
//! have, listed, called, cancelled, timed out and closed.

use std::collections::BTreeMap;
use std::path::Path;
use std::time::Duration;

use glave_event::{Arguments, Part};
use glave_mcp::{Limits, McpError, McpServer};
use glave_ports::block_on;
use glave_ports::settings::mcp::ServerConfig;
use glave_ports::testing::AlreadyCancelled;
use glave_ports::{Effect, NeverCancelled, ToolOutput, ToolSource as _, conformance};
use serde_json::json;

const SERVER: &str = env!("CARGO_BIN_EXE_glave-mcp-test-server");

/// glave's own environment, as the server sees it before the allowlist: with a key in it.
fn environment() -> BTreeMap<String, String> {
    let mut environment: BTreeMap<String, String> = std::env::vars().collect();
    environment.insert("GLAVE_API_KEY".into(), "not-a-real-key".into());
    environment
}

fn stdio(command: &str, args: &[&str], env: &[(&str, &str)]) -> ServerConfig {
    ServerConfig::Stdio {
        command: command.to_owned(),
        args: args.iter().map(|arg| (*arg).to_owned()).collect(),
        env: env
            .iter()
            .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
            .collect(),
    }
}

fn launch(name: &str, config: &ServerConfig, limits: Limits) -> McpServer {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    McpServer::start(
        name,
        config,
        &environment(),
        dir,
        limits,
        &glave_process::Processes::unrecorded(),
    )
}

async fn start(env: &[(&str, &str)]) -> McpServer {
    let server = launch("tests", &stdio(SERVER, &[], env), Limits::default());
    server.started().await.unwrap();
    server
}

pub(crate) fn arguments(value: serde_json::Value) -> Arguments {
    match value {
        serde_json::Value::Object(map) => Arguments::Json(map),
        other => panic!("not an object: {other}"),
    }
}

pub(crate) fn text(output: &ToolOutput) -> String {
    match output.parts.as_slice() {
        [Part::Text { text }] => text.clone(),
        other => panic!("not one text part: {other:?}"),
    }
}

#[test]
fn passes_the_tool_source_conformance_suite() {
    block_on!(async {
        let server = start(&[]).await;
        assert_eq!(conformance::tool_source(&server).await, Ok(()));
        server.close().await;
    });
}

#[test]
fn tools_are_named_for_their_server_and_marked_by_what_they_do() {
    block_on!(async {
        let server = start(&[]).await;
        let tools = server.tools().await.unwrap();
        let named: Vec<(&str, Effect, bool)> = tools
            .iter()
            .map(|tool| (tool.spec.name.as_str(), tool.effect, tool.parallel))
            .collect();
        assert_eq!(
            named,
            [
                ("mcp__tests__echo", Effect::Reads, true),
                ("mcp__tests__fail", Effect::Runs, false),
                ("mcp__tests__slow", Effect::Runs, false),
                ("mcp__tests__env", Effect::Runs, false),
                ("mcp__tests__header", Effect::Runs, false),
                ("mcp__tests__grow", Effect::Runs, false),
                ("mcp__tests__listed", Effect::Runs, false),
                ("mcp__tests__flood", Effect::Runs, false),
                ("mcp__tests__whose", Effect::Runs, false),
                ("mcp__tests__roots", Effect::Runs, false),
                ("mcp__tests__progress", Effect::Runs, false),
                ("mcp__tests__lookup", Effect::Reads, true),
            ]
        );
        assert_eq!(tools[0].spec.description, "Say the text back.");
        assert_eq!(
            server.instructions().as_deref(),
            Some("## tests\n\nCall echo to repeat text; it costs nothing.")
        );
        assert_eq!(
            tools[0].spec.parameters["properties"]["text"]["type"],
            "string"
        );
        server.close().await;
    });
}

#[test]
fn calls_reach_the_server_and_its_answers_come_back() {
    block_on!(async {
        let server = start(&[]).await;
        server.tools().await.unwrap();
        let echo = arguments(json!({"text": "hello"}));
        let said = server
            .call("mcp__tests__echo", &echo, &NeverCancelled)
            .await;
        assert_eq!(said, ToolOutput::text(false, "hello"));
        let failed = server
            .call("mcp__tests__fail", &arguments(json!({})), &NeverCancelled)
            .await;
        assert!(failed.is_error);
        assert_eq!(text(&failed), "it failed, as it always does");
        let flood = server
            .call("mcp__tests__flood", &arguments(json!({})), &NeverCancelled)
            .await;
        let flood = text(&flood);
        assert!(flood.starts_with("xxxx"), "{flood:.20}");
        assert!(flood.ends_with("[cut short: 250000 characters, only the first 100000 shown]"));
        server.close().await;
    });
}

#[test]
fn the_server_gets_its_own_settings_and_none_of_glaves_secrets() {
    block_on!(async {
        let server = start(&[("TRACKER_TOKEN", "for-the-tracker")]).await;
        server.tools().await.unwrap();
        let mut seen = Vec::new();
        for name in ["GLAVE_API_KEY", "TRACKER_TOKEN", "PATH"] {
            let asked = arguments(json!({ "name": name }));
            seen.push(text(
                &server
                    .call("mcp__tests__env", &asked, &NeverCancelled)
                    .await,
            ));
        }
        assert_eq!(
            seen,
            [
                "GLAVE_API_KEY set: false",
                "TRACKER_TOKEN set: true",
                "PATH set: true"
            ]
        );
        server.close().await;
    });
}

#[test]
fn a_cancelled_call_stops_and_the_server_keeps_working() {
    block_on!(async {
        let server = start(&[]).await;
        server.tools().await.unwrap();
        let none = arguments(json!({}));
        let stopped = server
            .call("mcp__tests__slow", &none, &AlreadyCancelled)
            .await;
        assert_eq!(
            stopped,
            ToolOutput::text(true, "[stopped: the person cancelled the turn]")
        );
        let echo = arguments(json!({"text": "still here"}));
        let said = server
            .call("mcp__tests__echo", &echo, &NeverCancelled)
            .await;
        assert_eq!(text(&said), "still here");
        server.close().await;
    });
}

#[test]
fn a_call_that_takes_too_long_is_given_up_on() {
    block_on!(async {
        let limits = Limits {
            call: Duration::from_millis(200),
            ..Limits::default()
        };
        let server = launch("tests", &stdio(SERVER, &[], &[]), limits);
        server.tools().await.unwrap();
        let none = arguments(json!({}));
        let late = server
            .call("mcp__tests__slow", &none, &NeverCancelled)
            .await;
        assert_eq!(
            late,
            ToolOutput::text(true, "tests did not answer within 200ms.")
        );
        server.close().await;
    });
}

#[test]
fn calls_after_closing_fail_with_a_reason() {
    block_on!(async {
        let server = start(&[]).await;
        server.tools().await.unwrap();
        server.close().await;
        let echo = arguments(json!({"text": "hello"}));
        let said = server
            .call("mcp__tests__echo", &echo, &NeverCancelled)
            .await;
        assert!(said.is_error);
        assert!(text(&said).starts_with("tests: "), "{said:?}");
        assert!(server.tools().await.is_err());
    });
}

#[test]
fn a_server_starts_in_the_background_and_its_first_listing_waits_for_it() {
    block_on!(async {
        let server = launch("tests", &stdio(SERVER, &[], &[]), Limits::default());
        let tools = server.tools().await.unwrap();
        assert_eq!(tools.len(), 12);
        server.close().await;
    });
}

#[test]
fn open_world_tools_and_servers_marked_untrusted_return_untrusted_output() {
    block_on!(async {
        let server = start(&[]).await;
        server.tools().await.unwrap();
        let asked = arguments(json!({"text": "rust"}));
        let found = server
            .call("mcp__tests__lookup", &asked, &NeverCancelled)
            .await;
        assert!(found.untrusted && !found.is_error, "{found:?}");
        let echoed = server
            .call("mcp__tests__echo", &asked, &NeverCancelled)
            .await;
        assert!(
            !echoed.untrusted,
            "a closed-world tool's output is not marked"
        );
        server.close().await;
        let marked = launch("tests", &stdio(SERVER, &[], &[]), Limits::default()).untrusted(true);
        marked.tools().await.unwrap();
        let echoed = marked
            .call("mcp__tests__echo", &asked, &NeverCancelled)
            .await;
        assert!(echoed.untrusted && marked.is_untrusted());
        // The server's own error is its words too; glave's are not.
        let failed = marked
            .call("mcp__tests__fail", &arguments(json!({})), &NeverCancelled)
            .await;
        assert!(failed.is_error && failed.untrusted, "{failed:?}");
        let missing = marked
            .call("mcp__tests__missing", &asked, &NeverCancelled)
            .await;
        assert!(missing.is_error && !missing.untrusted, "{missing:?}");
        marked.close().await;
    });
}

fn start_error(server: &McpServer, outcome: Result<(), McpError>) -> String {
    match outcome {
        Err(McpError::Start {
            server: name,
            message,
        }) => {
            assert_eq!(name, server.name());
            message
        }
        other => panic!("{} started: {other:?}", server.name()),
    }
}

#[test]
fn a_server_that_cannot_start_says_why() {
    block_on!(async {
        let missing = launch(
            "gone",
            &stdio("/nonexistent/glave-no-such-server", &[], &[]),
            Limits::default(),
        );
        let why = start_error(&missing, missing.started().await);
        assert!(why.starts_with("could not run /nonexistent/"), "{why}");
        let listed = missing.tools().await.unwrap_err();
        assert!(listed.to_string().contains("could not run"), "{listed}");
        let mute = launch("mute", &stdio("true", &[], &[]), Limits::default());
        let why = start_error(&mute, mute.started().await);
        assert_eq!(why, "it exited or closed the connection before answering");
    });
}

#[test]
fn a_server_that_never_answers_is_given_up_on_and_stopped() {
    block_on!(async {
        let limits = Limits {
            startup: Duration::from_millis(200),
            ..Limits::default()
        };
        let silent = launch("silent", &stdio("sleep", &["60"], &[]), limits);
        let why = start_error(&silent, silent.started().await);
        assert_eq!(why, "it did not answer within 200ms");
        silent.close().await;
    });
}

#[test]
fn closing_a_server_that_is_still_starting_stops_it() {
    block_on!(async {
        let silent = launch("silent", &stdio("sleep", &["60"], &[]), Limits::default());
        silent.close().await;
        let why = start_error(&silent, silent.started().await);
        assert_eq!(why, "it was closed");
    });
}
