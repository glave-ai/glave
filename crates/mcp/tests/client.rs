//! What glave tells a server and hears from it: the folder a project's server works for (MCP's
//! roots), on either protocol, and the word a long call sends of its progress, which keeps it
//! from being given up on, up to a limit.

use std::collections::BTreeMap;
use std::path::Path;
use std::time::Duration;

use glave_event::{Arguments, Part};
use glave_mcp::{Limits, McpServer};
use glave_ports::settings::mcp::ServerConfig;
use glave_ports::{NeverCancelled, ToolOutput, ToolSource as _, block_on};
use serde_json::json;

const SERVER: &str = env!("CARGO_BIN_EXE_glave-mcp-test-server");

fn config(args: &[&str]) -> ServerConfig {
    ServerConfig::Stdio {
        command: SERVER.to_owned(),
        args: args.iter().map(|arg| (*arg).to_owned()).collect(),
        env: BTreeMap::new(),
    }
}

fn folder() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
}

/// The test server, started for the project in this crate's folder when `project`, else as a
/// server that works for every project.
async fn start(args: &[&str], project: bool, limits: Limits) -> McpServer {
    let (config, none) = (config(args), BTreeMap::new());
    let processes = glave_process::Processes::unrecorded();
    let server = if project {
        McpServer::start_for_project("tests", &config, &none, folder(), limits, &processes)
    } else {
        McpServer::start("tests", &config, &none, folder(), limits, &processes)
    };
    server.started().await.unwrap();
    server.tools().await.unwrap();
    server
}

fn arguments(value: serde_json::Value) -> Arguments {
    match value {
        serde_json::Value::Object(map) => Arguments::Json(map),
        other => panic!("not an object: {other}"),
    }
}

fn text(output: &ToolOutput) -> String {
    match output.parts.as_slice() {
        [Part::Text { text }] => text.clone(),
        other => panic!("not one text part: {other:?}"),
    }
}

async fn roots(server: &McpServer) -> ToolOutput {
    let none = arguments(json!({}));
    server
        .call("mcp__tests__roots", &none, &NeverCancelled)
        .await
}

#[test]
fn a_projects_server_is_told_its_folder_on_either_protocol() {
    block_on!(async {
        let root = url::Url::from_directory_path(folder()).unwrap();
        for args in [&[][..], &["--legacy"][..]] {
            let server = start(args, true, Limits::default()).await;
            let told = roots(&server).await;
            assert!(!told.is_error, "{args:?}: {told:?}");
            assert_eq!(text(&told), root.as_str(), "{args:?}");
            server.close().await;
        }
    });
}

#[test]
fn a_server_for_every_project_is_told_of_no_folder() {
    block_on!(async {
        for args in [&[][..], &["--legacy"][..]] {
            let server = start(args, false, Limits::default()).await;
            let told = roots(&server).await;
            assert_eq!(text(&told), "", "{args:?}: {told:?}");
            server.close().await;
        }
    });
}

#[test]
fn a_call_that_says_how_it_is_going_outlasts_the_quiet_limit() {
    block_on!(async {
        // Fifteen words a tenth of the limit apart: half as long again as the limit in all.
        let limits = Limits {
            call: Duration::from_secs(1),
            ..Limits::default()
        };
        let server = start(&[], false, limits).await;
        let ticks = arguments(json!({"every": 100, "times": 15}));
        let done = server
            .call("mcp__tests__progress", &ticks, &NeverCancelled)
            .await;
        assert_eq!(done, ToolOutput::text(false, "done after 15"));
        server.close().await;
    });
}

#[test]
fn a_call_that_never_finishes_is_stopped_at_its_total_limit() {
    block_on!(async {
        // Stopped at twelve quiet limits, however often it says how it is going.
        let limits = Limits {
            call: Duration::from_millis(250),
            ..Limits::default()
        };
        let server = start(&[], false, limits).await;
        let forever = arguments(json!({"every": 25, "times": 0}));
        let stopped = server
            .call("mcp__tests__progress", &forever, &NeverCancelled)
            .await;
        assert_eq!(
            stopped,
            ToolOutput::text(true, "tests was still working after 3s, so it was stopped.")
        );
        // Told to stop, the server answers the next call.
        let echo = arguments(json!({"text": "still here"}));
        let said = server
            .call("mcp__tests__echo", &echo, &NeverCancelled)
            .await;
        assert_eq!(text(&said), "still here");
        server.close().await;
    });
}
