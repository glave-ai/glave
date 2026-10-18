//! One set of scenarios, run over every transport: in-process, JSON-RPC 2.0 over a stream of
//! lines (as stdio carries it), the same over a Unix socket to a long-running server, and HTTP
//! with server-sent events, signed in. A transport changes how messages travel, never what they
//! mean (`specs/session.md`), so each scenario must end the same way over each.

use glave_event::{Body, Outcome, ThreadId, decode};
use glave_session::protocol::{
    Answer, ApiError, Meta, Notification, Reply, Request, TurnResult, Update, code,
};
use glave_session::{Answering, TurnItem, rpc};
use serde_json::{Value, json};
use tokio::io::{
    AsyncBufReadExt as _, AsyncRead, AsyncWrite, AsyncWriteExt as _, BufReader, Lines,
};
use tokio::task::JoinHandle;

use crate::support::issuer::{Issuer, Signer};
use crate::support::web::{Events, Served, Web, serve_http};
use crate::support::*;

/// What a scenario does after a notification: send these requests, in order.
type Reaction = Vec<Request>;

/// A client of one transport.
pub(crate) trait Client {
    async fn call(&mut self, request: Request) -> Result<Value, ApiError>;
    /// Runs a turn, letting `react` answer notifications with requests of its own.
    async fn turn(
        &mut self,
        thread: ThreadId,
        prompt: &str,
        react: &mut dyn FnMut(&Notification) -> Reaction,
    ) -> Result<TurnResult, ApiError>;
}

pub(crate) struct InProcess(pub(crate) Tested);

impl Client for InProcess {
    async fn call(&mut self, request: Request) -> Result<Value, ApiError> {
        let reply: Reply = self.0.request(request).await?;
        Ok(serde_json::to_value(reply).unwrap())
    }

    async fn turn(
        &mut self,
        thread: ThreadId,
        prompt: &str,
        react: &mut dyn FnMut(&Notification) -> Reaction,
    ) -> Result<TurnResult, ApiError> {
        let mut stream = stream_turn(&self.0, thread, prompt, None).await?;
        while let Some(item) = stream.next().await {
            match item {
                TurnItem::Note(note) => {
                    for request in react(&note) {
                        // Answers and cancels are not part of the turn's own result.
                        let _ = self.0.request(request).await;
                    }
                }
                TurnItem::Missed(_) => {}
                TurnItem::Ended(ended) => return ended,
            }
        }
        panic!("the turn never ended")
    }
}

type Writer = Box<dyn AsyncWrite + Unpin + Send>;
type Reader = Box<dyn AsyncRead + Unpin + Send>;

/// JSON-RPC 2.0, one message per line, over an in-memory pipe to `rpc::serve`, or over a
/// Unix socket to `rpc::serve_connection`.
pub(crate) struct OverJsonRpc {
    to_server: Writer,
    from_server: Lines<BufReader<Reader>>,
    next_id: u64,
    serving: Option<JoinHandle<std::io::Result<()>>>,
    /// A long-running server, shut down when the test is done with it.
    shared: Option<Tested>,
}

impl OverJsonRpc {
    pub(crate) fn new(server: Tested) -> Self {
        let (client_out, server_in) = tokio::io::duplex(1 << 16);
        let (server_out, client_in) = tokio::io::duplex(1 << 16);
        let serving = tokio::spawn(glave_session::rpc::serve(
            server,
            BufReader::new(server_in),
            server_out,
        ));
        Self {
            to_server: Box::new(client_out),
            from_server: BufReader::new(Box::new(client_in) as Reader).lines(),
            next_id: 0,
            serving: Some(serving),
            shared: None,
        }
    }

