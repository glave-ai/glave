//! Turns on a log that fails a write or a read, as a store that blips does. A turn that failed
//! between a call and its result is closed by the next one, so the model is never sent a call
//! without a result, and a command the call started is still recorded. An answer whose turn
//! could not go on leaves the thread to its next prompt; a stop that could not leaves the turn
//! waiting, and a prompt meanwhile records nothing.

use std::sync::Arc;

use glave_event::{Body, CallId, CommandEnding, Event, Outcome, ServiceAnswer, StopReason};
use glave_harness::{Cancel, Decision, HarnessError, NoOneToAsk};
use glave_ports::testing::{ASKING_TOOL, AskingTool, FaultyLog};
use glave_ports::{
    Caller, Effect, InMemoryTools, Message, ToolInfo, ToolOutput, ToolSource, ToolSpec,
};
use serde_json::json;

use crate::network::InTheLog;
use crate::support::*;

fn outcomes(events: &[Event]) -> Vec<Outcome> {
    events
        .iter()
        .filter_map(|event| match &event.body {
            Body::TurnEnded(ended) => Some(ended.outcome),
            _ => None,
        })
        .collect()
}

/// The ids of the calls `events` request, and of those they complete, each in order.
fn calls(events: &[Event]) -> (Vec<CallId>, Vec<CallId>) {
    let requested = events.iter().filter_map(|event| match &event.body {
        Body::ToolCallRequested(call) => Some(call.call_id.clone()),
        _ => None,
    });
    let completed = events.iter().filter_map(|event| match &event.body {
        Body::ToolCallCompleted(done) => Some(done.call_id.clone()),
        _ => None,
    });
    (requested.collect(), completed.collect())
}

/// A turn that asks to write `a.txt` and stops there, waiting in the log, then answers "Done."
/// when it goes on; its harness on `log`, and the call it waits on.
async fn waiting(
    dir: &std::path::Path,
    log: &FaultyLog,
) -> (
    glave_harness::Harness<ScriptedModel, FaultyLog, glave_ports::InMemoryBlobs, InTheLog>,
    glave_harness::Thread,
    CallId,
) {
    let answers = vec![
        answer(
            None,
            vec![("write", json!({"path": "a.txt", "content": "a\n"}))],
            StopReason::ToolUse,
        ),
        answer(Some("Done."), vec![], StopReason::EndTurn),
    ];
    let (harness, _) = harness_with(
        workspace(dir).await,
        answers,
        InTheLog,
        log.clone(),
        1,
        asking(),
    );
    let mut thread = harness.start_thread(scope()).await.unwrap();
    let stopped = harness
        .run_turn(&mut thread, "write a", &Cancel::new(), &mut |_| {})
        .await;
    assert!(
        matches!(stopped, Err(HarnessError::Waits(_))),
        "{stopped:?}"
    );
    let asked = events(harness.log(), thread.id)
        .await
        .iter()
        .find_map(|event| match &event.body {
            Body::ApprovalRequested(asked) => Some(asked.call_id.clone()),
            _ => None,
        })
        .unwrap();
    (harness, thread, asked)
}

