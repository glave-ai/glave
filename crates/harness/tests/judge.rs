//! The judge in Auto, behind the sandbox's wall, the one place it decides: a soft caution the
//! person asked for runs, recorded as an approval glave decided in one write, so it never waits
//! for anyone; one they didn't ask for still asks; a firm caution never reaches the judge; and
//! where every change asks the judge is never consulted.

use glave_event::{Body, StopReason, Verdict};
use glave_harness::{Cancel, HarnessError, NoOneToAsk, Permissions};
use glave_ports::{Completion, InMemoryLog};
use serde_json::json;

use crate::failing::FailingLog;
use crate::outside_sandbox::{judged, sandboxed, walled};
use crate::support::*;

fn bash(command: &str) -> Vec<(&'static str, serde_json::Value)> {
    vec![("bash", json!({ "command": command }))]
}

/// A `git push` the person asked for, and the judge's ruling that it runs.
fn cleared_push() -> Vec<Completion> {
    vec![
        answer(None, bash("git push origin main"), StopReason::ToolUse),
        answer(
            Some(r#"{"decision":"run","reason":"the person asked to push"}"#),
            vec![],
            StopReason::EndTurn,
        ),
    ]
}

fn verdicts(events: &[glave_event::Event]) -> Vec<Verdict> {
    events
        .iter()
        .filter_map(|event| match &event.body {
            Body::ApprovalDecided(decided) => Some(decided.decision),
            _ => None,
        })
        .collect()
}

fn deciders(events: &[glave_event::Event]) -> Vec<Option<glave_event::Principal>> {
    events
        .iter()
        .filter_map(|event| match &event.body {
            Body::ApprovalDecided(decided) => Some(decided.by.clone()),
            _ => None,
        })
        .collect()
}

#[test]
fn an_action_the_person_asked_for_runs_and_the_judge_is_on_record() {
    let Some(base) = sandboxed("judge-run") else {
        return;
    };
    block_on!(async {
        let mut answers = cleared_push();
        answers.push(answer(Some("Pushed."), vec![], StopReason::EndTurn));
        let workspace = walled(&base, false).await;
        let (harness, _) = harness_with(
            workspace,
            answers,
            NoOneToAsk,
            InMemoryLog::new(),
            1,
            judged(),
        );
        let mut thread = harness.start_thread(scope()).await.unwrap();
        turn(&harness, &mut thread, "push my commits to main").await;
        let said = results(thread.messages());
        assert!(!said[0].starts_with("Not run"), "{said:?}");
        let recorded = events(harness.log(), thread.id).await;
        assert_eq!(verdicts(&recorded), [Verdict::AllowOnce]);
        assert_eq!(
            deciders(&recorded),
            [Some(glave_event::Principal::glave())],
            "the judge's ruling is glave's decision"
        );
        let asked = recorded.iter().find_map(|event| match &event.body {
            Body::ApprovalRequested(asked) => Some(asked),
            _ => None,
        });
        assert_eq!(
            asked.map(|asked| (asked.summary.as_str(), asked.caution.as_deref())),
            Some((
                "bash: git push origin main",
                Some(
                    "It pushes to a remote. Allowed without asking: the person \
                         asked to push."
                )
            )),
            "what ran, and apart from it why it asked and why the judge let it run"
        );
    });
}

#[test]
fn a_cleared_action_is_asked_and_decided_in_one_write() {
    let Some(base) = sandboxed("judge-one-write") else {
        return;
    };
    block_on!(async {
        let answers = cleared_push();
        let log = FailingLog::default();
        let (harness, _) = harness_with(
            walled(&base, false).await,
            answers,
            NoOneToAsk,
            log.clone(),
            1,
            judged(),
        );
        let mut thread = harness.start_thread(scope()).await.unwrap();
        log.fail_with("approval.decided");
        let failed = harness
            .run_turn(
                &mut thread,
                "push my commits to main",
                &Cancel::new(),
                &mut |_| {},
            )
            .await;
        assert!(matches!(failed, Err(HarnessError::Log(_))), "{failed:?}");
        let recorded = events(harness.log(), thread.id).await;
        assert!(
            !recorded
                .iter()
                .any(|event| matches!(event.body, Body::ApprovalRequested(_))),
            "neither was written"
        );
        let resumed = harness.resume_thread(scope(), thread.id).await.unwrap();
        assert!(!resumed.waits(), "no one is asked what the judge cleared");
    });
}

#[test]
fn an_action_the_person_did_not_ask_for_still_asks() {
    let Some(base) = sandboxed("judge-ask") else {
        return;
    };
    block_on!(async {
        let answers = vec![
            answer(None, bash("npm publish"), StopReason::ToolUse),
            answer(
                Some(r#"{"decision":"ask","reason":"asked to bump, not to publish"}"#),
                vec![],
                StopReason::EndTurn,
            ),
            answer(Some("Left it."), vec![], StopReason::EndTurn),
        ];
        let workspace = walled(&base, false).await;
        let (harness, _) = harness_with(
            workspace,
            answers,
            NoOneToAsk,
            InMemoryLog::new(),
            1,
            judged(),
        );
        let mut thread = harness.start_thread(scope()).await.unwrap();
        turn(&harness, &mut thread, "bump the version to 1.4.0").await;
        let said = results(thread.messages());
        assert!(
            said[0].starts_with("Not run, because it publishes a package"),
            "{said:?}"
        );
    });
}

#[test]
fn a_firm_caution_never_reaches_the_judge() {
    let Some(base) = sandboxed("judge-firm") else {
        return;
    };
    std::fs::write(base.join("project/.env"), "KEY=canary\n").unwrap();
    block_on!(async {
        // Had the judge been asked, it would have taken the final answer below.
        let answers = vec![
            answer(None, bash("cat .env"), StopReason::ToolUse),
            answer(Some("I did not read it."), vec![], StopReason::EndTurn),
        ];
        let workspace = walled(&base, false).await;
        let (harness, _) = harness_with(
            workspace,
            answers,
            NoOneToAsk,
            InMemoryLog::new(),
            1,
            judged(),
        );
        let mut thread = harness.start_thread(scope()).await.unwrap();
        turn(&harness, &mut thread, "show me what's in .env").await;
        assert_eq!(
            verdicts(&events(harness.log(), thread.id).await),
            [Verdict::NoOneThere]
        );
    });
}

#[test]
fn where_every_change_asks_the_judge_is_never_consulted() {
    let dir = folder("judge-ask-mode");
    block_on!(async {
        let answers = vec![
            answer(None, bash("git push origin main"), StopReason::ToolUse),
            answer(Some("Waiting for you."), vec![], StopReason::EndTurn),
        ];
        let mut settings = judged();
        settings.permissions = Permissions::Ask;
        let harness = harness_on(&dir, answers, NoOneToAsk, InMemoryLog::new(), 1, settings).await;
        let mut thread = harness.start_thread(scope()).await.unwrap();
        turn(&harness, &mut thread, "push my commits").await;
        assert_eq!(
            verdicts(&events(harness.log(), thread.id).await),
            [Verdict::NoOneThere]
        );
    });
}
