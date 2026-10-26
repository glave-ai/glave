//! Approvals in the log: every question to the person is recorded with its answer, the person
//! is told which call and thread it is about, and "allow for this thread" survives resuming.

use glave_event::{Body, StopReason, Verdict};
use glave_harness::{Decision, NoOneToAsk};
use glave_ports::InMemoryLog;
use serde_json::json;

use crate::support::*;

fn touch(name: &str) -> Vec<(&'static str, serde_json::Value)> {
    vec![("bash", json!({"command": format!("touch {name}")}))]
}

/// The approval events of a thread, as (call, verdict) pairs.
fn verdicts(events: &[glave_event::Event]) -> Vec<(String, Verdict)> {
    events
        .iter()
        .filter_map(|event| match &event.body {
            Body::ApprovalDecided(decided) => {
                Some((decided.call_id.as_str().to_owned(), decided.decision))
            }
            _ => None,
        })
        .collect()
}

#[test]
fn each_question_and_its_answer_are_recorded_before_the_result() {
    let dir = folder("approvals-recorded");
    block_on!(async {
        let answers = vec![
            answer(None, touch("a"), StopReason::ToolUse),
            answer(None, touch("b"), StopReason::ToolUse),
            answer(Some("Done."), vec![], StopReason::EndTurn),
        ];
        let approver = Answers::new(&[Decision::AllowOnce, Decision::Deny]);
        let harness = asking_harness(&dir, answers, approver).await;
        let mut thread = harness.start_thread(scope()).await.unwrap();
        turn(&harness, &mut thread, "touch two files").await;
        assert!(dir.join("a").exists() && !dir.join("b").exists());

        let events = events(harness.log(), thread.id).await;
        let kinds = kinds(&events);
        let asked = kinds.iter().position(|kind| *kind == "approval.requested");
        let first = &kinds[asked.unwrap()..];
        assert_eq!(
            &first[..3],
            [
                "approval.requested",
                "approval.decided",
                "tool.call.completed"
            ],
            "{kinds:?}"
        );
        let decided = verdicts(&events);
        assert_eq!(
            decided
                .iter()
                .map(|(_, verdict)| *verdict)
                .collect::<Vec<_>>(),
            [Verdict::AllowOnce, Verdict::Deny]
        );
        let Body::ApprovalRequested(requested) = &events
            .iter()
            .find(|event| matches!(event.body, Body::ApprovalRequested(_)))
            .unwrap()
            .body
        else {
            panic!("no approval was requested");
        };
        assert_eq!(requested.tool, "bash");
        assert_eq!(requested.summary, "bash: touch a");
        assert_eq!(
            (requested.ask.as_str(), requested.detail.as_str()),
            ("Run this command?", "touch a"),
            "what it asks, in the server's sentence, and what would run"
        );
        assert_eq!(requested.caution, None, "only the mode asks");
        let asked = harness.approver().asked();
        assert_eq!(asked[0].thread, thread.id, "the approver knows the thread");
        assert_eq!(asked[0].call_id.as_str(), decided[0].0, "and the call");
    });
}

#[test]
fn allow_for_thread_survives_resuming_the_thread() {
    let dir = folder("approvals-thread");
    block_on!(async {
        let answers = vec![
            answer(None, touch("first"), StopReason::ToolUse),
            answer(Some("One."), vec![], StopReason::EndTurn),
            answer(None, touch("second"), StopReason::ToolUse),
            answer(Some("Two."), vec![], StopReason::EndTurn),
        ];
        // Allowed for the thread once; asked again, it would refuse.
        let approver = Answers::new(&[Decision::AllowForThread]);
        let harness = harness_on(&dir, answers, approver, InMemoryLog::new(), 1, asking()).await;
        let mut thread = harness.start_thread(scope()).await.unwrap();
        turn(&harness, &mut thread, "first").await;
        let mut resumed = harness.resume_thread(scope(), thread.id).await.unwrap();
        turn(&harness, &mut resumed, "second").await;
        assert!(
            dir.join("second").exists(),
            "not asked again after resuming"
        );
        assert_eq!(harness.approver().asked().len(), 1);
    });
}

#[test]
fn no_one_there_is_recorded_as_such() {
    let dir = folder("approvals-nobody");
    block_on!(async {
        let answers = vec![
            answer(None, touch("x"), StopReason::ToolUse),
            answer(Some("I could not."), vec![], StopReason::EndTurn),
        ];
        let harness = asking_harness(&dir, answers, NoOneToAsk).await;
        let mut thread = harness.start_thread(scope()).await.unwrap();
        turn(&harness, &mut thread, "touch x").await;
        let events = events(harness.log(), thread.id).await;
        assert_eq!(verdicts(&events)[0].1, Verdict::NoOneThere);
    });
}
