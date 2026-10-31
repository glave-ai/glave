//! What a server asks the person inside a call: a form comes back and the call goes on with the
//! answer, on as many rounds and forms as it asks, up to its limit; what glave never asks a
//! person for is declined unseen, and so is everything where no one can answer, or the server
//! would hold the turn open while it waited.

use std::collections::BTreeMap;
use std::path::Path;

use glave_event::{ServiceAnswer, ServiceFieldKind};
use glave_mcp::{Limits, McpServer};
use glave_ports::settings::mcp::ServerConfig;
use glave_ports::{
    Called, NeverCancelled, ServiceAsks, ServiceGiven, ToolOutput, ToolSource as _, block_on,
};
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

/// A call of `ask` as `what` says, where the person can be asked, with `given`.
async fn ask(server: &McpServer, what: &str, given: Option<ServiceGiven>) -> Called {
    let arguments = arguments(json!({"what": what}));
    server
        .call_asking("mcp__github__ask", &arguments, given, &NeverCancelled)
        .await
}

/// What `called` asks; it must ask. What it came to instead is the service's own words: its
/// answer never echoes what the person typed.
fn asks(called: Called) -> ServiceAsks {
    match called {
        Called::Asks(asks) => asks,
        Called::Done(output) => panic!("it did not ask: {output:?}"),
    }
}

/// What `called` came to; it must not ask again.
fn done(called: Called) -> ToolOutput {
    match called {
        Called::Done(output) => output,
        Called::Asks(_) => panic!("it asked"),
    }
}

/// `asks` answered with `values`.
fn answered(asks: ServiceAsks, values: &Value) -> ServiceGiven {
    ServiceGiven {
        state: asks.state,
        answer: ServiceAnswer::Answered {
            values: values.as_object().unwrap().clone(),
        },
    }
}

/// Whether the server heard `answers` from the person, compared here so that a failure never
/// prints what was typed.
async fn heard_is(server: &McpServer, answers: &Value) -> bool {
    let none = arguments(json!({}));
    let said = server
        .call("mcp__github__answers", &none, &NeverCancelled)
        .await;
    serde_json::from_str::<Value>(&text(&said)).unwrap() == *answers
}

#[test]
fn a_form_comes_back_and_the_call_goes_on_with_the_answer() {
    block_on!(async {
        let server = start(&[]).await;
        let form = asks(ask(&server, "form", None).await);
        assert_eq!(form.service, "github", "by the name the person knows it by");
        assert_eq!(form.message, "Which label?");
        let names: Vec<&str> = form
            .fields
            .iter()
            .map(|field| field.name.as_str())
            .collect();
        assert_eq!(names, ["label", "note", "urgent"]);
        let label = form.fields.first().unwrap();
        assert!(label.required);
        assert!(
            matches!(&label.kind, ServiceFieldKind::Choice { options, .. } if options.len() == 2)
        );
        let typed = json!({"label": "bug", "note": "only the service reads this"});
        let output = done(ask(&server, "form", Some(answered(form, &typed))).await);
        assert!(heard_is(&server, &json!([typed])).await);
        let said = text(&output);
        // First, so what the rest print cannot hold what was typed.
        assert!(!said.contains("only the service reads this"));
        assert!(!output.is_error, "{said}");
        assert!(said.contains("You said: accept"), "{said}");
        assert!(
            said.contains("[github asked the person something directly"),
            "{said}"
        );
        server.close().await;
    });
}

#[test]
fn a_person_who_declines_is_said_to_have_and_the_call_goes_on() {
    block_on!(async {
        let server = start(&[]).await;
        let form = asks(ask(&server, "form", None).await);
        let given = ServiceGiven {
            state: form.state,
            answer: ServiceAnswer::Declined,
        };
        let said = text(&done(ask(&server, "form", Some(given)).await));
        assert!(said.contains("You said: decline"), "{said}");
        assert!(said.contains("they declined"), "{said}");
        assert!(heard_is(&server, &json!([])).await);
        server.close().await;
    });
}

