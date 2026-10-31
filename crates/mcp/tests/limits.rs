//! How long a call may take: a word of its progress keeps a long one from being given up on, up
//! to a limit, and a server that goes quiet is given up on, in a sentence that says which limit
//! it met. Against a server written by hand on a pipe, on a paused clock, as `tests/probe.rs`
//! is: an hour passes without waiting, and no stall of a busy machine can put one word after a
//! limit, so nothing depends on scheduling.

use std::future::pending;
use std::time::Duration;

use glave_mcp::{Limits, McpServer};
use glave_ports::{NeverCancelled, ToolOutput, ToolSource as _, block_on};
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt as _, AsyncWriteExt as _, BufReader, DuplexStream, WriteHalf};
use tokio::time::{sleep, timeout};

use crate::server::{arguments, text};

/// How far apart the server's words are: a tenth of the default quiet limit of 5 minutes.
const EVERY: Duration = Duration::from_secs(30);

/// How long the test waits for the server to end once its pipe is closed, on the paused clock.
const ENDS_WITHIN: Duration = Duration::from_secs(60);

/// A call of `long` the server is at work on.
struct Working {
    id: Value,
    token: Value,
    /// How many times it will still say how it is going.
    words: u32,
}

async fn send(write: &mut WriteHalf<DuplexStream>, message: Value) {
    let line = format!("{message}\n");
    write.write_all(line.as_bytes()).await.unwrap();
}

/// Serves glave at `theirs`. Its tool `long` says how it is going `words` times, `EVERY` apart,
/// then answers if `answers`, or goes quiet; its tool `echo` says the text back. Says, once the
/// pipe closes, whether it was told to stop.
async fn serve(theirs: DuplexStream, words: u32, answers: bool) -> bool {
    let (read, mut write) = tokio::io::split(theirs);
    let mut lines = BufReader::new(read).lines();
    let mut working: Option<Working> = None;
    let mut told_to_stop = false;
    loop {
        let word = async {
            match &working {
                Some(call) if call.words > 0 => sleep(EVERY).await,
                _ => pending().await,
            }
        };
        tokio::select! {
            () = word => {
                let Some(call) = working.as_mut() else { continue };
                call.words -= 1;
                send(&mut write, json!({
                    "jsonrpc": "2.0",
                    "method": "notifications/progress",
                    "params": {"progressToken": call.token, "progress": 1},
                }))
                .await;
                if call.words == 0 && answers {
                    let result = json!({"content": [{"type": "text", "text": "done"}]});
                    send(&mut write, json!({"jsonrpc": "2.0", "id": call.id, "result": result}))
                        .await;
                    working = None;
                }
            }
            line = lines.next_line() => {
                let Some(line) = line.unwrap() else { return told_to_stop };
                let asked: Value = serde_json::from_str(&line).unwrap();
                let params = &asked["params"];
                let result = match asked["method"].as_str().unwrap_or_default() {
                    "initialize" => json!({
                        "protocolVersion": "2025-11-25",
                        "capabilities": {"tools": {}},
                        "serverInfo": {"name": "tests", "version": "1"},
                    }),
                    "tools/list" => json!({"tools": [
                        {"name": "long", "inputSchema": {"type": "object"}},
                        {"name": "echo", "inputSchema": {"type": "object"}},
                    ]}),
                    "tools/call" if params["name"] == "long" => {
                        working = Some(Working {
                            id: asked["id"].clone(),
                            token: params["_meta"]["progressToken"].clone(),
                            words,
                        });
                        continue;
                    }
                    "tools/call" => json!({"content": [
                        {"type": "text", "text": params["arguments"]["text"]},
                    ]}),
                    "notifications/cancelled" => {
                        told_to_stop = true;
                        working = None;
                        continue;
                    }
                    "server/discover" => {
                        let error = json!({"code": -32601, "message": "Method not found"});
                        send(&mut write, json!({"jsonrpc": "2.0", "id": asked["id"], "error": error}))
                            .await;
                        continue;
                    }
                    _ => continue,
                };
                send(&mut write, json!({"jsonrpc": "2.0", "id": asked["id"], "result": result}))
                    .await;
            }
        }
    }
}

/// What a call of `long` comes to on a server that says how it is going `words` times, with the
/// default limits, and whether the server was told to stop.
async fn call(words: u32, answers: bool) -> (ToolOutput, bool) {
    tokio::time::pause();
    let (ours, theirs) = tokio::io::duplex(64 * 1024);
    let serving = tokio::spawn(serve(theirs, words, answers));
    let server = McpServer::connect("tests", ours, Limits::default());
    server.tools().await.unwrap();
    let none = arguments(json!({}));
    let came_to = server
        .call("mcp__tests__long", &none, &NeverCancelled)
        .await;
    // Whatever became of that call, the server answers the next.
    let echo = arguments(json!({"text": "still here"}));
    let said = server
        .call("mcp__tests__echo", &echo, &NeverCancelled)
        .await;
    assert_eq!(text(&said), "still here");
    server.close().await;
    let told_to_stop = timeout(ENDS_WITHIN, serving).await.unwrap().unwrap();
    (came_to, told_to_stop)
}

#[test]
fn a_call_that_says_how_it_is_going_outlasts_the_quiet_limit() {
    block_on!(async {
        // Fifteen words a tenth of the limit apart: half as long again as the limit in all.
        let (came_to, told_to_stop) = call(15, true).await;
        assert_eq!(came_to, ToolOutput::text(false, "done"));
        assert!(!told_to_stop);
    });
}

#[test]
fn a_call_that_never_finishes_is_stopped_at_its_total_limit() {
    block_on!(async {
        // More words than the hour has room for: twelve quiet limits, however often it says so.
        let (came_to, told_to_stop) = call(u32::MAX, false).await;
        assert_eq!(
            came_to,
            ToolOutput::text(
                true,
                "tests was still working after 3600s, so it was stopped."
            )
        );
        assert!(told_to_stop);
    });
}

#[test]
fn a_call_that_goes_quiet_is_given_up_on_at_the_quiet_limit() {
    block_on!(async {
        let (came_to, told_to_stop) = call(2, false).await;
        assert_eq!(
            came_to,
            ToolOutput::text(true, "tests did not answer within 300s.")
        );
        assert!(told_to_stop);
    });
}
