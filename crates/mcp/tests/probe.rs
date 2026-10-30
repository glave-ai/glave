//! A server slow to answer its first request still starts: rmcp gives up on its probe for MCP
//! 2026-07-28 after 10 s and starts the older handshake, and the probe's answer, coming after,
//! is passed over. On a paused clock, so the probe's limit passes without waiting, and the
//! server answers the probe only once the handshake has begun: nothing depends on timing.

use glave_mcp::{Limits, McpServer};
use glave_ports::ToolSource as _;
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt as _, AsyncWriteExt as _, BufReader, DuplexStream};

/// Serves glave at `theirs` as a server slow to answer its first request: the probe is
/// answered `late`, only once the older handshake has begun, then that handshake and a listing
/// of one tool.
async fn slow_server(theirs: DuplexStream, late: Value) {
    let (read, mut write) = tokio::io::split(theirs);
    let mut lines = BufReader::new(read).lines();
    let mut next = async || -> Value {
        let line = lines.next_line().await.unwrap().unwrap();
        serde_json::from_str(&line).unwrap()
    };
    let mut answer = async |message: Value| {
        let line = format!("{message}\n");
        write.write_all(line.as_bytes()).await.unwrap();
    };
    let probe = next().await;
    assert_eq!(probe["method"], "server/discover", "{probe}");
    let initialize = next().await;
    assert_eq!(initialize["method"], "initialize", "{initialize}");
    let mut probed = late;
    probed["jsonrpc"] = json!("2.0");
    probed["id"] = probe["id"].clone();
    answer(probed).await;
    answer(json!({"jsonrpc": "2.0", "id": initialize["id"], "result": {
        "protocolVersion": "2025-11-25",
        "capabilities": {"tools": {}},
        "serverInfo": {"name": "slow", "version": "1"},
    }}))
    .await;
    let initialized = next().await;
    assert_eq!(initialized["method"], "notifications/initialized");
    let listing = next().await;
    assert_eq!(listing["method"], "tools/list", "{listing}");
    answer(json!({"jsonrpc": "2.0", "id": listing["id"], "result": {
        "tools": [{"name": "echo", "inputSchema": {"type": "object"}}],
    }}))
    .await;
}

#[test]
fn a_server_slow_to_answer_its_first_request_still_starts() {
    let legacy = json!({"error": {"code": -32601, "message": "Method not found"}});
    let modern = json!({"result": {}});
    for late in [legacy, modern] {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .start_paused(true)
            .build()
            .unwrap();
        runtime.block_on(async {
            let (ours, theirs) = tokio::io::duplex(64 * 1024);
            let serving = tokio::spawn(slow_server(theirs, late.clone()));
            let server = McpServer::connect("slow", ours, Limits::default());
            let tools = server.tools().await;
            let names: Vec<String> = tools
                .unwrap_or_else(|error| panic!("{late}: {error}"))
                .into_iter()
                .map(|tool| tool.spec.name)
                .collect();
            assert_eq!(names, ["mcp__slow__echo"], "{late}");
            serving.await.unwrap();
            server.close().await;
        });
    }
}
