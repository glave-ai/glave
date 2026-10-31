//! What glave tells a server: the folder a project's server works for (MCP's roots), on either
//! protocol.

use std::collections::BTreeMap;
use std::path::Path;

use glave_mcp::{Limits, McpServer};
use glave_ports::settings::mcp::ServerConfig;
use glave_ports::{NeverCancelled, ToolOutput, ToolSource as _, block_on};
use serde_json::json;

use crate::server::{arguments, text};

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
