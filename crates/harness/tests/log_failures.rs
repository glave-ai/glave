//! Turns on a log that fails a write or a read, as a store that blips does. A turn that failed
//! between a call and its result is closed by the next one, so the model is never sent a call
//! without a result, and a command the call started is still recorded. An answer whose turn
//! could not go on leaves the thread to its next prompt; a stop that could not leaves the turn
//! waiting, and a prompt meanwhile records nothing.

use glave_event::{Body, CallId, CommandEnding, Event, Outcome, StopReason};
use glave_harness::{Cancel, Decision, HarnessError, NoOneToAsk};
use glave_ports::{Caller, Message};
use serde_json::json;

use crate::failing::FailingLog;
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
    log: &FailingLog,
) -> (
    glave_harness::Harness<ScriptedModel, FailingLog, glave_ports::InMemoryBlobs, InTheLog>,
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
        let log = FailingLog::default();
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

#[test]
fn an_answer_whose_turn_could_not_go_on_leaves_the_thread_to_its_next_prompt() {
    let dir = folder("log-fails-going-on");
    block_on!(async {
        let log = FailingLog::default();
        let (harness, mut thread, asked) = waiting(&dir, &log).await;
        harness
            .decide(scope(), thread.id, &asked, Decision::AllowOnce, None)
            .await
            .unwrap();
        // Going on reads the answer first, and that read fails.
        log.fail_with("approval.decided");
        let failed = harness
            .go_on(&mut thread, &Caller::default(), &Cancel::new(), &mut |_| {})
            .await;
        assert!(matches!(failed, Err(HarnessError::Log(_))), "{failed:?}");
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
fn a_stop_whose_turn_could_not_go_on_leaves_it_waiting() {
    let dir = folder("log-fails-stopping");
    block_on!(async {
        let log = FailingLog::default();
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
