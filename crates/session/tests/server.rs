//! The session server against a scripted model: threads and turns through its methods, the
//! order of a turn's notifications, one turn at a time, cancelling, approvals answered by a
//! client or by no one, and a fresh server picking a thread up from the log alone.

#[cfg(test)]
#[path = "support/model.rs"]
#[expect(dead_code, reason = "shared helpers; this file uses some of them")]
mod support;

#[cfg(test)]
mod session {
    use std::sync::Arc;

    use glave_event::{Body, Outcome, Verdict, decode};
    use glave_ports::{EventLog, MemoryLog, ModelProvider};
    use glave_session::protocol::{
        Answer, ApiError, IdempotencyKey, Meta, Notification, Reply, Request, Status, Update, code,
    };
    use glave_session::{Answering, Caller, Server, TurnItem};
    use jiff::Timestamp;
    use serde_json::json;
    use tokio::sync::Notify;

    use super::support::*;

    /// Keys under a key file beside `database`, so a second store on it reads the first's.
    fn keys(database: &std::path::Path) -> glave_keys::KeySettings {
        let root = glave_keys::KeyFileRoot::open_or_create(database.with_extension("key")).unwrap();
        glave_keys::KeySettings::new(Arc::new(root))
    }

    async fn new_thread<P, L, B>(server: &Server<P, L, B>) -> glave_event::ThreadId
    where
        P: ModelProvider + 'static,
        L: EventLog + 'static,
        B: glave_ports::BlobStore + 'static,
    {
        match server
            .request(Request::ThreadStart { worktree: None })
            .await
            .unwrap()
        {
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
                .request(Request::ThreadList {
                    limit: None,
                    show: None,
                })
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
    fn a_retry_with_the_same_key_follows_the_running_turn_instead_of_starting_one() {
        let dir = folder("retry-running");
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
            let key = IdempotencyKey::new("attempt-1").unwrap();
            let local = Caller::default();
            let start = |key| server.start_turn_as(&local, thread, "slow", key);
            let mut first = start(Some(&key)).await.unwrap();
            let mut retried = start(Some(&key)).await.unwrap();
            let other = IdempotencyKey::new("attempt-2").unwrap();
            let refused = start(Some(&other)).await;
            assert_eq!(refused.err().map(|e| e.code), Some(code::TURN_RUNNING));
            gate.notify_one();
            let (first, retried) = (first.finish().await, retried.finish().await);
            assert_eq!(first.unwrap(), retried.unwrap());
            // Once ended, the key still names that turn.
            let mut late = start(Some(&key)).await.unwrap();
            assert_eq!(late.finish().await.unwrap().outcome, Outcome::Completed);
            assert!(IdempotencyKey::new("").is_err());
            assert!(IdempotencyKey::new("k".repeat(256)).is_err());
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
                    meta: Meta::default(),
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
            let first_store = glave_store_sqlite::Store::open(&database, keys(&database))
                .await
                .unwrap();
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
                    meta: Meta::default(),
                })
                .await
                .unwrap();
            first.shutdown().await;

            let second_store = glave_store_sqlite::Store::open(&database, keys(&database))
                .await
                .unwrap();
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
                    meta: Meta::default(),
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
            meta: Meta::default(),
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
