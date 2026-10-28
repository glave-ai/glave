//! A whole turn against a scripted model: tools run in a real temporary folder, events go to
//! the in-memory log, and the conversation rebuilt from the log matches what the model saw.

use crate::support::*;
use glave_event::{Body, Outcome, Part, RawEvent, StopReason, ToolKind};
use glave_harness::{AllowAll, Cancel, Live, conversation};
use glave_ports::{EventLog, InMemoryLog, OrgId, ProjectId, Scope};
use serde_json::json;

#[test]
fn a_random_source_that_fails_makes_no_thread() {
    let dir = folder("no-entropy");
    block_on!(async {
        let parts = glave_harness::Parts {
            provider: ScriptedModel::new(Vec::new()),
            log: InMemoryLog::new(),
            blobs: glave_ports::InMemoryBlobs::new(),
            approver: AllowAll,
            clock: Box::new(glave_ports::testing::ManualClock::ticking(
                1_791_000_000_000,
            )),
            entropy: Box::new(glave_ports::testing::FailingEntropy),
            workspace: workspace(&dir).await,
        };
        let harness = glave_harness::Harness::new(parts, settings());
        let started = harness.start_thread(scope()).await;
        assert!(
            matches!(started, Err(glave_harness::HarnessError::Entropy(_))),
            "an id with no randomness is never made: {:?}",
            started.err()
        );
    });
}

#[test]
fn a_turn_writes_a_file_and_records_every_step() {
    let dir = folder("write");
    block_on!(async {
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
        assert!(
            matches!(&events[8].body, Body::TurnEnded(ended) if *ended == summary),
            "the turn's end records what it answered with"
        );
    });
}

