//! What a server asks the person inside a call: glave has no way to reach them yet, so it
//! declines for them, on as many rounds and forms as it asks, up to its limit, and the call goes
//! on, whichever protocol the server speaks.

use std::collections::BTreeMap;
use std::path::Path;

use glave_mcp::{Limits, McpServer};
use glave_ports::settings::mcp::ServerConfig;
use glave_ports::{NeverCancelled, ToolOutput, ToolSource as _, block_on};
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt as _, AsyncWriteExt as _, BufReader, DuplexStream};

use crate::server::{arguments, text};

const SERVER: &str = env!("CARGO_BIN_EXE_glave-mcp-test-server");

async fn start(args: &[&str]) -> McpServer {
    let config = ServerConfig::Stdio {
        command: SERVER.to_owned(),
        args: args.iter().map(|arg| (*arg).to_owned()).collect(),
        env: BTreeMap::new(),
    };
    let processes = glave_process::Processes::unrecorded();
    let folder = Path::new(env!("CARGO_MANIFEST_DIR"));
    let none = BTreeMap::new();
    let server = McpServer::start(
        "github",
        &config,
        &none,
        folder,
        Limits::default(),
        &processes,
    );
    server.tools().await.unwrap();
    server
}

/// A call of `ask` as `what` says.
async fn ask(server: &McpServer, what: &str) -> ToolOutput {
    let arguments = arguments(json!({"what": what}));
    server
        .call("mcp__github__ask", &arguments, &NeverCancelled)
        .await
}

#[test]
fn what_a_server_asks_the_person_is_declined_and_the_call_goes_on() {
    block_on!(async {
        let server = start(&[]).await;
        for what in ["form", "password", "page"] {
            let output = ask(&server, what).await;
            let said = text(&output);
            assert!(!output.is_error, "{what}: {said}");
            assert!(said.contains("You said: decline"), "{what}: {said}");
            assert!(said.contains("[github asked the person"), "{what}: {said}");
        }
        let said = text(&ask(&server, "several").await);
        assert!(said.contains("You said: decline, decline"), "{said}");
        let said = text(&ask(&server, "twice").await);
        assert!(said.contains("You said: decline"), "{said}");
        server.close().await;
    });
}

#[test]
fn rounds_stay_capped() {
    block_on!(async {
        let server = start(&[]).await;
        let output = ask(&server, "forever").await;
        assert!(output.is_error);
        assert!(
            text(&output).contains("kept asking for more, 10 times"),
            "{output:?}"
        );
        server.close().await;
    });
}

#[test]
fn a_gateway_serving_a_tool_on_declines_what_it_asks() {
    block_on!(async {
        let server = start(&[]).await;
        let what = json!({"what": "form"}).as_object().cloned();
        let result = server.forward("ask", what).await.unwrap();
        let said = glave_mcp::tool_output(&result);
        assert_eq!(text(&said), "You said: decline");
        server.close().await;
    });
}

#[test]
fn a_server_on_the_older_protocol_is_declined_at_once_so_nothing_hangs() {
    block_on!(async {
        let server = start(&["--legacy"]).await;
        let said = text(&ask(&server, "form").await);
        assert_eq!(said, "You said: Decline");
        server.close().await;
    });
}

/// A server by hand, answering a call of `ask` with a form rmcp cannot read, a field of a format
/// MCP does not know, then with what it heard back.
async fn asks_with_a_password_format(theirs: DuplexStream) -> Value {
    let (read, mut write) = tokio::io::split(theirs);
    let mut lines = BufReader::new(read).lines();
    let mut heard = Value::Null;
    while let Some(line) = lines.next_line().await.unwrap() {
        let asked: Value = serde_json::from_str(&line).unwrap();
        let params = &asked["params"];
        let result = match asked["method"].as_str().unwrap_or_default() {
            "server/discover" => json!({
                "resultType": "complete",
                "supportedVersions": ["2026-07-28"],
                "capabilities": {"tools": {}},
                "ttlMs": 0,
                "cacheScope": "public",
            }),
            "tools/list" => json!({"tools": [{"name": "ask", "inputSchema": {"type": "object"}}]}),
            "tools/call" if params["inputResponses"].is_null() => json!({
                "resultType": "input_required",
                "requestState": "kept",
                "inputRequests": {"form": {"method": "elicitation/create", "params": {
                    "mode": "form", "message": "Sign in", "requestedSchema": {
                        "type": "object",
                        "properties": {"p": {"type": "string", "format": "password"}},
                    },
                }}},
            }),
            "tools/call" => {
                heard = json!({"state": params["requestState"], "given": params["inputResponses"]});
                json!({"resultType": "complete", "content": [{"type": "text", "text": "ok"}]})
            }
            _ => continue,
        };
        let answer = json!({"jsonrpc": "2.0", "id": asked["id"], "result": result});
        let line = format!("{answer}\n");
        write.write_all(line.as_bytes()).await.unwrap();
    }
    heard
}

#[test]
fn a_form_glave_cannot_read_is_declined_not_failed() {
    block_on!(async {
        let (ours, theirs) = tokio::io::duplex(64 * 1024);
        let serving = tokio::spawn(asks_with_a_password_format(theirs));
        let server = McpServer::connect("tests", ours, Limits::default());
        server.tools().await.unwrap();
        let none = arguments(json!({}));
        let said = server.call("mcp__tests__ask", &none, &NeverCancelled).await;
        assert!(!said.is_error, "{said:?}");
        assert!(text(&said).contains("it was declined"), "{said:?}");
        server.close().await;
        let heard = serving.await.unwrap();
        assert_eq!(heard["state"], "kept", "echoed as it was");
        assert_eq!(heard["given"]["form"]["action"], "decline");
    });
}
