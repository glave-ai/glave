//! The presets in the real sandbox. Read-only works as Codex's Read Only does: a command runs
//! without asking but cannot change the project, a change asks first, what the person allows
//! of it lands, the judge is never asked, and with no one there nothing changes. The policy's
//! cap holds from the next turn whatever a thread chose before.

use glave_event::{Body, Permissions, StopReason};
use glave_harness::{Cancel, Decision, Defaults, NoOneToAsk};
use glave_ports::{Caller, InMemoryLog};
use serde_json::json;

use crate::outside_sandbox::{Person, judges, results, sandboxed, walled};
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
            .run_turn(
                &mut thread,
                "look, then note it",
                &Cancel::new(),
                &mut |_| {},
            )
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
        assert_eq!(
            declined, 1,
            "the change was declined, the command never asked"
        );
    });
}

#[test]
fn in_read_only_the_judge_is_never_asked_and_a_risky_command_waits_for_the_person() {
    let Some(base) = sandboxed("read-only-judge") else {
        return;
    };
    block_on!(async {
        let answers = vec![
            answer(
                None,
                vec![("bash", json!({"command": "git push origin main"}))],
                StopReason::ToolUse,
            ),
            answer(Some("Waiting for you."), vec![], StopReason::EndTurn),
        ];
        let mut settings = asking();
        settings.judge = glave_ports::settings::Judge::Main;
        let workspace = walled(&base, false).await;
        let (harness, seen) = harness_with(
            workspace,
            answers,
            NoOneToAsk,
            InMemoryLog::new(),
            1,
            settings,
        );
        let mut thread = harness.start_thread(scope()).await.unwrap();
        harness
            .run_turn(&mut thread, "push my branch", &Cancel::new(), &mut |_| {})
            .await
            .unwrap();
        assert!(
            judges(&seen.lock().unwrap()).is_empty(),
            "no judge in Read-only"
        );
        assert!(results(thread.messages())[0].contains("no one is available"));
    });
}

#[test]
fn the_policy_s_cap_holds_from_the_next_turn_over_a_thread_s_own_full_access() {
    let Some(base) = sandboxed("capped") else {
        return;
    };
    let beside = |file: &str| {
        let command = format!("echo x > {}", base.join(file).display());
        answer(
            None,
            vec![("bash", json!({ "command": command }))],
            StopReason::ToolUse,
        )
    };
    let answers = vec![
        beside("one.txt"),
        answer(Some("Done."), vec![], StopReason::EndTurn),
        beside("two.txt"),
        answer(Some("Done."), vec![], StopReason::EndTurn),
    ];
    block_on!(async {
        let workspace = walled(&base, false).await;
        let (harness, _) = harness_with(
            workspace,
            answers,
            NoOneToAsk,
            InMemoryLog::new(),
            1,
            in_the_sandbox(),
        );
        let mut thread = harness.start_thread(scope()).await.unwrap();
        harness
            .choose_permissions(&mut thread, &Caller::default(), Permissions::FullAccess)
            .await
            .unwrap();
        harness
            .run_turn(&mut thread, "write beside it", &Cancel::new(), &mut |_| {})
            .await
            .unwrap();
        assert!(base.join("one.txt").exists(), "full access: no sandbox");
        // The organization caps it at Workspace while the server runs.
        let mut defaults = Defaults::clone(&harness.defaults());
        defaults.highest = Permissions::Workspace;
        harness.set_defaults(defaults);
        harness
            .run_turn(&mut thread, "again", &Cancel::new(), &mut |_| {})
            .await
            .unwrap();
        assert_eq!(harness.permissions_for(&thread), Permissions::Workspace);
        assert!(!base.join("two.txt").exists(), "in the sandbox again");
    });
}