#[test]
fn a_second_round_asks_again_and_keeps_what_it_asked_to_have_back() {
    block_on!(async {
        let server = start(&[]).await;
        let first = asks(ask(&server, "twice", None).await);
        assert_eq!(first.message, "Which label?");
        let given = answered(first, &json!({"label": "bug"}));
        let second = asks(ask(&server, "twice", Some(given)).await);
        assert_eq!(second.message, "And then?");
        let given = answered(second, &json!({"label": "idea"}));
        done(ask(&server, "twice", Some(given)).await);
        assert!(heard_is(&server, &json!([{"label": "bug"}, {"label": "idea"}])).await);
        server.close().await;
    });
}

#[test]
fn several_forms_in_a_round_reach_the_person_one_at_a_time() {
    block_on!(async {
        let server = start(&[]).await;
        let first = asks(ask(&server, "several", None).await);
        assert_eq!(first.message, "First?");
        let given = answered(first, &json!({"label": "bug"}));
        let second = asks(ask(&server, "several", Some(given)).await);
        assert_eq!(second.message, "Second?");
        assert!(
            heard_is(&server, &json!([])).await,
            "the round waits for both"
        );
        let given = answered(second, &json!({"label": "idea"}));
        let said = text(&done(ask(&server, "several", Some(given)).await));
        assert!(heard_is(&server, &json!([{"label": "bug"}, {"label": "idea"}])).await);
        assert!(said.contains("You said: accept, accept"), "{said}");
        server.close().await;
    });
}

#[test]
fn rounds_stay_capped_across_every_wait() {
    block_on!(async {
        let server = start(&[]).await;
        let mut called = ask(&server, "forever", None).await;
        let mut waits = 0;
        let output = loop {
            match called {
                Called::Asks(form) => {
                    waits += 1;
                    assert!(waits <= 10, "it should have stopped");
                    let given = answered(form, &json!({"label": "bug"}));
                    called = ask(&server, "forever", Some(given)).await;
                }
                Called::Done(output) => break output,
            }
        };
        assert_eq!(waits, 10);
        assert!(output.is_error);
        assert!(
            text(&output).contains("kept asking for more, 10 times"),
            "{output:?}"
        );
        server.close().await;
    });
}

#[test]
fn a_password_or_a_page_is_refused_with_no_card() {
    block_on!(async {
        let server = start(&[]).await;
        for what in ["password", "page"] {
            let said = text(&done(ask(&server, what, None).await));
            assert!(said.contains("You said: decline"), "{what}: {said}");
            assert!(said.contains("glave declined for them"), "{what}: {said}");
            assert!(
                said.contains("Do not ask them for it yourself"),
                "{what}: {said}"
            );
        }
        server.close().await;
    });
}

#[test]
fn with_no_one_to_answer_everything_is_declined() {
    block_on!(async {
        let server = start(&[]).await;
        let none = arguments(json!({"what": "several"}));
        let said = server
            .call("mcp__github__ask", &none, &NeverCancelled)
            .await;
        let said = text(&said);
        assert!(said.contains("You said: decline, decline"), "{said}");
        assert!(said.contains("no one was there to answer"), "{said}");
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
        let said = text(&done(ask(&server, "form", None).await));
        assert_eq!(said, "You said: Decline");
        assert!(heard_is(&server, &json!([])).await);
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
fn a_form_glave_cannot_read_is_refused_not_failed() {
    block_on!(async {
        let (ours, theirs) = tokio::io::duplex(64 * 1024);
        let serving = tokio::spawn(asks_with_a_password_format(theirs));
        let server = McpServer::connect("tests", ours, Limits::default());
        server.tools().await.unwrap();
        let said = done(ask_named(&server).await);
        assert!(!said.is_error, "{said:?}");
        assert!(text(&said).contains("glave declined for them"), "{said:?}");
        server.close().await;
        let heard = serving.await.unwrap();
        assert_eq!(heard["state"], "kept", "echoed as it was");
        assert_eq!(heard["given"]["form"]["action"], "decline");
    });
}

async fn ask_named(server: &McpServer) -> Called {
    let none = arguments(json!({}));
    server
        .call_asking("mcp__tests__ask", &none, None, &NeverCancelled)
        .await
}
