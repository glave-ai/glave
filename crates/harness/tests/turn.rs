//! A whole turn against a scripted model: tools run in a real temporary folder, events go to
//! the in-memory log, and the conversation rebuilt from the log matches what the model saw.

#[cfg(test)]
mod scripted {
    use std::collections::VecDeque;
    use std::future::{Future, ready};
    use std::path::PathBuf;
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};

    use glave_event::{
        Arguments, Body, Event, ModelRef, Outcome, Part, RawEvent, StopReason, TokenCounts, Usage,
        decode,
    };
    use glave_harness::{
        AllowAll, ApprovalRequest, Approver, Clock, Decision, Entropy, Harness, Live, Parts,
        Settings, Workspace, conversation,
    };
    use glave_ports::{
        Completion, Delta, EventLog, MemoryBlobs, MemoryLog, ModelProvider, ModelRequest, OrgId,
        ProviderError, RequestedCall, Scope, SpaceId,
    };
    use jiff::Timestamp;
    use serde_json::json;

    struct Script {
        answers: Mutex<VecDeque<Completion>>,
        seen: Mutex<Vec<ModelRequest>>,
    }

    impl ModelProvider for Script {
        fn complete(
            &self,
            request: &ModelRequest,
            on_delta: &mut (dyn FnMut(Delta) + Send),
        ) -> impl Future<Output = Result<Completion, ProviderError>> + Send {
            self.seen.lock().unwrap().push(request.clone());
            let next = self.answers.lock().unwrap().pop_front();
            if let Some(Part::Text { text }) = next.as_ref().and_then(|c| c.parts.first()) {
                on_delta(Delta::Text(text.clone()));
            }
            ready(next.ok_or_else(|| ProviderError::Transport("script exhausted".into())))
        }
    }

    struct Ticking(AtomicI64);
    impl Clock for Ticking {
        fn now(&self) -> Timestamp {
            let ms = self.0.fetch_add(1, Ordering::SeqCst);
            Timestamp::from_millisecond(ms).unwrap()
        }
    }

    struct Counting(AtomicU64);
    impl Entropy for Counting {
        fn bytes(&self) -> [u8; 10] {
            let n = self.0.fetch_add(1, Ordering::SeqCst).to_be_bytes();
            [n[0], n[1], n[2], n[3], n[4], n[5], n[6], n[7], 0, 0]
        }
    }

    struct Refuse;
    impl Approver for Refuse {
        fn decide(&self, _request: &ApprovalRequest) -> impl Future<Output = Decision> + Send {
            ready(Decision::Deny)
        }
    }

    fn answer(
        text: Option<&str>,
        calls: Vec<(&str, serde_json::Value)>,
        stop: StopReason,
    ) -> Completion {
        let counts = TokenCounts {
            input: 100,
            cache_read: 40,
            cache_write: 0,
            output: 10,
            reasoning: 0,
        };
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
            tool_calls: calls
                .into_iter()
                .map(|(tool, args)| RequestedCall {
                    provider_call_id: Some(format!("p_{tool}")),
                    tool: tool.into(),
                    arguments: Arguments::Json(args.as_object().unwrap().clone()),
                })
                .collect(),
            stop,
            provider_stop: None,
            usage: Usage::new(counts, false, None).unwrap(),
            cost: None,
        }
    }

    fn folder(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("glave-turn-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn scope() -> Scope {
        Scope {
            org: OrgId::from_uuid(uuid_like(1)),
            space: SpaceId::from_uuid(uuid_like(2)),
        }
    }

    fn uuid_like(n: u8) -> uuid::Uuid {
        uuid::Uuid::from_bytes([n; 16])
    }

    fn block_on<F: Future>(future: F) -> F::Output {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(future)
    }

    async fn harness<A: Approver>(
        dir: &PathBuf,
        answers: Vec<Completion>,
        approver: A,
    ) -> Harness<Script, MemoryLog, MemoryBlobs, A> {
        let script = Script {
            answers: Mutex::new(answers.into()),
            seen: Mutex::new(Vec::new()),
        };
        let parts = Parts {
            provider: script,
            log: MemoryLog::new(),
            blobs: MemoryBlobs::new(),
            approver,
            clock: Box::new(Ticking(AtomicI64::new(1_791_000_000_000))),
            entropy: Box::new(Counting(AtomicU64::new(1))),
            workspace: Workspace::open(dir).await.unwrap(),
        };
        Harness::new(parts, Settings::new("test-model", "script"))
    }

    async fn events<L: EventLog>(log: &L, thread: glave_event::ThreadId) -> Vec<Event> {
        let raw: Vec<RawEvent> = log.read(&scope(), thread, None, usize::MAX).await.unwrap();
        raw.into_iter().map(|e| decode(e).unwrap()).collect()
    }

    fn kinds(events: &[Event]) -> Vec<&'static str> {
        events
            .iter()
            .map(|e| match e.body {
                Body::ThreadStarted(_) => "thread.started",
                Body::TurnStarted(_) => "turn.started",
                Body::MessageAdded(_) => "message.added",
                Body::ToolCallRequested(_) => "tool.call.requested",
                Body::ToolCallCompleted(_) => "tool.call.completed",
                Body::ModelResponded(_) => "model.responded",
                Body::TurnEnded(_) => "turn.ended",
                Body::Unknown(_) => "unknown",
            })
            .collect()
    }

    #[test]
    fn a_turn_writes_a_file_and_records_every_step() {
        let dir = folder("write");
        block_on(async {
            let answers = vec![
                answer(
                    None,
                    vec![("write", json!({"path": "hello.txt", "content": "hi\n"}))],
                    StopReason::ToolUse,
                ),
                answer(Some("Created hello.txt."), vec![], StopReason::EndTurn),
            ];
            let harness = harness(&dir, answers, AllowAll).await;
            let mut thread = harness.start_thread(scope()).await.unwrap();
            let mut seen = Vec::new();
            let summary = harness
                .run_turn(&mut thread, "make hello.txt", &mut |live| seen.push(live))
                .await
                .unwrap();

            assert_eq!(
                std::fs::read_to_string(dir.join("hello.txt")).unwrap(),
                "hi\n"
            );
            assert_eq!(summary.outcome, Outcome::Completed);
            assert_eq!((summary.model_calls, summary.tool_calls), (2, 1));
            assert_eq!(summary.tokens.input, 200);
            assert_eq!(summary.cost, None);
            assert!(seen.contains(&Live::Text("Created hello.txt.".into())));

            let events = events(harness.log(), thread.id).await;
            assert_eq!(
                kinds(&events),
                [
                    "thread.started",
                    "turn.started",
                    "message.added",
                    "tool.call.requested",
                    "model.responded",
                    "tool.call.completed",
                    "message.added",
                    "model.responded",
                    "turn.ended",
                ]
            );
            let seqs: Vec<u64> = events.iter().map(|e| e.header.seq.get()).collect();
            assert_eq!(seqs, (1..=9).collect::<Vec<_>>());
            assert_eq!(conversation(&events), thread.messages());
        });
    }

    #[test]
    fn a_declined_action_is_reported_to_the_model() {
        let dir = folder("deny");
        block_on(async {
            let answers = vec![
                answer(
                    None,
                    vec![("bash", json!({"command": "rm -rf build"}))],
                    StopReason::ToolUse,
                ),
                answer(Some("Understood."), vec![], StopReason::EndTurn),
            ];
            let harness = harness(&dir, answers, Refuse).await;
            let mut thread = harness.start_thread(scope()).await.unwrap();
            harness
                .run_turn(&mut thread, "clean up", &mut |_| {})
                .await
                .unwrap();
            let declined = thread.messages().iter().any(|m| {
                matches!(m,
                glave_ports::Message::Tool { is_error: true, output, .. }
                    if matches!(output.first(), Some(Part::Text { text }) if text.contains("declined")))
            });
            assert!(declined);
        });
    }

    #[test]
    fn read_before_edit_and_edits_land() {
        let dir = folder("edit");
        std::fs::write(
            dir.join("main.rs"),
            "fn main() {\n    println!(\"hi\");\n}\n",
        )
        .unwrap();
        block_on(async {
            let answers = vec![
                answer(
                    None,
                    vec![(
                        "edit",
                        json!({"path": "main.rs", "old_string": "\"hi\"", "new_string": "\"hello\""}),
                    )],
                    StopReason::ToolUse,
                ),
                answer(
                    None,
                    vec![("read", json!({"path": "main.rs"}))],
                    StopReason::ToolUse,
                ),
                answer(
                    None,
                    vec![(
                        "edit",
                        json!({"path": "main.rs", "old_string": "\"hi\"", "new_string": "\"hello\""}),
                    )],
                    StopReason::ToolUse,
                ),
                answer(Some("Done."), vec![], StopReason::EndTurn),
            ];
            let harness = harness(&dir, answers, AllowAll).await;
            let mut thread = harness.start_thread(scope()).await.unwrap();
            harness
                .run_turn(&mut thread, "say hello", &mut |_| {})
                .await
                .unwrap();
            assert!(
                std::fs::read_to_string(dir.join("main.rs"))
                    .unwrap()
                    .contains("\"hello\"")
            );
            let results: Vec<bool> = thread
                .messages()
                .iter()
                .filter_map(|m| match m {
                    glave_ports::Message::Tool { is_error, .. } => Some(*is_error),
                    _ => None,
                })
                .collect();
            assert_eq!(
                results,
                [true, false, false],
                "edit before read must fail first"
            );
        });
    }

    #[test]
    fn bash_reports_output_and_exit_code() {
        let dir = folder("bash");
        block_on(async {
            let answers = vec![
                answer(
                    None,
                    vec![("bash", json!({"command": "echo out; echo err >&2; exit 3"}))],
                    StopReason::ToolUse,
                ),
                answer(Some("It failed."), vec![], StopReason::EndTurn),
            ];
            let harness = harness(&dir, answers, AllowAll).await;
            let mut thread = harness.start_thread(scope()).await.unwrap();
            harness
                .run_turn(&mut thread, "run it", &mut |_| {})
                .await
                .unwrap();
            let Some(glave_ports::Message::Tool {
                is_error, output, ..
            }) = thread.messages().get(2)
            else {
                panic!("expected a tool result");
            };
            let Some(Part::Text { text }) = output.first() else {
                panic!("expected text")
            };
            assert!(*is_error);
            assert!(text.contains("out\nerr"), "{text}");
            assert!(text.contains("[exit 3 ·"), "{text}");
        });
    }

    #[test]
    fn a_model_stuck_repeating_itself_is_stopped() {
        let dir = folder("repeat");
        block_on(async {
            let same = || {
                answer(
                    None,
                    vec![("bash", json!({"command": "true"}))],
                    StopReason::ToolUse,
                )
            };
            let answers = vec![
                same(),
                same(),
                same(),
                answer(Some("ok"), vec![], StopReason::EndTurn),
            ];
            let harness = harness(&dir, answers, AllowAll).await;
            let mut thread = harness.start_thread(scope()).await.unwrap();
            harness
                .run_turn(&mut thread, "loop", &mut |_| {})
                .await
                .unwrap();
            let errors = thread
                .messages()
                .iter()
                .filter(|m| matches!(m, glave_ports::Message::Tool { is_error: true, .. }))
                .count();
            assert_eq!(errors, 1, "the third identical call is refused");
        });
    }
}