#[test]
fn a_turn_that_failed_between_a_call_and_its_result_is_closed_by_the_next() {
    let dir = folder("log-fails-mid-call");
    block_on!(async {
        let log = FaultyLog::default();
        let answers = vec![
            answer(
                None,
                vec![("bash", json!({"command": "sleep 30", "background": true}))],
                StopReason::ToolUse,
            ),
            answer(
                None,
                vec![("command_output", json!({"command": 1, "stop": true}))],
                StopReason::ToolUse,
            ),
            answer(Some("Stopped."), vec![], StopReason::EndTurn),
        ];
        let (harness, seen) = harness_with(
            workspace(&dir).await,
            answers,
            NoOneToAsk,
            log.clone(),
            1,
            settings(),
        );
        let mut thread = harness.start_thread(scope()).await.unwrap();
        log.fail_with("tool.call.completed");
        let failed = harness
            .run_turn(&mut thread, "start the server", &Cancel::new(), &mut |_| {})
            .await;
        assert!(matches!(failed, Err(HarnessError::Log(_))), "{failed:?}");
        turn(&harness, &mut thread, "stop it").await;

        let events = events(harness.log(), thread.id).await;
        assert_eq!(
            outcomes(&events),
            [Outcome::Interrupted, Outcome::Completed]
        );
        let (requested, completed) = calls(&events);
        assert_eq!(requested.len(), 2, "{requested:?}");
        assert_eq!(requested, completed, "each call has its one result");
        // The command the failed call started was recorded after all, and stopped.
        let started = events
            .iter()
            .filter(|event| matches!(event.body, Body::CommandStarted(_)))
            .count();
        assert_eq!(started, 1);
        assert!(events.iter().any(|event| matches!(
            &event.body,
            Body::CommandEnded(ended) if matches!(ended.ending, CommandEnding::Stopped { .. })
        )));
        // The model was never sent a call without its result.
        for request in seen.lock().unwrap().iter() {
            let asked = request.messages.iter().flat_map(|message| match message {
                Message::Assistant { tool_calls, .. } => tool_calls.clone(),
                _ => Vec::new(),
            });
            for call in asked {
                let answered = request.messages.iter().any(|message| {
                    matches!(message, Message::Tool { call_id, .. } if *call_id == call.call_id)
                });
                assert!(answered, "{:?} has no result", call.call_id);
            }
        }
    });
}

/// `asked` allowed, and the turn gone on with until the write or read of `kind` fails.
async fn allowed_and_lost(
    harness: &glave_harness::Harness<
        ScriptedModel,
        FaultyLog,
        glave_ports::InMemoryBlobs,
        InTheLog,
    >,
    thread: &mut glave_harness::Thread,
    asked: &CallId,
    log: &FaultyLog,
    kind: &'static str,
) {
    harness
        .decide(scope(), thread.id, asked, Decision::AllowOnce, None)
        .await
        .unwrap();
    log.fail_with(kind);
    let failed = harness
        .go_on(thread, &Caller::default(), &Cancel::new(), &mut |_| {})
        .await;
    assert!(matches!(failed, Err(HarnessError::Log(_))), "{failed:?}");
}

#[test]
fn an_answer_whose_turn_could_not_go_on_leaves_the_thread_to_its_next_prompt() {
    let dir = folder("log-fails-going-on");
    block_on!(async {
        let log = FaultyLog::default();
        let (harness, mut thread, asked) = waiting(&dir, &log).await;
        // Going on reads the answer first, and that read fails.
        allowed_and_lost(&harness, &mut thread, &asked, &log, "approval.decided").await;
        assert!(!thread.waits(), "its answer is in the log: nothing waits");

        turn(&harness, &mut thread, "go on").await;
        let events = events(harness.log(), thread.id).await;
        assert_eq!(
            outcomes(&events),
            [Outcome::Interrupted, Outcome::Completed]
        );
        let (requested, completed) = calls(&events);
        assert_eq!(requested, completed);
        assert!(!dir.join("a.txt").exists(), "the answered call never ran");
    });
}

#[test]
fn an_answered_call_whose_result_never_landed_is_closed_and_never_run_again() {
    let dir = folder("log-fails-after-running");
    block_on!(async {
        let log = FaultyLog::default();
        let (harness, mut thread, asked) = waiting(&dir, &log).await;
        allowed_and_lost(&harness, &mut thread, &asked, &log, "tool.call.completed").await;
        let ran = dir.join("a.txt");
        assert!(ran.exists(), "it ran, and only its result was lost");
        std::fs::remove_file(&ran).unwrap();

        let again = harness
            .go_on(&mut thread, &Caller::default(), &Cancel::new(), &mut |_| {})
            .await
            .unwrap();
        assert_eq!(again.outcome, Outcome::Interrupted);
        assert!(!ran.exists(), "going on again must not run it again");
        let (requested, completed) = calls(&events(harness.log(), thread.id).await);
        assert_eq!(requested, completed, "it has the interrupted result");
    });
}

