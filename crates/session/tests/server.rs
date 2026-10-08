//! The session server against a scripted model: threads and turns through its methods, the
//! order of a turn's notifications, one turn at a time, cancelling, approvals answered by a
//! client or by no one, and a fresh server picking a thread up from the log alone.

#[cfg(test)]
mod session {
    use std::collections::VecDeque;
    use std::future::Future;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};
    use std::sync::{Arc, Mutex};

    use glave_event::{
        Arguments, Body, ModelRef, Outcome, Part, StopReason, TokenCounts, Usage, Verdict, decode,
    };
    use glave_harness::{Clock, Entropy, Settings, Workspace};
    use glave_ports::{
        Completion, Delta, EventLog, MemoryBlobs, MemoryLog, ModelProvider, ModelRequest, OrgId,
        ProviderError, RequestedCall, Scope, SpaceId,
    };
    use glave_session::protocol::{
        Answer, ApiError, Notification, Reply, Request, Status, Update, code,
    };
    use glave_session::{Answering, Server, TurnItem, Wiring};
    use jiff::Timestamp;
    use serde_json::json;
    use tokio::sync::Notify;

    /// A model that answers from a script, optionally waiting at a gate before each answer.
    struct Script {
        answers: Mutex<VecDeque<Completion>>,
        gate: Option<Arc<Notify>>,
    }

    impl ModelProvider for Script {
        fn complete(
            &self,
            _request: &ModelRequest,
            on_delta: &mut (dyn FnMut(Delta) + Send),
        ) -> impl Future<Output = Result<Completion, ProviderError>> + Send {
            let next = self.answers.lock().unwrap().pop_front();
            if let Some(Part::Text { text }) = next.as_ref().and_then(|c| c.parts.first()) {
                on_delta(Delta::Text(text.clone()));
            }
            let gate = self.gate.clone();
            async move {
                if let Some(gate) = gate {
                    gate.notified().await;
                }
                next.ok_or_else(|| ProviderError::Protocol("script exhausted".into()))
            }
        }
    }

    struct Ticking(AtomicI64);
    impl Clock for Ticking {
        fn now(&self) -> Timestamp {
            Timestamp::from_millisecond(self.0.fetch_add(1, Ordering::SeqCst)).unwrap()
        }
    }

    struct Counting(AtomicU64);
    impl Entropy for Counting {
        fn bytes(&self) -> [u8; 10] {
            let n = self.0.fetch_add(1, Ordering::SeqCst).to_be_bytes();
            [n[0], n[1], n[2], n[3], n[4], n[5], n[6], n[7], 0, 0]
        }
    }

    fn answer(text: Option<&str>, call: Option<(&str, serde_json::Value)>) -> Completion {
        Completion {
            model: ModelRef {
                provider: "script".into(),
                name: "test-model".into(),
            },
            response_id: None,
            parts: text
                .map(|t| Part::Text { text: t.into() })
                .into_iter()
                .collect(),
            tool_calls: call
                .map(|(tool, args)| RequestedCall {
                    provider_call_id: None,
                    tool: tool.into(),
                    arguments: Arguments::Json(args.as_object().unwrap().clone()),
                })
                .into_iter()
                .collect(),
            stop: if text.is_some() {
                StopReason::EndTurn
            } else {
                StopReason::ToolUse
            },
            provider_stop: None,
            usage: Usage::new(TokenCounts::default(), false, None).unwrap(),
            cost: None,
        }
    }

    fn folder(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("glave-session-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn scope() -> Scope {
        Scope {
            org: OrgId::from_uuid(uuid::Uuid::from_u128(1)),
            space: SpaceId::from_uuid(uuid::Uuid::from_u128(2)),
        }
    }

    fn block_on<F: Future>(future: F) -> F::Output {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(future)
    }

    async fn server_on<L: EventLog + 'static>(
        dir: &PathBuf,
        answers: Vec<Completion>,
        gate: Option<Arc<Notify>>,
        log: L,
        answering: Answering,
        seed: u64,
    ) -> Server<Script, L, MemoryBlobs> {
        let wiring = Wiring {
            provider: Script {
                answers: Mutex::new(answers.into()),
                gate,
            },
            log,
            blobs: MemoryBlobs::new(),
            clock: Box::new(Ticking(AtomicI64::new(
                1_791_000_000_000 + 1_000_000 * i64::try_from(seed).unwrap(),
            ))),
            entropy: Box::new(Counting(AtomicU64::new(seed << 32))),
            workspace: Workspace::open(dir, std::env::vars_os()).await.unwrap(),
        };
        Server::new(
            wiring,
            Settings::new("test-model", "script"),
            scope(),
            answering,
        )
    }

    async fn server(
        dir: &PathBuf,
        answers: Vec<Completion>,
        answering: Answering,
    ) -> Server<Script, MemoryLog, MemoryBlobs> {
        server_on(dir, answers, None, MemoryLog::new(), answering, 1).await
    }

    async fn new_thread<P, L, B>(server: &Server<P, L, B>) -> glave_event::ThreadId
    where
        P: ModelProvider + 'static,
        L: EventLog + 'static,
        B: glave_ports::BlobStore + 'static,
    {
        match server.request(Request::ThreadStart {}).await.unwrap() {
            Reply::Thread { thread } => {
                assert_eq!(thread.status, Status::Idle);
                thread.id
            }
            other => panic!("expected a thread, got {other:?}"),
        }
    }

    /// The type of every stored event in the notifications, in order.
    fn event_types(items: &[TurnItem]) -> Vec<String> {
        items
            .iter()
            .filter_map(|item| match item {
                TurnItem::Note(note) => match note.as_ref() {
                    Notification::Event { event, .. } => Some(event.kind.as_str().to_owned()),
                    Notification::Live { .. } => None,
                },
                _ => None,
            })
            .collect()
    }

    async fn follow(stream: &mut glave_session::TurnStream) -> Vec<TurnItem> {
        let mut items = Vec::new();
        while let Some(item) = stream.next().await {
            items.push(item);
        }
        items
    }

    #[test]
    fn a_turn_streams_its_events_and_progress_then_its_ending() {
        let dir = folder("stream");
        block_on(async {
            let answers = vec![answer(Some("Hello there."), None)];
            let server = server(&dir, answers, Answering::NoOne).await;
            let thread = new_thread(&server).await;
            let mut stream = server.start_turn(thread, "hi").await.unwrap();
            let items = follow(&mut stream).await;
            assert_eq!(
                event_types(&items),
                [
                    "turn.started",
                    "message.added",
                    "message.added",
                    "model.responded",
                    "turn.ended"
                ]
            );
            assert!(items.iter().any(|item| matches!(item, TurnItem::Note(note)
                if matches!(note.as_ref(), Notification::Live { update: Update::Text { text }, .. } if text == "Hello there."))));
            let Some(TurnItem::Ended(Ok(turn))) = items.last() else {
                panic!("the ending comes last: {items:?}");
            };
            assert_eq!(turn.outcome, Outcome::Completed);
            assert_eq!(turn.model_calls, 1);
            match server
                .request(Request::ThreadList { limit: None })
                .await
                .unwrap()
            {
                Reply::Threads { threads } => assert_eq!(threads[0].id, thread),
                other => panic!("{other:?}"),
            }
        });
    }

    #[test]
    fn one_turn_runs_at_a_time_and_cancel_ends_it() {
        let dir = folder("one-at-a-time");
        block_on(async {
            let gate = Arc::new(Notify::new());
            let answers = vec![answer(Some("Slow."), None)];
            let server = server_on(
                &dir,
                answers,
                Some(Arc::clone(&gate)),
                MemoryLog::new(),
                Answering::NoOne,
                1,
            )
            .await;
            let thread = new_thread(&server).await;
            let mut first = server.start_turn(thread, "slow").await.unwrap();
            let second = server.start_turn(thread, "again").await;
            assert_eq!(second.err().map(|e| e.code), Some(code::TURN_RUNNING));
            match server.request(Request::ThreadGet { thread }).await.unwrap() {
                Reply::Thread { thread } => assert_eq!(thread.status, Status::Running),
                other => panic!("{other:?}"),
            }
            let cancelled = server
                .request(Request::TurnCancel { thread })
                .await
                .unwrap();
            assert!(matches!(cancelled, Reply::Done {}));
            let ended = first.finish().await.unwrap();
            assert_eq!(ended.outcome, Outcome::Cancelled);
            let again = server.request(Request::TurnCancel { thread }).await;
            assert_eq!(again.err().map(|e| e.code), Some(code::NO_TURN));
        });
    }

    #[test]
    fn a_client_answers_an_approval_and_the_turn_continues() {
        let dir = folder("approve");
        block_on(async {
            let answers = vec![
                answer(None, Some(("bash", json!({"command": "touch ran.txt"})))),
                answer(Some("Done."), None),
            ];
            let server = server(&dir, answers, Answering::Clients).await;
            let thread = new_thread(&server).await;
            let mut stream = server.start_turn(thread, "touch it").await.unwrap();
            let mut seen = Vec::new();
            while let Some(item) = stream.next().await {
                if let TurnItem::Note(note) = &item
                    && let Notification::Event { event, .. } = note.as_ref()
                    && let Body::ApprovalRequested(asked) = decode(event.clone()).unwrap().body
                {
                    match server.request(Request::ThreadGet { thread }).await.unwrap() {
                        Reply::Thread { thread } => assert_eq!(thread.status, Status::Waiting),
                        other => panic!("{other:?}"),
                    }
                    let decide = Request::ApprovalDecide {
                        thread,
                        approval: asked.call_id.clone(),
                        decision: Answer::AllowOnce,
                    };
                    server.request(decide).await.unwrap();
                }
                seen.push(item);
            }
            assert!(dir.join("ran.txt").exists());
            assert!(
                matches!(seen.last(), Some(TurnItem::Ended(Ok(turn))) if turn.outcome == Outcome::Completed)
            );
            let types = event_types(&seen);
            let asked = types
                .iter()
                .position(|t| t == "approval.requested")
                .unwrap();
            assert_eq!(types[asked + 1], "approval.decided");
            let stale = Request::ApprovalDecide {
                thread,
                approval: glave_event::CallId::new("call_99").unwrap(),
                decision: Answer::Deny,
            };
            assert_eq!(
                server.request(stale).await.err().map(|e| e.code),
                Some(code::NO_APPROVAL)
            );
        });
    }

    #[test]
    fn with_no_one_to_answer_the_call_is_not_run() {
        let dir = folder("nobody");
        block_on(async {
            let answers = vec![
                answer(None, Some(("bash", json!({"command": "touch ran.txt"})))),
                answer(Some("I could not."), None),
            ];
            let server = server(&dir, answers, Answering::NoOne).await;
            let thread = new_thread(&server).await;
            let reply = server
                .request(Request::TurnStart {
                    thread,
                    prompt: "touch it".into(),
                })
                .await
                .unwrap();
            assert!(matches!(reply, Reply::Turn { turn } if turn.outcome == Outcome::Completed));
            assert!(!dir.join("ran.txt").exists());
            let Reply::Events { events, .. } = server
                .request(Request::ThreadEvents {
                    thread,
                    after: None,
                    limit: None,
                })
                .await
                .unwrap()
            else {
                panic!("expected events");
            };
            let decided: Vec<Verdict> = events
                .into_iter()
                .filter_map(|raw| match decode(raw).unwrap().body {
                    Body::ApprovalDecided(decided) => Some(decided.decision),
                    _ => None,
                })
                .collect();
            assert_eq!(decided, [Verdict::NoOneThere]);
        });
    }

    #[test]
    fn a_fresh_server_continues_a_thread_from_the_log_alone() {
        let dir = folder("stateless");
        block_on(async {
            let database = dir.join("glave.db");
            let first_store = glave_store_sqlite::Store::open(&database).await.unwrap();
            let first = server_on(
                &dir,
                vec![answer(Some("One."), None)],
                None,
                first_store.log(),
                Answering::NoOne,
                1,
            )
            .await;
            let thread = new_thread(&first).await;
            first
                .request(Request::TurnStart {
                    thread,
                    prompt: "one".into(),
                })
                .await
                .unwrap();
            first.shutdown().await;

            let second_store = glave_store_sqlite::Store::open(&database).await.unwrap();
            let second = server_on(
                &dir,
                vec![answer(Some("Two."), None)],
                None,
                second_store.log(),
                Answering::NoOne,
                2,
            )
            .await;
            let reply = second
                .request(Request::TurnStart {
                    thread,
                    prompt: "two".into(),
                })
                .await
                .unwrap();
            assert!(matches!(reply, Reply::Turn { turn } if turn.outcome == Outcome::Completed));
            let Reply::Events { events, next } = second
                .request(Request::ThreadEvents {
                    thread,
                    after: None,
                    limit: Some(3),
                })
                .await
                .unwrap()
            else {
                panic!("expected events");
            };
            assert_eq!(events.len(), 3);
            assert_eq!(
                next,
                events.last().map(|e| e.seq),
                "a full page says where to go on"
            );
        });
    }

    #[test]
    fn an_unknown_thread_is_named_as_such() {
        let dir = folder("unknown");
        block_on(async {
            let server = server(&dir, Vec::new(), Answering::NoOne).await;
            let missing = glave_event::ThreadId::new(Timestamp::UNIX_EPOCH, [9; 10]).unwrap();
            let error: Option<ApiError> = server
                .request(Request::ThreadGet { thread: missing })
                .await
                .err();
            assert_eq!(error.map(|e| e.code), Some(code::NO_THREAD));
        });
    }

    #[test]
    fn messages_have_the_shape_the_spec_gives() {
        let thread = glave_event::ThreadId::new(Timestamp::UNIX_EPOCH, [1; 10]).unwrap();
        let request = Request::TurnStart {
            thread,
            prompt: "hi".into(),
        };
        let wire = serde_json::to_value(&request).unwrap();
        assert_eq!(wire["method"], "turn/start");
        assert_eq!(wire["params"]["prompt"], "hi");
        let parsed: Request = serde_json::from_value(json!({
            "method": "approval/decide",
            "params": {"thread": thread, "approval": "call_7", "decision": "allow_for_thread",
                       "_meta": {"protocol": "2026-10"}}
        }))
        .unwrap();
        assert!(matches!(
            parsed,
            Request::ApprovalDecide {
                decision: Answer::AllowForThread,
                ..
            }
        ));
        let live = Notification::Live {
            thread,
            update: Update::Retrying {
                retry: 1,
                of: 4,
                wait_ms: 1000,
                error: "503".into(),
            },
        };
        let wire = serde_json::to_value(&live).unwrap();
        assert_eq!(wire["method"], "turn/live");
        assert_eq!(wire["params"]["update"]["kind"], "retrying");
    }
}