    /// A client of `server` serving on a Unix socket, as `glave serve --socket` does.
    pub(crate) async fn on_socket(server: Tested, name: &str) -> Self {
        // Short, as socket paths must be.
        let path =
            std::path::PathBuf::from(format!("/tmp/glave-{name}-{}.sock", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let listener = tokio::net::UnixListener::bind(&path).unwrap();
        let serving_server = server.clone();
        let serving = tokio::spawn(async move {
            let (stream, _) = listener.accept().await?;
            let (read, write) = stream.into_split();
            glave_session::rpc::serve_connection(serving_server, BufReader::new(read), write).await
        });
        let stream = tokio::net::UnixStream::connect(&path).await.unwrap();
        let _ = std::fs::remove_file(&path);
        let (read, write) = stream.into_split();
        Self {
            to_server: Box::new(write),
            from_server: BufReader::new(Box::new(read) as Reader).lines(),
            next_id: 0,
            serving: Some(serving),
            shared: Some(server),
        }
    }

    async fn send(&mut self, request: &Request) -> u64 {
        self.next_id += 1;
        let line = format!("{}\n", rpc::message(self.next_id, request));
        self.to_server.write_all(line.as_bytes()).await.unwrap();
        self.next_id
    }

    async fn receive(&mut self) -> Value {
        let line = self.from_server.next_line().await.unwrap().unwrap();
        serde_json::from_str(&line).unwrap()
    }

    /// Ends the client's input and waits for the server to finish serving it.
    pub(crate) async fn close(mut self) {
        self.to_server.shutdown().await.unwrap();
        drop(self.to_server);
        if let Some(serving) = self.serving.take() {
            serving.await.unwrap().unwrap();
        }
        if let Some(server) = self.shared.take() {
            server.shutdown().await;
        }
    }
}

/// What a response says, as the scenarios read it: the result as JSON, or the error. Kept as
/// JSON: a reply holding events cannot be read back as a `Reply`, whose untagged variants
/// cannot hold raw JSON.
fn outcome(message: &Value) -> Result<Value, ApiError> {
    assert_eq!(message["jsonrpc"], "2.0");
    match message.get("error") {
        Some(error) => Err(
            serde_json::from_value(error.clone()).unwrap_or_else(|err| ApiError {
                code: 0,
                message: format!("unreadable error: {err}"),
                data: None,
            }),
        ),
        None => Ok(message["result"].clone()),
    }
}

/// How a turn ended, from its response.
fn ended(message: &Value) -> Result<TurnResult, ApiError> {
    match rpc::outcome(message)? {
        Reply::Turn { turn } => Ok(turn),
        other => panic!("a turn, not {other:?}"),
    }
}

impl Client for OverJsonRpc {
    async fn call(&mut self, request: Request) -> Result<Value, ApiError> {
        let id = self.send(&request).await;
        loop {
            let message = self.receive().await;
            if message["id"] == id {
                return outcome(&message);
            }
        }
    }

    async fn turn(
        &mut self,
        thread: ThreadId,
        prompt: &str,
        react: &mut dyn FnMut(&Notification) -> Reaction,
    ) -> Result<TurnResult, ApiError> {
        let request = Request::TurnStart {
            background: false,
            budget: None,
            thread,
            prompt: prompt.to_owned(),
            meta: Meta::default(),
            model: None,
            approval: None,
        };
        let id = self.send(&request).await;
        loop {
            let message = self.receive().await;
            if message["id"] == id {
                return ended(&message);
            }
            if message.get("method").is_some() {
                let note: Notification = serde_json::from_value(message).unwrap();
                for reaction in react(&note) {
                    self.send(&reaction).await;
                }
            }
        }
    }
}

/// JSON-RPC 2.0 over HTTP, signed in through a fake identity provider.
pub(crate) struct OverHttp {
    web: Web,
    served: Served,
    _issuer: Issuer,
}

impl OverHttp {
    pub(crate) async fn new(tested: Tested) -> Self {
        let issuer = Issuer::start().await;
        let signer = Signer::es256("key-1");
        issuer.publish(&signer);
        let served = serve_http(tested, &issuer).await;
        let token = signer.sign(&issuer.claims("ada"));
        Self {
            web: Web::new(&served, Some(token)),
            served,
            _issuer: issuer,
        }
    }

    pub(crate) async fn close(self) {
        self.served.close().await;
    }
}

impl Client for OverHttp {
    async fn call(&mut self, request: Request) -> Result<Value, ApiError> {
        outcome(&self.web.call(&request).await)
    }

    async fn turn(
        &mut self,
        thread: ThreadId,
        prompt: &str,
        react: &mut dyn FnMut(&Notification) -> Reaction,
    ) -> Result<TurnResult, ApiError> {
        let request = Request::TurnStart {
            background: false,
            budget: None,
            thread,
            prompt: prompt.to_owned(),
            meta: Meta::default(),
            model: None,
            approval: None,
        };
        let (message, id) = self.web.message(&request);
        let response = self.web.open(&message, Some("turn/start")).await;
        let streams = response.headers()["content-type"] == "text/event-stream";
        let mut events = Events::new(response);
        if !streams {
            let answer: Value = serde_json::from_str(&events.rest().await).unwrap();
            return Err(outcome(&answer).unwrap_err());
        }
        while let Some(message) = events.next().await {
            if message["id"] == id {
                return ended(&message);
            }
            let note: Notification = serde_json::from_value(message).unwrap();
            for reaction in react(&note) {
                self.web.call(&reaction).await;
            }
        }
        panic!("the stream ended without the turn's response")
    }
}

/// The approval a notification asks for, if it asks for one.
fn asked(note: &Notification) -> Option<glave_event::CallId> {
    let Notification::Event { event, .. } = note else {
        return None;
    };
    match decode(event.clone()).ok()?.body {
        Body::ApprovalRequested(asked) => Some(asked.call_id),
        _ => None,
    }
}

async fn thread_of(client: &mut impl Client) -> ThreadId {
    let reply = client
        .call(Request::ThreadStart {
            space: Some(scope().space),
            background: false,
            worktree: None,
            computer: None,
            model: None,
            approval: None,
        })
        .await
        .unwrap();
    serde_json::from_value(reply["thread"]["id"].clone()).unwrap()
}

/// Runs `scenario` over every transport, each with a fresh server and folder.
macro_rules! over_each_transport {
    ($name:literal, $answers:expr, |$client:ident, $dir:ident| $body:block) => {{
        let $dir = folder(concat!($name, "-in-process"));
        let tested = server(&$dir, $answers, Answering::Clients).await;
        let mut $client = InProcess(tested.clone());
        $body
        tested.shutdown().await;

        let $dir = folder(concat!($name, "-json-rpc"));
        let tested = server(&$dir, $answers, Answering::Clients).await;
        let mut $client = OverJsonRpc::new(tested);
        $body
        $client.close().await;

        let $dir = folder(concat!($name, "-socket"));
        let tested = server(&$dir, $answers, Answering::Clients).await;
        let mut $client = OverJsonRpc::on_socket(tested, $name).await;
        $body
        $client.close().await;

        let $dir = folder(concat!($name, "-http"));
        let tested = server(&$dir, $answers, Answering::Clients).await;
        let mut $client = OverHttp::new(tested).await;
        $body
        $client.close().await;
    }};
}

#[test]
fn a_turn_streams_its_events_and_progress_and_they_are_in_the_log() {
    block_on!(async {
        over_each_transport!(
            "complete",
            vec![answer(Some("Hello there."), None)],
            |client, dir| {
                let thread = thread_of(&mut client).await;
                let (mut streamed, mut said) = (Vec::new(), String::new());
                let mut follow = |note: &Notification| {
                    match note {
                        Notification::Event { event, .. } => {
                            streamed.push(event.kind.as_str().to_owned());
                        }
                        Notification::Live {
                            update: Update::Text { text },
                            ..
                        } => said.push_str(text),
                        _ => {}
                    }
                    Vec::new()
                };
                let turn = client.turn(thread, "hi", &mut follow).await.unwrap();
                assert_eq!(turn.outcome, Outcome::Completed);
                assert_eq!(turn.model_calls, 1);
                assert_eq!(
                    streamed,
                    [
                        "turn.started",
                        "message.added",
                        "message.added",
                        "model.responded",
                        "turn.ended"
                    ]
                );
                assert_eq!(said, "Hello there.");
                let events = client
                    .call(Request::ThreadEvents {
                        thread,
                        after: None,
                        limit: None,
                    })
                    .await
                    .unwrap();
                let logged: Vec<String> = events["events"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|event| event["type"].as_str().unwrap().to_owned())
                    .collect();
                assert!(logged.ends_with(&streamed), "{logged:?}");
                let listed = client
                    .call(Request::ThreadList {
                        space: Some(scope().space),
                        limit: None,
                        show: None,
                        cursor: None,
                    })
                    .await
                    .unwrap();
                assert_eq!(listed["threads"][0]["id"], json!(thread));
                let _ = &dir;
            }
        );
    });
}

#[test]
fn an_approval_answered_by_the_client_lets_the_call_run() {
    block_on!(async {
        let touch = || {
            vec![
                answer(None, Some(("bash", json!({"command": "touch ran.txt"})))),
                answer(Some("Done."), None),
            ]
        };
        over_each_transport!("approve", touch(), |client, dir| {
            let thread = thread_of(&mut client).await;
            let mut streamed = Vec::new();
            let mut react = |note: &Notification| {
                if let Notification::Event { event, .. } = note {
                    streamed.push(event.kind.as_str().to_owned());
                }
                asked(note)
                    .map(|approval| Request::ApprovalDecide {
                        thread,
                        approval,
                        decision: Answer::AllowOnce,
                    })
                    .into_iter()
                    .collect()
            };
            let turn = client.turn(thread, "touch it", &mut react).await.unwrap();
            assert_eq!(turn.outcome, Outcome::Completed);
            let asked = streamed
                .iter()
                .position(|kind| kind == "approval.requested")
                .unwrap();
            assert_eq!(streamed[asked + 1], "approval.decided", "{streamed:?}");
            assert!(dir.join("ran.txt").exists());
        });
    });
}

#[test]
fn a_message_sent_while_a_turn_runs_reaches_the_model_in_the_same_stream() {
    block_on!(async {
        let touch = || {
            vec![
                answer(None, Some(("bash", json!({"command": "touch ran.txt"})))),
                answer(Some("Done."), None),
                // Had the turn ended before the message came, it would start the next one.
                answer(Some("Done again."), None),
            ]
        };
        over_each_transport!("steer", touch(), |client, dir| {
            let thread = thread_of(&mut client).await;
            let mut said = Vec::new();
            // While the turn waits for the person, they send a correction, then allow the call.
            let mut react = |note: &Notification| {
                if let Notification::Event { event, .. } = note
                    && let Body::MessageAdded(added) = decode(event.clone()).unwrap().body
                    && added.role == glave_event::Role::User
                {
                    said.push(serde_json::to_value(&added.parts).unwrap());
                }
                asked(note)
                    .map(|approval| {
                        vec![
                            Request::TurnStart {
                                thread,
                                prompt: "and name it done.txt".into(),
                                background: false,
                                budget: None,
                                meta: Meta::default(),
                                model: None,
                                approval: None,
                            },
                            Request::ApprovalDecide {
                                thread,
                                approval,
                                decision: Answer::AllowOnce,
                            },
                        ]
                    })
                    .unwrap_or_default()
            };
            let turn = client.turn(thread, "touch it", &mut react).await.unwrap();
            assert_eq!(turn.outcome, Outcome::Completed);
            assert!(dir.join("ran.txt").exists());
            let last = said.last().unwrap();
            assert_eq!(last[0]["text"], "and name it done.txt", "{said:?}");
        });
    });
}

#[test]
fn cancelling_a_paused_turn_ends_it_and_the_call_never_runs() {
    block_on!(async {
        let touch = || {
            vec![answer(
                None,
                Some(("bash", json!({"command": "touch ran.txt"}))),
            )]
        };
        over_each_transport!("cancel", touch(), |client, dir| {
            let thread = thread_of(&mut client).await;
            let mut react = |note: &Notification| {
                asked(note)
                    .map(|_| Request::TurnCancel { thread })
                    .into_iter()
                    .collect()
            };
            let turn = client.turn(thread, "touch it", &mut react).await.unwrap();
            assert_eq!(turn.outcome, Outcome::Cancelled);
            assert!(!dir.join("ran.txt").exists());
        });
    });
}

#[test]
fn errors_carry_the_same_codes() {
    block_on!(async {
        over_each_transport!("errors", Vec::new(), |client, dir| {
            let missing = ThreadId::new(jiff::Timestamp::UNIX_EPOCH, [7; 10]).unwrap();
            let failed = client.call(Request::ThreadGet { thread: missing }).await;
            assert_eq!(failed.err().map(|e| e.code), Some(code::NO_THREAD));
            let thread = thread_of(&mut client).await;
            let stale = Request::ApprovalDecide {
                thread,
                approval: glave_event::CallId::new("call_1").unwrap(),
                decision: Answer::Deny,
            };
            assert_eq!(
                client.call(stale).await.err().map(|e| e.code),
                Some(code::NO_APPROVAL)
            );
            let _ = &dir;
        });
    });
}

#[test]
fn models_are_listed_and_one_the_server_does_not_run_is_refused() {
    block_on!(async {
        over_each_transport!("models", Vec::new(), |client, dir| {
            let listed = client.call(Request::ModelList {}).await.unwrap();
            assert_eq!(
                listed,
                json!({"models": [{"id": "test-model", "default": true, "hope": false}]})
            );
            let refused = client
                .call(Request::ThreadStart {
                    space: Some(scope().space),
                    background: false,
                    worktree: None,
                    computer: None,
                    model: Some("elsewhere".into()),
                    approval: None,
                })
                .await
                .unwrap_err();
            assert_eq!(refused.code, code::NO_MODEL);
            assert!(refused.message.contains("elsewhere"), "{refused:?}");
            let _ = &dir;
        });
    });
}

#[test]
fn a_thread_chooses_its_approval_mode_and_its_summary_shows_it() {
    block_on!(async {
        over_each_transport!("modes", Vec::new(), |client, dir| {
            let info = client.call(Request::Discover {}).await.unwrap();
            assert_eq!(info["approval"], json!("ask"));
            assert_eq!(info["approvals"], json!(["ask", "auto"]));
            let started = client
                .call(Request::ThreadStart {
                    space: Some(scope().space),
                    background: false,
                    worktree: None,
                    computer: None,
                    model: None,
                    approval: Some(glave_session::protocol::ApprovalMode::Auto),
                })
                .await
                .unwrap();
            assert_eq!(started["thread"]["approval"], json!("auto"));
            let _ = &dir;
        });
    });
}

#[test]
fn a_background_turn_in_another_space_answers_at_once_and_its_task_is_listed_when_done() {
    block_on!(async {
        over_each_transport!("task", vec![answer(Some("Done."), None)], |client, dir| {
            // A project with no folder, whose threads work in scratch folders of their own.
            let made = client
                .call(Request::SpaceCreate {
                    name: "Notes".into(),
                    members: Vec::new(),
                    folder: None,
                })
                .await
                .unwrap();
            let reply = client
                .call(Request::ThreadStart {
                    space: serde_json::from_value(made["space"]["id"].clone()).unwrap(),
                    background: true,
                    worktree: None,
                    computer: None,
                    model: None,
                    approval: None,
                })
                .await
                .unwrap();
            let thread: ThreadId = serde_json::from_value(reply["thread"]["id"].clone()).unwrap();
            let started = client
                .call(Request::TurnStart {
                    thread,
                    prompt: "go".into(),
                    background: true,
                    budget: Some(10_000),
                    meta: Meta::default(),
                    model: None,
                    approval: None,
                })
                .await
                .unwrap();
            assert_eq!(started["task"]["thread"], json!(thread));
            assert_eq!(started["task"]["kind"], "agent");
            assert_eq!(started["task"]["budget"], 10_000);
            let mut finished = Value::Null;
            for _ in 0..10_000 {
                let listed = client
                    .call(Request::TasksList { limit: None })
                    .await
                    .unwrap();
                if listed["tasks"][0]["status"] == "finished" {
                    finished = listed["tasks"][0].clone();
                    break;
                }
                tokio::task::yield_now().await;
            }
            assert_eq!(finished["outcome"], "completed", "{finished}");
            let _ = &dir;
        });
    });
}
