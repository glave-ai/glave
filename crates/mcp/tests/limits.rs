//! How long a call may take: a word of its progress keeps a long one from being given up on, up
//! to a limit, and a server that goes quiet is given up on, in a sentence that says which limit
//! it met. Against a server written by hand on a pipe, on a paused clock, as `tests/probe.rs`
//! is: an hour passes without waiting, and no stall of a busy machine can put one word after a
//! limit, so nothing depends on scheduling.

use std::time::Duration;

use glave_mcp::{Limits, McpServer};
use glave_ports::{NeverCancelled, ToolOutput, ToolSource as _, block_on};
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt as _, AsyncWriteExt as _, BufReader, DuplexStream, WriteHalf};
use tokio::time::{Instant, sleep, timeout};

use crate::server::{arguments, text};

/// How far apart the server's words are: a tenth of the default quiet limit of 5 minutes.
const EVERY: Duration = Duration::from_secs(30);

/// How long the test waits for the server to end once its pipe is closed, on the paused clock.
const ENDS_WITHIN: Duration = Duration::from_secs(60);

async fn send(write: &mut WriteHalf<DuplexStream>, message: Value) {
    let line = format!("{message}\n");
    write.write_all(line.as_bytes()).await.unwrap();
}

/// Serves glave at `theirs` until the pipe closes. Its tool `long` says how it is going `words`
/// times, `EVERY` apart, then answers if `answers`, or goes quiet; its tool `echo` says the text
/// back. Says whether it was told to stop.
async fn serve(theirs: DuplexStream, words: u32, answers: bool) -> bool {
    let (read, mut write) = tokio::io::split(theirs);
    let mut lines = BufReader::new(read).lines();
    let mut told_to_stop = false;
    while let Some(line) = lines.next_line().await.unwrap() {
        let asked: Value = serde_json::from_str(&line).unwrap();
        let params = &asked["params"];
        let result = match asked["method"].as_str().unwrap_or_default() {
            "server/discover" => {
                let error = json!({"code": -32601, "message": "Method not found"});
                let refused = json!({"jsonrpc": "2.0", "id": asked["id"], "error": error});
                send(&mut write, refused).await;
                continue;
            }
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
                let token = &params["_meta"]["progressToken"];
                for word in 1..=words {
                    sleep(EVERY).await;
                    let params = json!({"progressToken": token, "progress": word});
                    let method = "notifications/progress";
                    let said = json!({"jsonrpc": "2.0", "method": method, "params": params});
                    send(&mut write, said).await;
                }
                if !answers {
                    continue;
                }
                json!({"content": [{"type": "text", "text": "done"}]})
            }
            "tools/call" => json!({"content": [
                {"type": "text", "text": params["arguments"]["text"]},
            ]}),
            "notifications/cancelled" => {
                told_to_stop = true;
                continue;
            }
            _ => continue,
        };
        let answer = json!({"jsonrpc": "2.0", "id": asked["id"], "result": result});
        send(&mut write, answer).await;
    }
    told_to_stop
}

/// What became of a call of `long`, with the default limits.
struct CameTo {
    output: ToolOutput,
    /// How long it took, in whole seconds of the paused clock: a sleep on it can run a
    /// millisecond over.
    seconds: u64,
    told_to_stop: bool,
}

/// A call of `long` on a server that says how it is going `words` times.
async fn call(words: u32, answers: bool) -> CameTo {
    tokio::time::pause();
    let (ours, theirs) = tokio::io::duplex(64 * 1024);
    let serving = tokio::spawn(serve(theirs, words, answers));
    let server = McpServer::connect("tests", ours, Limits::default());
    server.tools().await.unwrap();
    let none = arguments(json!({}));
    let began = Instant::now();
    let output = server
        .call("mcp__tests__long", &none, &NeverCancelled)
        .await;
    let seconds = began.elapsed().as_secs();
    // Whatever became of that call, the server answers the next.
    let echo = arguments(json!({"text": "still here"}));
    let said = server
        .call("mcp__tests__echo", &echo, &NeverCancelled)
        .await;
    assert_eq!(text(&said), "still here");
    server.close().await;
    let told_to_stop = timeout(ENDS_WITHIN, serving).await.unwrap().unwrap();
    CameTo {
        output,
        seconds,
        told_to_stop,
    }
}

#[test]
fn a_call_that_says_how_it_is_going_outlasts_the_quiet_limit() {
    block_on!(async {
        // Fifteen words a tenth of the limit apart: half as long again as the limit in all.
        let call = call(15, true).await;
        assert_eq!(call.output, ToolOutput::text(false, "done"));
        assert_eq!(call.seconds, 450);
        assert!(!call.told_to_stop);
    });
}

#[test]
fn a_call_that_never_finishes_is_stopped_at_its_total_limit() {
    block_on!(async {
        // More words than the hour has room for, however often it says so: twelve quiet limits.
        let call = call(125, false).await;
        assert_eq!(
            call.output,
            ToolOutput::text(
                true,
                "tests was still working after 3600s, so it was stopped."
            )
        );
        assert_eq!(call.seconds, 3600);
        assert!(call.told_to_stop);
    });
}

#[test]
fn the_quiet_limit_is_counted_from_the_last_word() {
    block_on!(async {
        let call = call(2, false).await;
        assert_eq!(
            call.output,
            ToolOutput::text(true, "tests did not answer within 300s.")
        );
        assert_eq!(call.seconds, 360);
        assert!(call.told_to_stop);
    });
}