/// A tool that needs no one, and counts its runs.
fn counting() -> Arc<InMemoryTools> {
    let info = ToolInfo {
        spec: ToolSpec {
            name: "count".into(),
            description: "Counts.".into(),
            parameters: json!({"type": "object"}),
        },
        effect: Effect::Reads,
        parallel: false,
    };
    Arc::new(InMemoryTools::new(vec![(
        info,
        ToolOutput::text(false, "counted"),
    )]))
}

/// A harness on `log` answering with `answers`, offering `tools`, and a thread there.
async fn counted(
    dir: &std::path::Path,
    log: &FaultyLog,
    tools: &Arc<InMemoryTools>,
    answers: Vec<glave_ports::Completion>,
) -> (
    glave_harness::Harness<ScriptedModel, FaultyLog, glave_ports::InMemoryBlobs, InTheLog>,
    glave_harness::Thread,
) {
    let (harness, _) = harness_with(
        workspace(dir).await,
        answers,
        InTheLog,
        log.clone(),
        1,
        settings(),
    );
    let harness = harness.with_tools(vec![Arc::clone(tools) as Arc<dyn ToolSource>]);
    let thread = harness.start_thread(scope()).await.unwrap();
    (harness, thread)
}

/// A call that asked nothing, whose result was lost, is not run by work left over from an answer.
#[test]
fn a_call_that_asked_nothing_and_lost_its_result_is_not_run_by_going_on() {
    let dir = folder("log-fails-unasked");
    block_on!(async {
        let log = FaultyLog::default();
        let tools = counting();
        let answers = vec![answer(
            None,
            vec![("count", json!({}))],
            StopReason::ToolUse,
        )];
        let (harness, mut thread) = counted(&dir, &log, &tools, answers).await;
        log.fail_with("tool.call.completed");
        let failed = harness
            .run_turn(&mut thread, "count", &Cancel::new(), &mut |_| {})
            .await;
        assert!(matches!(failed, Err(HarnessError::Log(_))), "{failed:?}");
        assert_eq!(tools.calls().len(), 1);

        let again = harness
            .go_on(&mut thread, &Caller::default(), &Cancel::new(), &mut |_| {})
            .await
            .unwrap();
        assert_eq!(again.outcome, Outcome::Interrupted);
        assert_eq!(tools.calls().len(), 1, "going on must not run it again");
    });
}

/// A call behind a question in one response never began when the question is answered: going on
/// runs it, and the turn goes on to the model.
#[test]
fn a_plain_call_after_an_answered_question_runs_once() {
    let dir = folder("log-question-then-call");
    block_on!(async {
        let log = FaultyLog::default();
        let tools = counting();
        let asks = json!({"questions": [{"question": "Which database?"}]});
        let answers = vec![
            answer(
                None,
                vec![("question", asks), ("count", json!({}))],
                StopReason::ToolUse,
            ),
            answer(Some("Done."), vec![], StopReason::EndTurn),
        ];
        let (harness, mut thread) = counted(&dir, &log, &tools, answers).await;
        let stopped = harness
            .run_turn(&mut thread, "ask", &Cancel::new(), &mut |_| {})
            .await;
        assert!(
            matches!(stopped, Err(HarnessError::Waits(_))),
            "{stopped:?}"
        );
        assert_eq!(tools.calls().len(), 0);
        let question = events(harness.log(), thread.id)
            .await
            .iter()
            .find_map(|event| match &event.body {
                Body::ToolCallRequested(call) if call.tool == "question" => {
                    Some(call.call_id.clone())
                }
                _ => None,
            })
            .unwrap();
        harness
            .answer(scope(), thread.id, &question, &["Postgres".to_owned()])
            .await
            .unwrap();
        let ended = harness
            .go_on(&mut thread, &Caller::default(), &Cancel::new(), &mut |_| {})
            .await
            .unwrap();
        assert_eq!(ended.outcome, Outcome::Completed);
        assert_eq!(tools.calls().len(), 1);
    });
}

