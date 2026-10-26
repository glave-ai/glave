//! The thread's plan (`specs/session.md`, "Plans"): kept by the model with `update_plan`,
//! recorded beside each call's result without changing what the model is sent, and read from
//! the log.

use glave_event::{Event, PlanStatus, StopReason};
use glave_harness::{Cancel, NoOneToAsk};
use glave_ports::{Completion, InMemoryLog, Message, ModelRequest};
use serde_json::json;

use crate::support::*;

/// The model's `update_plan` call with `steps`, each with its status.
fn plan(steps: &[(&str, &str)]) -> Completion {
    let plan: Vec<_> = steps
        .iter()
        .map(|(step, status)| json!({"step": step, "status": status}))
        .collect();
    answer(
        None,
        vec![("update_plan", json!({ "plan": plan }))],
        StopReason::ToolUse,
    )
}

fn one(tool: &str, arguments: serde_json::Value) -> Completion {
    answer(None, vec![(tool, arguments)], StopReason::ToolUse)
}

fn of_kind<'a>(events: &'a [Event], kind: &str) -> Vec<&'a Event> {
    let kinds = kinds(events);
    events
        .iter()
        .zip(kinds)
        .filter(|(_, each)| *each == kind)
        .map(|(event, _)| event)
        .collect()
}

/// The ids of the calls the model was sent, in the order it was sent them.
fn sent_ids(sent: &[ModelRequest]) -> Vec<String> {
    let last = sent.last().unwrap();
    last.messages
        .iter()
        .filter_map(|message| match message {
            Message::Tool { call_id, .. } => Some(call_id.as_str().to_owned()),
            _ => None,
        })
        .collect()
}

/// What a checklist came to, with plans on or off: whether `update_plan` was offered, the call
/// ids the model was sent, how many `plan.updated` were recorded, and the plan.
fn checklist(plans: bool) -> (bool, Vec<String>, usize, Option<glave_harness::Plan>) {
    let answers = vec![
        plan(&[("Look", "in_progress"), ("Fix", "pending")]),
        one("bash", json!({"command": "echo hi"})),
        plan(&[("Look", "completed"), ("Fix", "completed")]),
        one("bash", json!({"command": "echo done"})),
        answer(Some("Done."), vec![], StopReason::EndTurn),
    ];
    let dir = folder("plan-ids");
    let mut settings = settings();
    settings.plans = plans;
    block_on!(async {
        let log = InMemoryLog::new();
        let (harness, seen) = harness_seeing(&dir, answers, NoOneToAsk, log, 1, settings).await;
        let mut thread = harness.start_thread(scope()).await.unwrap();
        harness
            .run_turn(&mut thread, "fix it", &Cancel::new(), &mut |_| {})
            .await
            .unwrap();
        let sent = seen.lock().unwrap().clone();
        let offered = sent[0].tools.iter().any(|tool| tool.name == "update_plan");
        let events = events(harness.log(), thread.id).await;
        let updates = of_kind(&events, "plan.updated").len();
        (
            offered,
            sent_ids(&sent),
            updates,
            harness.plan(scope(), thread.id).await.unwrap(),
        )
    })
}

#[test]
fn the_checklist_is_kept_read_from_the_log_and_the_model_is_sent_the_same_ids() {
    let (offered, kept, updates, plan) = checklist(true);
    assert!(offered);
    assert_eq!(updates, 2, "each update recorded");
    let done = plan
        .unwrap()
        .steps
        .iter()
        .all(|step| step.status == PlanStatus::Completed);
    assert!(done, "the latest update is the plan");
    let (offered, without, updates, plan) = checklist(false);
    assert!(!offered, "off");
    assert_eq!((updates, plan), (0, None));
    assert_eq!(kept, without, "recording the plan changes no call's id");
}
