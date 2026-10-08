//! One set of scenarios, run over every transport: in-process, and JSON-RPC 2.0 over a stream
//! of lines (as stdio carries it). A transport changes how messages travel, never what they
//! mean (`specs/session.md`), so each scenario must end the same way over each.

#[cfg(test)]
#[path = "support/model.rs"]
#[expect(dead_code, reason = "shared helpers; this file uses some of them")]
mod support;

#[cfg(test)]
mod conformance {
    use glave_event::{Body, Outcome, ThreadId, decode};
    use glave_ports::MemoryLog;
    use glave_session::protocol::{
        Answer, ApiError, Notification, Reply, Request, TurnResult, code,
    };
    use glave_session::{Answering, Server, TurnItem};
    use serde_json::{Value, json};
    use tokio::io::{AsyncBufReadExt as _, AsyncWriteExt as _, BufReader, DuplexStream, Lines};
    use tokio::task::JoinHandle;

    use super::support::*;

    type Tested = Server<Script, MemoryLog, glave_ports::MemoryBlobs>;

    /// What a scenario does after a notification: nothing, or send another request.
    type Reaction = Option<Request>;

    /// A client of one transport.
    trait Client {
        async fn call(&mut self, request: Request) -> Result<Value, ApiError>;
        /// Runs a turn, letting `react` answer notifications with requests of its own.
        async fn turn(
            &mut self,
            thread: ThreadId,
            prompt: &str,
            react: &mut dyn FnMut(&Notification) -> Reaction,
        ) -> Result<TurnResult, ApiError>;
    }

    struct InProcess(Tested);

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
            let mut stream = self.0.start_turn(thread, prompt).await?;
            while let Some(item) = stream.next().await {
                match item {
                    TurnItem::Note(note) => {
                        if let Some(request) = react(&note) {
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

    /// JSON-RPC 2.0, one message per line, over an in-memory pipe to `rpc::serve`.
    struct OverJsonRpc {
        to_server: DuplexStream,
        from_server: Lines<BufReader<DuplexStream>>,
        next_id: i64,
        serving: Option<JoinHandle<std::io::Result<()>>>,
    }

    impl OverJsonRpc {
        fn new(server: Tested) -> Self {
            let (client_out, server_in) = tokio::io::duplex(1 << 16);
            let (server_out, client_in) = tokio::io::duplex(1 << 16);
            let serving = tokio::spawn(glave_session::rpc::serve(
                server,
                BufReader::new(server_in),
                server_out,
            ));
            Self {
                to_server: client_out,
                from_server: BufReader::new(client_in).lines(),
                next_id: 0,
                serving: Some(serving),
            }
        }

        async fn send(&mut self, request: &Request) -> i64 {
            self.next_id += 1;
            let mut message = serde_json::to_value(request).unwrap();
            message["jsonrpc"] = json!("2.0");
            message["id"] = json!(self.next_id);
            let line = format!("{message}\n");
            self.to_server.write_all(line.as_bytes()).await.unwrap();
            self.next_id
        }

        async fn receive(&mut self) -> Value {
            let line = self.from_server.next_line().await.unwrap().unwrap();
            serde_json::from_str(&line).unwrap()
        }

        /// Ends the client's input and waits for the server to finish serving it.
        async fn close(mut self) {
            self.to_server.shutdown().await.unwrap();
            drop(self.to_server);
            if let Some(serving) = self.serving.take() {
                serving.await.unwrap().unwrap();
            }
        }
    }

    fn outcome(message: &Value) -> Result<Value, ApiError> {
        assert_eq!(message["jsonrpc"], "2.0");
        match message.get("error") {
            Some(error) => {
                Err(serde_json::from_value(error.clone()).unwrap_or_else(|err| unreadable(&err)))
            }
            None => Ok(message["result"].clone()),
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
                thread,
                prompt: prompt.to_owned(),
            };
            let id = self.send(&request).await;
            loop {
                let message = self.receive().await;
                if message["id"] == id {
                    return outcome(&message).and_then(|result| {
                        serde_json::from_value(result["turn"].clone())
                            .map_err(|err| unreadable(&err))
                    });
                }
                if message.get("method").is_some() {
                    let note: Notification = serde_json::from_value(message).unwrap();
                    if let Some(reaction) = react(&note) {
                        self.send(&reaction).await;
                    }
                }
            }
        }
    }

    /// A message the test could not read, as an error the scenario will show.
    fn unreadable(err: &serde_json::Error) -> ApiError {
        ApiError {
            code: 0,
            message: format!("unreadable message: {err}"),
            data: None,
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
        let reply = client.call(Request::ThreadStart {}).await.unwrap();
        serde_json::from_value(reply["thread"]["id"].clone()).unwrap()
    }

    /// Runs `scenario` over both transports, each with a fresh server and folder.
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
        }};
    }

    #[test]
    fn a_turn_completes_and_its_events_are_in_the_log() {
        block_on(async {
            over_each_transport!(
                "complete",
                vec![answer(Some("Hello."), None)],
                |client, dir| {
                    let thread = thread_of(&mut client).await;
                    let turn = client.turn(thread, "hi", &mut |_| None).await.unwrap();
                    assert_eq!(turn.outcome, Outcome::Completed);
                    let events = client
                        .call(Request::ThreadEvents {
                            thread,
                            after: None,
                            limit: None,
                        })
                        .await
                        .unwrap();
                    let types: Vec<&str> = events["events"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .map(|event| event["type"].as_str().unwrap())
                        .collect();
                    assert_eq!(types.last(), Some(&"turn.ended"), "{types:?}");
                    let _ = &dir;
                }
            );
        });
    }

    #[test]
    fn an_approval_answered_by_the_client_lets_the_call_run() {
        block_on(async {
            let touch = || {
                vec![
                    answer(None, Some(("bash", json!({"command": "touch ran.txt"})))),
                    answer(Some("Done."), None),
                ]
            };
            over_each_transport!("approve", touch(), |client, dir| {
                let thread = thread_of(&mut client).await;
                let mut react = |note: &Notification| {
                    asked(note).map(|approval| Request::ApprovalDecide {
                        thread,
                        approval,
                        decision: Answer::AllowOnce,
                    })
                };
                let turn = client.turn(thread, "touch it", &mut react).await.unwrap();
                assert_eq!(turn.outcome, Outcome::Completed);
                assert!(dir.join("ran.txt").exists());
            });
        });
    }

    #[test]
    fn cancelling_a_paused_turn_ends_it_and_the_call_never_runs() {
        block_on(async {
            let touch = || {
                vec![answer(
                    None,
                    Some(("bash", json!({"command": "touch ran.txt"}))),
                )]
            };
            over_each_transport!("cancel", touch(), |client, dir| {
                let thread = thread_of(&mut client).await;
                let mut react =
                    |note: &Notification| asked(note).map(|_| Request::TurnCancel { thread });
                let turn = client.turn(thread, "touch it", &mut react).await.unwrap();
                assert_eq!(turn.outcome, Outcome::Cancelled);
                assert!(!dir.join("ran.txt").exists());
            });
        });
    }

    #[test]
    fn errors_carry_the_same_codes() {
        block_on(async {
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
}
