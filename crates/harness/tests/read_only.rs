//! Read-only in the real sandbox, as Codex's Read Only works: a command runs without asking
//! but cannot change the project, a change asks first, and what the person allows of it
//! lands; with no one there, nothing changes.

use glave_event::{Body, StopReason};
use glave_harness::{Cancel, Decision, NoOneToAsk};
use glave_ports::InMemoryLog;
use serde_json::json;

use crate::outside_sandbox::{Person, results, sandboxed, walled};
use crate::support::*;

/// A command writing `ran.txt` in the project, an edit writing `notes.md`, then the last word.
fn command_then_change() -> Vec<glave_ports::Completion> {
    vec![
        answer(
            None,
            vec![("bash", json!({"command": "echo x > ran.txt"}))],
            StopReason::ToolUse,
        ),
        answer(
            None,
            vec![("write", json!({"path": "notes.md", "content": "hi"}))],
            StopReason::ToolUse,
        ),
        answer(Some("Done."), vec![], StopReason::EndTurn),
    ]
}

#[test]
fn a_command_runs_unasked_but_changes_nothing_and_a_change_asks_first() {
    let Some(base) = sandboxed("read-only") else {
        return;
    };
    block_on!(async {
        let person = Person::answering(Decision::AllowOnce);
        let workspace = walled(&base, false).await;
        let (harness, _) = harness_with(
            workspace,
            command_then_change(),
            person.clone(),
            InMemoryLog::new(),
            1,
            asking(),
        );
        let mut thread = harness.start_thread(scope()).await.unwrap();
        harness
            .run_turn(&mut thread, "look, then note it", &Cancel::new(), &mut |_| {})
            .await
            .unwrap();
        let asked: Vec<String> = person.asked().iter().map(|a| a.tool.clone()).collect();
        assert_eq!(asked, ["write"], "only the change asked");
        let project = base.join("project");
        assert!(!project.join("ran.txt").exists(), "the sandbox kept it");
        assert!(
            results(thread.messages())[0].contains("sandbox"),
            "the model is told the sandbox may have blocked it"
        );
        assert!(project.join("notes.md").exists(), "allowed, it landed");
    });
}

#[test]
fn with_no_one_there_read_only_changes_nothing() {
    let Some(base) = sandboxed("read-only-alone") else {
        return;
    };
    block_on!(async {
        let workspace = walled(&base, false).await;
        let (harness, _) = harness_with(
            workspace,
            command_then_change(),
            NoOneToAsk,
            InMemoryLog::new(),
            1,
            asking(),
        );
        let mut thread = harness.start_thread(scope()).await.unwrap();
        harness
            .run_turn(&mut thread, "note it", &Cancel::new(), &mut |_| {})
            .await
            .unwrap();
        let project = base.join("project");
        assert!(!project.join("ran.txt").exists() && !project.join("notes.md").exists());
        let declined = events(harness.log(), thread.id)
            .await
            .iter()
            .filter(|event| matches!(event.body, Body::ApprovalDecided(_)))
            .count();
        assert_eq!(declined, 1, "the change was declined, the command never asked");
    });
}
