//! A whole turn against a scripted model: tools run in a real temporary folder, events go to
//! the in-memory log, and the conversation rebuilt from the log matches what the model saw.

#[cfg(test)]
#[path = "support/script.rs"]
#[expect(dead_code, reason = "shared helpers; this file uses most of them")]
mod support;

#[cfg(test)]
mod scripted {
    use super::support::*;
    use glave_event::{Body, Outcome, Part, RawEvent, StopReason};
    use glave_harness::{AllowAll, Cancel, Live, conversation};
    use glave_ports::{EventLog, MemoryLog, OrgId, Scope, SpaceId};
    use serde_json::json;

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
                .run_turn(&mut thread, "make hello.txt", &Cancel::new(), &mut |live| {
                    seen.push(live);
                })
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
                .run_turn(&mut thread, "clean up", &Cancel::new(), &mut |_| {})
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
    fn the_approval_mode_decides_what_asks() {
        use glave_harness::ApprovalMode;
        for (mode, wrote, ran) in [
            (ApprovalMode::Ask, false, false),
            (ApprovalMode::Edits, true, false),
            (ApprovalMode::Auto, true, true),
        ] {
            let dir = folder(&format!("mode-{}", mode.name()));
            block_on(async {
                let answers = vec![
                    answer(
                        None,
                        vec![
                            ("write", json!({"path": "notes.md", "content": "hi"})),
                            ("bash", json!({"command": "touch ran.txt"})),
                        ],
                        StopReason::ToolUse,
                    ),
                    answer(Some("Done."), vec![], StopReason::EndTurn),
                ];
                let mut settings = settings();
                settings.approval = mode;
                let harness =
                    harness_on(&dir, answers, Refuse, MemoryLog::new(), 1, settings).await;
                let mut thread = harness.start_thread(scope()).await.unwrap();
                harness
                    .run_turn(&mut thread, "go", &Cancel::new(), &mut |_| {})
                    .await
                    .unwrap();
            });
            assert_eq!(dir.join("notes.md").exists(), wrote, "{mode:?} write");
            assert_eq!(dir.join("ran.txt").exists(), ran, "{mode:?} command");
        }
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
                .run_turn(&mut thread, "say hello", &Cancel::new(), &mut |_| {})
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
                .run_turn(&mut thread, "run it", &Cancel::new(), &mut |_| {})
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
                .run_turn(&mut thread, "loop", &Cancel::new(), &mut |_| {})
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

    #[test]
    fn a_resumed_thread_continues_where_it_left_off() {
        let dir = folder("resume");
        block_on(async {
            let first = vec![answer(Some("Hello."), vec![], StopReason::EndTurn)];
            let harness = harness(&dir, first, AllowAll).await;
            let mut thread = harness.start_thread(scope()).await.unwrap();
            harness
                .run_turn(&mut thread, "hi", &Cancel::new(), &mut |_| {})
                .await
                .unwrap();

            let raw = harness
                .log()
                .read(&scope(), thread.id, None, usize::MAX)
                .await
                .unwrap();
            let log = MemoryLog::new();
            log.append(&scope(), &raw).await.unwrap();
            let later = vec![answer(Some("Again."), vec![], StopReason::EndTurn)];
            let resumed_harness = harness_on(&dir, later, AllowAll, log, 1_000, settings()).await;
            let mut resumed = resumed_harness
                .resume_thread(scope(), thread.id)
                .await
                .unwrap();
            assert_eq!(resumed.messages(), thread.messages());

            resumed_harness
                .run_turn(&mut resumed, "again", &Cancel::new(), &mut |_| {})
                .await
                .unwrap();
            let events = events(resumed_harness.log(), thread.id).await;
            let seqs: Vec<u64> = events.iter().map(|e| e.header.seq.get()).collect();
            assert_eq!(
                seqs,
                (1..=u64::try_from(events.len()).unwrap()).collect::<Vec<_>>()
            );
            assert_eq!(conversation(&events), resumed.messages());
        });
    }

    #[test]
    fn resuming_after_a_crash_closes_what_was_left_open() {
        let dir = folder("crash");
        block_on(async {
            let answers = vec![
                answer(
                    None,
                    vec![("bash", json!({"command": "true"}))],
                    StopReason::ToolUse,
                ),
                answer(Some("ok"), vec![], StopReason::EndTurn),
            ];
            let harness = harness(&dir, answers, AllowAll).await;
            let mut thread = harness.start_thread(scope()).await.unwrap();
            harness
                .run_turn(&mut thread, "go", &Cancel::new(), &mut |_| {})
                .await
                .unwrap();

            // Keep only what was written before the tool finished: a crash mid-call.
            let raw = harness
                .log()
                .read(&scope(), thread.id, None, usize::MAX)
                .await
                .unwrap();
            let cut: Vec<RawEvent> = raw.into_iter().take(5).collect();
            let log = MemoryLog::new();
            log.append(&scope(), &cut).await.unwrap();
            let resumed_harness = harness_on(&dir, vec![], AllowAll, log, 1_000, settings()).await;
            let resumed = resumed_harness
                .resume_thread(scope(), thread.id)
                .await
                .unwrap();

            let events = events(resumed_harness.log(), thread.id).await;
            assert_eq!(
                kinds(&events),
                [
                    "thread.started",
                    "turn.started",
                    "message.added",
                    "tool.call.requested",
                    "model.responded",
                    "tool.call.completed",
                    "turn.ended",
                ]
            );
            assert!(matches!(events.last().map(|e| &e.body),
                Some(Body::TurnEnded(ended)) if ended.outcome == Outcome::Interrupted));
            assert!(matches!(
                resumed.messages().last(),
                Some(glave_ports::Message::Tool { is_error: true, .. })
            ));
            assert_eq!(conversation(&events), resumed.messages());
        });
    }

    #[test]
    fn resuming_a_thread_that_does_not_exist_is_an_error() {
        let dir = folder("missing");
        block_on(async {
            let harness = harness(&dir, vec![], AllowAll).await;
            let thread = harness.start_thread(scope()).await.unwrap();
            let elsewhere = Scope {
                org: OrgId::from_uuid(uuid_like(9)),
                space: SpaceId::from_uuid(uuid_like(9)),
            };
            assert!(harness.resume_thread(elsewhere, thread.id).await.is_err());
        });
    }
}
