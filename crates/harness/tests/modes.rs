//! The modes. Ask first, in the real sandbox, works as Codex's Read Only does: a command runs
//! without asking but cannot change the project, a change asks first, what the person allows
//! of it lands, the judge is never asked, and with no one there nothing changes. Full access
//! never stops, whether or not someone is there: a risky command runs unasked and unjudged, and
//! what would show the model a secret is declined at once, recorded as glave's decision. The
//! policy's cap holds from the next turn whatever a thread chose before.

use glave_event::{Body, Event, Permissions, Principal, StopReason, Verdict};
use glave_harness::{Approver, Decision, Defaults, NoOneToAsk, Settings};
use glave_ports::settings::Judge;
use glave_ports::testing::TempDir;
use glave_ports::{Caller, InMemoryLog, Message};
use serde_json::json;

use crate::outside_sandbox::{Person, judges, sandboxed, walled};
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
    let Some(base) = sandboxed("ask-first") else {
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
        turn(&harness, &mut thread, "look, then note it").await;
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
fn with_no_one_there_ask_first_changes_nothing() {
    let Some(base) = sandboxed("ask-first-alone") else {
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
        turn(&harness, &mut thread, "note it").await;
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
fn in_ask_first_the_judge_is_never_asked_and_a_risky_command_waits_for_the_person() {
    let Some(base) = sandboxed("ask-first-judge") else {
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
        settings.judge = Judge::Main;
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
        turn(&harness, &mut thread, "push my branch").await;
        assert!(
            judges(&seen.lock().unwrap()).is_empty(),
            "no judge in Ask first"
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
        turn(&harness, &mut thread, "write beside it").await;
        assert!(base.join("one.txt").exists(), "full access: no sandbox");
        // The organization caps it at Auto while the server runs.
        let mut defaults = Defaults::clone(&harness.defaults());
        defaults.highest = Permissions::Auto;
        harness.set_defaults(defaults);
        turn(&harness, &mut thread, "again").await;
        assert_eq!(harness.permissions_for(&thread), Permissions::Auto);
        assert!(!base.join("two.txt").exists(), "in the sandbox again");
    });
}

/// A project, and beside it, outside the project and the temp folder, a folder that deleting
/// is risky: `rm -rf` of it is a caution in every mode that asks.
fn beside_the_project(name: &str) -> TempDir {
    let base = TempDir::outside_temp(&format!("full-{name}")).unwrap();
    std::fs::create_dir_all(base.join("project")).unwrap();
    std::fs::create_dir_all(base.join("beside")).unwrap();
    base
}

/// Runs a turn in `base`'s project with `settings` and the judge on, in which the model deletes
/// the folder beside the project: what the log holds, and what the model was sent.
async fn deleting_beside<A: Approver>(
    base: &TempDir,
    approver: A,
    mut settings: Settings,
) -> (Vec<Event>, Seen) {
    let command = format!("rm -rf {}", base.join("beside").display());
    let answers = vec![
        answer(
            None,
            vec![("bash", json!({ "command": command }))],
            StopReason::ToolUse,
        ),
        answer(Some("Done."), vec![], StopReason::EndTurn),
    ];
    settings.judge = Judge::Main;
    let project = workspace(&base.join("project")).await;
    let (harness, seen) = harness_with(project, answers, approver, InMemoryLog::new(), 1, settings);
    let mut thread = harness.start_thread(scope()).await.unwrap();
    turn(&harness, &mut thread, "tidy up").await;
    (events(harness.log(), thread.id).await, seen)
}

fn asked_anyone(events: &[Event]) -> bool {
    events
        .iter()
        .any(|event| matches!(event.body, Body::ApprovalRequested(_)))
}

#[test]
fn full_access_runs_a_risky_command_unasked_and_unjudged_with_or_without_someone_there() {
    block_on!(async {
        // Control: where a mode asks, the same command is a caution.
        let base = beside_the_project("control");
        let person = Person::answering(Decision::Deny);
        deleting_beside(&base, person.clone(), asking()).await;
        let asked = person.asked();
        assert!(
            asked
                .first()
                .is_some_and(|request| request.caution.is_some())
        );
        assert!(base.join("beside").exists(), "declined, it never ran");

        let base = beside_the_project("there");
        let person = Person::answering(Decision::Deny);
        let (events, seen) = deleting_beside(&base, person.clone(), settings()).await;
        assert!(person.asked().is_empty(), "no one is asked");
        assert!(!asked_anyone(&events), "nor is anything recorded as asked");
        assert!(
            judges(&seen.lock().unwrap()).is_empty(),
            "the judge is never called"
        );
        assert!(!base.join("beside").exists(), "it ran");

        let base = beside_the_project("alone");
        let (events, _) = deleting_beside(&base, NoOneToAsk, settings()).await;
        assert!(!asked_anyone(&events));
        assert!(
            !base.join("beside").exists(),
            "with no one there it ran too"
        );
    });
}

/// What `.env` holds in the secret test: it must never reach the model.
const CANARY: &str = "glave-canary-full-access";

/// Runs a turn in Full access in which the model makes `call`, which would show it a secret,
/// in a folder whose `.env` holds the canary: the thread's messages, what the model was sent,
/// and the log.
async fn showing_a_secret<A: Approver>(
    call: (&'static str, serde_json::Value),
    approver: A,
) -> (Vec<Message>, String, Vec<Event>) {
    let dir = folder("full-secret");
    std::fs::write(dir.join(".env"), format!("TOKEN={CANARY}\n")).unwrap();
    let answers = vec![
        answer(None, vec![call], StopReason::ToolUse),
        answer(Some("Done."), vec![], StopReason::EndTurn),
    ];
    let (harness, seen) =
        harness_seeing(&dir, answers, approver, InMemoryLog::new(), 1, settings()).await;
    let mut thread = harness.start_thread(scope()).await.unwrap();
    turn(&harness, &mut thread, "check the token").await;
    let sent = format!("{:?}", seen.lock().unwrap());
    let events = events(harness.log(), thread.id).await;
    (thread.messages().to_vec(), sent, events)
}

#[test]
fn full_access_declines_a_secret_at_once_as_glave_with_or_without_someone_there() {
    let calls = [
        ("read", json!({ "path": ".env" })),
        ("bash", json!({ "command": "gh auth token" })),
        // glave's own credential helper, which would hand out a token.
        (
            "bash",
            json!({ "command": "cat /tmp/glave-credentials-1/git-credential-glave" }),
        ),
    ];
    block_on!(async {
        for call in calls {
            let person = Person::answering(Decision::AllowOnce);
            let (messages, sent, events) = showing_a_secret(call.clone(), person.clone()).await;
            assert!(person.asked().is_empty(), "no one is asked: {call:?}");
            let said = results(&messages);
            assert!(
                said[0].starts_with("Declined: this would show you secrets"),
                "{said:?}"
            );
            assert!(!sent.contains(CANARY), "the secret never reaches the model");
            let asked = events.iter().find_map(|event| match &event.body {
                Body::ApprovalRequested(asked) => Some(asked.clone()),
                _ => None,
            });
            let asked = asked.unwrap();
            assert!(asked.secret, "{call:?}");
            assert!(
                asked.caution.is_some_and(|caution| caution.ends_with(
                    "Declined without asking: Full access never shows the model a secret."
                )),
                "the person reads why no one was asked"
            );
            let decided = events.iter().find_map(|event| match &event.body {
                Body::ApprovalDecided(decided) => Some(decided.clone()),
                _ => None,
            });
            let decided = decided.unwrap();
            assert_eq!(decided.decision, Verdict::Deny);
            assert_eq!(decided.by, Some(Principal::glave()), "glave declined it");

            let (messages, sent, _) = showing_a_secret(call, NoOneToAsk).await;
            assert!(results(&messages)[0].starts_with("Declined: this would show you secrets"));
            assert!(!sent.contains(CANARY));
        }
    });
}