#[test]
fn a_call_that_could_not_record_beginning_never_runs() {
    let dir = folder("log-fails-going-ahead");
    block_on!(async {
        let log = FaultyLog::default();
        let (harness, mut thread, asked) = waiting(&dir, &log).await;
        allowed_and_lost(&harness, &mut thread, &asked, &log, "tool.call.started").await;
        assert!(!dir.join("a.txt").exists(), "nothing says it may have run");

        // Still only answered: going on runs it, for the first time.
        let ended = harness
            .go_on(&mut thread, &Caller::default(), &Cancel::new(), &mut |_| {})
            .await
            .unwrap();
        assert_eq!(ended.outcome, Outcome::Completed);
        assert!(dir.join("a.txt").exists());
    });
}

#[test]
fn a_services_answer_whose_result_never_landed_is_not_given_again() {
    let dir = folder("log-fails-after-answering");
    block_on!(async {
        let log = FaultyLog::default();
        let source = AskingTool::asking(&["Which repository?"]);
        let answers = vec![
            answer(None, vec![(ASKING_TOOL, json!({}))], StopReason::ToolUse),
            answer(Some("Filed."), vec![], StopReason::EndTurn),
        ];
        let (harness, _) = harness_with(
            workspace(&dir).await,
            answers,
            InTheLog,
            log.clone(),
            1,
            settings(),
        );
        let harness = harness.with_tools(vec![Arc::clone(&source) as Arc<dyn ToolSource>]);
        let mut thread = harness.start_thread(scope()).await.unwrap();
        let stopped = harness
            .run_turn(&mut thread, "file it", &Cancel::new(), &mut |_| {})
            .await;
        assert!(
            matches!(stopped, Err(HarnessError::Waits(_))),
            "{stopped:?}"
        );
        let (call, at) = events(harness.log(), thread.id)
            .await
            .iter()
            .find_map(|event| match &event.body {
                Body::ServiceAsked(asked) => Some((asked.call_id.clone(), event.header.seq)),
                _ => None,
            })
            .unwrap();
        harness
            .answer_service(scope(), thread.id, &call, at, ServiceAnswer::Declined, None)
            .await
            .unwrap();
        log.fail_with("tool.call.completed");
        let failed = harness
            .go_on(&mut thread, &Caller::default(), &Cancel::new(), &mut |_| {})
            .await;
        assert!(matches!(failed, Err(HarnessError::Log(_))), "{failed:?}");
        assert_eq!(source.given().len(), 2, "asked, then given its answer");

        let again = harness
            .go_on(&mut thread, &Caller::default(), &Cancel::new(), &mut |_| {})
            .await
            .unwrap();
        assert_eq!(again.outcome, Outcome::Interrupted);
        assert_eq!(source.given().len(), 2, "never given it a second time");
    });
}

#[test]
fn a_stop_whose_turn_could_not_go_on_leaves_it_waiting() {
    let dir = folder("log-fails-stopping");
    block_on!(async {
        let log = FaultyLog::default();
        let (harness, mut thread, _) = waiting(&dir, &log).await;
        let stop = Cancel::new();
        stop.cancel();
        // Going on reads what the turn waits for first, and that read fails.
        log.fail_with("approval.requested");
        let failed = harness
            .go_on(&mut thread, &Caller::default(), &stop, &mut |_| {})
            .await;
        assert!(matches!(failed, Err(HarnessError::Log(_))), "{failed:?}");

        // Nothing was answered: a prompt meanwhile is refused, and records nothing.
        let before = events(harness.log(), thread.id).await.len();
        let refused = harness
            .run_turn(&mut thread, "something else", &Cancel::new(), &mut |_| {})
            .await;
        assert!(
            matches!(refused, Err(HarnessError::Waits(_))),
            "{refused:?}"
        );
        assert!(thread.waits());
        assert_eq!(events(harness.log(), thread.id).await.len(), before);

        let stop = Cancel::new();
        stop.cancel();
        let stopped = harness
            .go_on(&mut thread, &Caller::default(), &stop, &mut |_| {})
            .await
            .unwrap();
        assert_eq!(stopped.outcome, Outcome::Cancelled);
    });
}