#[test]
fn each_step_is_recorded_in_the_line_it_began_with() {
    let dir = folder("step-lines");
    block_on!(async {
        let answers = vec![
            answer(
                None,
                vec![
                    ("write", json!({"path": "a.txt", "content": "one\ntwo\n"})),
                    ("bash", json!({"command": "cat a.txt"})),
                    ("nothing", json!({"x": 1})),
                ],
                StopReason::ToolUse,
            ),
            answer(Some("Done."), vec![], StopReason::EndTurn),
        ];
        let harness = harness(&dir, answers, AllowAll).await;
        let mut thread = harness.start_thread(scope()).await.unwrap();
        let mut began = Vec::new();
        harness
            .run_turn(&mut thread, "write and read", &Cancel::new(), &mut |live| {
                if let Live::ToolStarted { summary } = live {
                    began.push(summary);
                }
            })
            .await
            .unwrap();

        let events = events(harness.log(), thread.id).await;
        let recorded: Vec<&str> = events
            .iter()
            .filter_map(|event| match &event.body {
                Body::ToolCallRequested(call) => Some(call.summary.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(
            recorded,
            [
                "write a.txt (2 lines)",
                "bash: cat a.txt",
                r#"nothing {"x":1}"#
            ]
        );
        assert_eq!(
            began,
            recorded[..2],
            "the line it began with, for every call that ran"
        );
        let kinds: Vec<ToolKind> = events
            .iter()
            .filter_map(|event| match &event.body {
                Body::ToolCallRequested(call) => Some(call.kind),
                _ => None,
            })
            .collect();
        assert_eq!(
            kinds,
            [ToolKind::Edit, ToolKind::Command, ToolKind::Other],
            "each call says what kind of work it is, as its source does"
        );
        assert_eq!(
            conversation(&events),
            thread.messages(),
            "the line is glave's record: the model is sent the same"
        );
    });
}

#[test]
fn a_declined_action_is_reported_to_the_model() {
    let dir = folder("deny");
    block_on!(async {
        let answers = vec![
            answer(
                None,
                vec![("bash", json!({"command": "rm -rf build"}))],
                StopReason::ToolUse,
            ),
            answer(Some("Understood."), vec![], StopReason::EndTurn),
        ];
        let harness = asking_harness(&dir, answers, Refuse).await;
        let mut thread = harness.start_thread(scope()).await.unwrap();
        turn(&harness, &mut thread, "clean up").await;
        let declined = thread.messages().iter().any(|m| {
                matches!(m,
                glave_ports::Message::Tool { is_error: true, output, .. }
                    if matches!(output.first(), Some(Part::Text { text }) if text.contains("declined")))
            });
        assert!(declined);
    });
}

#[test]
fn the_mode_decides_what_asks() {
    use glave_harness::Permissions;
    // No sandbox here: Auto asks before everything, as Ask first does, and only Full access
    // goes through.
    for (mode, wrote, ran) in [
        (Permissions::Ask, false, false),
        (Permissions::Auto, false, false),
        (Permissions::FullAccess, true, true),
    ] {
        let dir = folder(&format!("mode-{}", mode.name()));
        block_on!(async {
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
            settings.permissions = mode;
            let harness = harness_on(&dir, answers, Refuse, InMemoryLog::new(), 1, settings).await;
            let mut thread = harness.start_thread(scope()).await.unwrap();
            turn(&harness, &mut thread, "go").await;
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
    block_on!(async {
        let answers = vec![
            answer(
                None,
                vec![(
                    "edit",
                    json!({"path": "main.rs", "old_string": "\"hi\"", "new_string": "\"hello\""}),
                )],
                StopReason::ToolUse,
            ),
            reads("main.rs"),
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
        turn(&harness, &mut thread, "say hello").await;
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
    block_on!(async {
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
        turn(&harness, &mut thread, "run it").await;
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
    block_on!(async {
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
        turn(&harness, &mut thread, "loop").await;
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
    block_on!(async {
        let first = vec![answer(Some("Hello."), vec![], StopReason::EndTurn)];
        let harness = harness(&dir, first, AllowAll).await;
        let mut thread = harness.start_thread(scope()).await.unwrap();
        turn(&harness, &mut thread, "hi").await;

        let raw = harness
            .log()
            .read(&scope(), thread.id, None, usize::MAX)
            .await
            .unwrap();
        let log = InMemoryLog::new();
        log.append(&scope(), &raw).await.unwrap();
        let later = vec![answer(Some("Again."), vec![], StopReason::EndTurn)];
        let resumed_harness = harness_on(&dir, later, AllowAll, log, 1_000, settings()).await;
        let mut resumed = resumed_harness
            .resume_thread(scope(), thread.id)
            .await
            .unwrap();
        assert_eq!(resumed.messages(), thread.messages());

        turn(&resumed_harness, &mut resumed, "again").await;
        let events = events(resumed_harness.log(), thread.id).await;
        let seqs: Vec<u64> = events.iter().map(|e| e.header.seq.get()).collect();
        assert_eq!(
            seqs,
            (1..=u64::try_from(events.len()).unwrap()).collect::<Vec<_>>()
        );
        assert_eq!(conversation(&events), resumed.messages());
    });
}

/// The log of a turn that ran its one command, cut before the command's result: a crash
/// mid-call.
async fn crashed_mid_call(dir: &std::path::Path) -> (InMemoryLog, glave_event::ThreadId) {
    let answers = vec![
        answer(
            None,
            vec![("bash", json!({"command": "true"}))],
            StopReason::ToolUse,
        ),
        answer(Some("ok"), vec![], StopReason::EndTurn),
    ];
    let harness = harness(dir, answers, AllowAll).await;
    let mut thread = harness.start_thread(scope()).await.unwrap();
    turn(&harness, &mut thread, "go").await;
    let raw = harness
        .log()
        .read(&scope(), thread.id, None, usize::MAX)
        .await
        .unwrap();
    let cut: Vec<RawEvent> = raw.into_iter().take(5).collect();
    let log = InMemoryLog::new();
    log.append(&scope(), &cut).await.unwrap();
    (log, thread.id)
}

/// What a crash left: the turn's start, its response with the call, and no result or end.
const LEFT: [&str; 5] = [
    "thread.started",
    "turn.started",
    "message.added",
    "tool.call.requested",
    "model.responded",
];

#[test]
fn resuming_after_a_crash_records_nothing_and_the_next_turn_closes_what_was_left_open() {
    let dir = folder("crash");
    block_on!(async {
        let (log, id) = crashed_mid_call(&dir).await;
        let again = vec![answer(Some("Again."), vec![], StopReason::EndTurn)];
        let resumed_harness = harness_on(&dir, again, AllowAll, log, 1_000, settings()).await;
        let mut resumed = resumed_harness.resume_thread(scope(), id).await.unwrap();
        assert!(resumed.left_open());
        let read = events(resumed_harness.log(), id).await;
        assert_eq!(kinds(&read), LEFT, "reading the thread records nothing");

        turn(&resumed_harness, &mut resumed, "again").await;
        let events = events(resumed_harness.log(), id).await;
        let closed = [LEFT.as_slice(), &["tool.call.completed", "turn.ended"]].concat();
        assert_eq!(kinds(&events).get(..7), Some(closed.as_slice()));
        assert!(matches!(&events[6].body,
                Body::TurnEnded(ended) if ended.outcome == Outcome::Interrupted));
        assert!(matches!(&events[5].body,
                Body::ToolCallCompleted(done) if done.is_error));
        assert_eq!(kinds(&events).get(7), Some(&"turn.started"));
        assert!(!resumed.left_open());
        assert_eq!(conversation(&events), resumed.messages());
    });
}

#[test]
fn repairing_closes_a_turn_left_open_once() {
    let dir = folder("repair");
    block_on!(async {
        let (log, id) = crashed_mid_call(&dir).await;
        let harness = harness_on(&dir, vec![], AllowAll, log, 1_000, settings()).await;
        assert!(harness.repair(scope(), id).await.unwrap());
        assert!(!harness.repair(scope(), id).await.unwrap(), "once");
        let events = events(harness.log(), id).await;
        let closed = [LEFT.as_slice(), &["tool.call.completed", "turn.ended"]].concat();
        assert_eq!(kinds(&events), closed);
        let Some(Body::TurnEnded(ended)) = events.last().map(|e| &e.body) else {
            panic!("the turn ends");
        };
        assert_eq!(ended.outcome, Outcome::Interrupted);
        assert_eq!(
            (ended.model_calls, ended.tokens.input),
            (1, 100),
            "what it did before the crash, from its log"
        );
        let resumed = harness.resume_thread(scope(), id).await.unwrap();
        assert!(!resumed.left_open());
        assert!(matches!(
            resumed.messages().last(),
            Some(glave_ports::Message::Tool { is_error: true, .. })
        ));
    });
}

#[test]
fn resuming_a_thread_that_does_not_exist_is_an_error() {
    let dir = folder("missing");
    block_on!(async {
        let harness = harness(&dir, vec![], AllowAll).await;
        let thread = harness.start_thread(scope()).await.unwrap();
        let elsewhere = Scope::glaves_own(
            OrgId::from_uuid(uuid_like(9)),
            ProjectId::from_uuid(uuid_like(9)),
        );
        assert!(harness.resume_thread(elsewhere, thread.id).await.is_err());
    });
}
