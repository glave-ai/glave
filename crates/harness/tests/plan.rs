//! The thread's plan (`specs/approvals.md`, "Plan mode"). Plan mode changes nothing, in the
//! real sandbox and where none runs, asks no one, and ends its turn with the plan it proposed.
//! The plan is recorded, read from the log, edited by the person and told to the model, and
//! "Build it" approves it and builds it in Auto, never above the policy's cap. Outside plan
//! mode `update_plan` keeps the checklist without changing what the model is sent.

use std::path::Path;

use glave_event::{Body, Event, Outcome, Part, Permissions, PlanStatus, StopReason};
use glave_harness::{Cancel, Decision, HarnessError, NoOneToAsk, Settings};
use glave_ports::{Caller, Completion, InMemoryLog, Message, ModelRequest};
use serde_json::json;

use crate::outside_sandbox::{Person, results, sandboxed, walled};
use crate::support::*;

/// Settings for plan mode.
fn planning() -> Settings {
    let mut settings = settings();
    settings.permissions = Permissions::Plan;
    settings
}

/// The model's `update_plan` call with `steps`, each with its status.
fn plan(steps: &[(&str, &str)]) -> (&'static str, serde_json::Value) {
    let plan: Vec<_> = steps
        .iter()
        .map(|(step, status)| json!({"step": step, "status": status}))
        .collect();
    ("update_plan", json!({ "plan": plan }))
}

fn calls(calls: Vec<(&str, serde_json::Value)>) -> Completion {
    answer(None, calls, StopReason::ToolUse)
}

fn one(tool: &str, arguments: serde_json::Value) -> Completion {
    calls(vec![(tool, arguments)])
}

/// The plan the model proposes first: two steps, to do.
fn proposal() -> Completion {
    calls(vec![plan(&[
        ("Read the parser", "pending"),
        ("Add --verbose", "pending"),
    ])])
}

/// What the model was sent, each request, taken out of `seen`.
fn sent(seen: &Seen) -> Vec<ModelRequest> {
    seen.lock().unwrap().clone()
}

/// The text of every user message `request` sends, in order.
fn said(request: &ModelRequest) -> Vec<String> {
    request
        .messages
        .iter()
        .filter_map(|message| match message {
            Message::User { parts } => parts.iter().find_map(|part| match part {
                Part::Text { text } => Some(text.clone()),
                _ => None,
            }),
            _ => None,
        })
        .collect()
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

/// What a model in plan mode tries that would change something, in `base`'s project: a write,
/// an edit, a command that writes, one asking to leave the sandbox, a push and a secret's read;
/// then a command that reads, and the plan.
fn tries_everything(base: &Path) -> Vec<Completion> {
    let outside = base.join("out.txt").display().to_string();
    let edit = json!({"path": "README.md", "old_string": "keep", "new_string": "lose"});
    let leave = json!({"command": format!("echo x > {outside}"), "outside_sandbox": true, "reason": "it must"});
    vec![
        one("write", json!({"path": "notes.md", "content": "hi"})),
        one("edit", edit),
        one("bash", json!({"command": "echo x > ran.txt"})),
        one("bash", leave),
        one("bash", json!({"command": "git push origin main"})),
        one("read", json!({"path": ".env"})),
        one("bash", json!({"command": "cat README.md"})),
        proposal(),
        answer(Some("Never asked for."), vec![], StopReason::EndTurn),
    ]
}

/// What the model was told of each call `tries_everything` made, in order.
fn told_why(results: &[String]) {
    for refused in [0, 1, 4, 5] {
        let result = &results[refused];
        assert!(
            result.starts_with("Not run: plan mode changes nothing"),
            "{result}"
        );
    }
    let blocked = &results[2];
    assert!(
        blocked.contains("in plan mode commands cannot change"),
        "{blocked}"
    );
    assert!(results[3].contains("stay in the sandbox"), "{}", results[3]);
    assert!(results[6].contains("keep me"), "a command that reads runs");
}

/// What the model in plan mode was offered and told: the mode in its system prompt, `bash`
/// with no way out of the sandbox, and `update_plan`.
fn offered_for_a_plan(first: &ModelRequest) {
    assert!(first.system.contains("Plan mode:"), "{}", first.system);
    let shell = first.tools.iter().find(|tool| tool.name == "bash").unwrap();
    let leaves = shell.parameters.to_string().contains("outside_sandbox");
    assert!(!leaves, "none may leave the sandbox");
    assert!(first.tools.iter().any(|tool| tool.name == "update_plan"));
}

#[test]
fn plan_mode_changes_nothing_in_the_real_sandbox_and_asks_no_one() {
    let Some(base) = sandboxed("plan-mode") else {
        return;
    };
    let project = base.join("project");
    std::fs::write(project.join("README.md"), "keep me\n").unwrap();
    std::fs::write(project.join(".env"), "TOKEN=plan-canary\n").unwrap();
    block_on!(async {
        // Someone is there who would allow anything: plan mode asks them nothing.
        let person = Person::answering(Decision::AllowOnce);
        let workspace = walled(&base, false).await;
        let answers = tries_everything(&base);
        let log = InMemoryLog::new();
        let (harness, seen) = harness_with(workspace, answers, person.clone(), log, 1, planning());
        let mut thread = harness.start_thread(scope()).await.unwrap();
        let cancel = Cancel::new();
        let turn = harness
            .run_turn(&mut thread, "add a --verbose flag", &cancel, &mut |_| {})
            .await
            .unwrap();
        assert!(person.asked().is_empty(), "no one is asked in plan mode");
        for made in ["notes.md", "ran.txt"] {
            assert!(!project.join(made).exists(), "{made} was made");
        }
        assert!(!base.join("out.txt").exists(), "nothing left the sandbox");
        let readme = std::fs::read_to_string(project.join("README.md")).unwrap();
        assert_eq!(readme, "keep me\n");
        told_why(&results(thread.messages()));
        let sent = sent(&seen);
        assert_eq!(sent.len(), 8, "the turn ended with the plan it proposed");
        let leaked = format!("{sent:?}").contains("plan-canary");
        assert!(!leaked, "no secret reached the model");
        offered_for_a_plan(&sent[0]);
        assert_eq!(turn.outcome, Outcome::Completed);
        let proposed = harness.plan(scope(), thread.id).await.unwrap().unwrap();
        assert!(proposed.proposed && proposed.steps.len() == 2);
        let events = events(harness.log(), thread.id).await;
        assert!(
            of_kind(&events, "approval.requested").is_empty(),
            "nothing asked"
        );
    });
}

#[test]
fn with_no_one_there_and_no_sandbox_the_plan_is_the_answer_and_nothing_changes() {
    let dir = folder("plan-alone");
    std::fs::write(dir.join("README.md"), "keep me\n").unwrap();
    let answers = vec![
        one("write", json!({"path": "notes.md", "content": "hi"})),
        one("bash", json!({"command": "touch made.txt"})),
        one("bash", json!({"command": "ls"})),
        proposal(),
    ];
    block_on!(async {
        let log = InMemoryLog::new();
        let harness = harness_on(&dir, answers, NoOneToAsk, log, 1, planning()).await;
        let mut thread = harness.start_thread(scope()).await.unwrap();
        harness
            .run_turn(
                &mut thread,
                "add a --verbose flag",
                &Cancel::new(),
                &mut |_| {},
            )
            .await
            .unwrap();
        assert!(!dir.join("notes.md").exists() && !dir.join("made.txt").exists());
        let results = results(thread.messages());
        for refused in &results[..2] {
            assert!(refused.starts_with("Not run: plan mode"), "{refused}");
        }
        assert!(
            results[2].contains("README.md"),
            "a command that only reads runs"
        );
        assert!(
            harness
                .plan(scope(), thread.id)
                .await
                .unwrap()
                .is_some_and(|plan| plan.proposed)
        );
        let events = events(harness.log(), thread.id).await;
        assert!(
            of_kind(&events, "plan.approved").is_empty(),
            "nothing is built"
        );
        assert!(
            of_kind(&events, "approval.decided").is_empty(),
            "no one was asked"
        );
    });
}

/// The steps the person edits the proposal to.
fn edited() -> [String; 3] {
    [
        "Read the parser".to_owned(),
        "Add --verbose to the parser".to_owned(),
        "Run the tests".to_owned(),
    ]
}

/// The proposal, then the build: the model marks its steps as it goes, then is done.
fn proposed_then_built() -> Vec<Completion> {
    vec![
        proposal(),
        calls(vec![plan(&[
            ("Read the parser", "in_progress"),
            ("Add --verbose to the parser", "pending"),
            ("Run the tests", "pending"),
        ])]),
        calls(vec![plan(&[
            ("Read the parser", "completed"),
            ("Add --verbose to the parser", "completed"),
            ("Run the tests", "completed"),
        ])]),
        answer(Some("Built."), vec![], StopReason::EndTurn),
    ]
}

/// What the turn that built the plan was sent: glave's prompt with the plan as edited, out of
/// plan mode.
fn built_from_the_edit(build: &ModelRequest) {
    let prompt = said(build).pop().unwrap();
    assert!(
        prompt.starts_with("The person approved this plan."),
        "{prompt}"
    );
    assert!(prompt.contains("3. [pending] Run the tests"), "{prompt}");
    assert!(
        !build.system.contains("Plan mode:"),
        "no longer in plan mode"
    );
}

#[test]
fn a_proposed_plan_survives_a_resume_is_edited_and_is_built_from_the_edit_in_auto() {
    let dir = folder("plan-build");
    block_on!(async {
        let log = InMemoryLog::new();
        let answers = proposed_then_built();
        let (harness, seen) = harness_seeing(&dir, answers, NoOneToAsk, log, 1, planning()).await;
        let mut thread = harness.start_thread(scope()).await.unwrap();
        let cancel = Cancel::new();
        harness
            .run_turn(&mut thread, "add a --verbose flag", &cancel, &mut |_| {})
            .await
            .unwrap();
        let mut thread = harness.resume_thread(scope(), thread.id).await.unwrap();
        assert!(
            harness
                .plan(scope(), thread.id)
                .await
                .unwrap()
                .is_some_and(|plan| plan.proposed),
            "kept across a resume"
        );
        let person = Caller::default();
        harness
            .edit_plan(&mut thread, &person, &edited())
            .await
            .unwrap();
        let mut thread = harness.resume_thread(scope(), thread.id).await.unwrap();
        let steps: Vec<_> = (harness
            .plan(scope(), thread.id)
            .await
            .unwrap()
            .unwrap()
            .steps
            .iter())
        .map(|step| step.step.clone())
        .collect();
        assert_eq!(steps, edited(), "the edit is the plan");
        harness
            .build_plan(&mut thread, &person, &cancel, &mut |_| {})
            .await
            .unwrap();
        assert_eq!(
            thread.permissions(),
            Some(Permissions::Auto),
            "built in Auto"
        );
        let built = harness.plan(scope(), thread.id).await.unwrap().unwrap();
        let done = built
            .steps
            .iter()
            .all(|step| step.status == PlanStatus::Completed);
        assert!(!built.proposed && done, "{built:?}");
        built_from_the_edit(&sent(&seen)[1]);
        let events = events(harness.log(), thread.id).await;
        let kinds = kinds(&events);
        let approved = kinds
            .iter()
            .position(|kind| *kind == "plan.approved")
            .unwrap();
        assert_eq!(kinds[approved + 1], "permissions.chosen");
        assert_eq!(kinds[approved + 2], "turn.started");
        let Some(Body::MessageAdded(prompt)) = events.get(approved + 3).map(|e| &e.body) else {
            panic!("the build's prompt")
        };
        assert!(prompt.author.is_some(), "glave's words, not the person's");
    });
}

#[test]
fn changing_it_after_an_edit_tells_the_model_the_plan_first_and_proposes_again() {
    let dir = folder("plan-change");
    let answers = vec![
        proposal(),
        calls(vec![plan(&[("Read the parser", "pending")])]),
    ];
    block_on!(async {
        let log = InMemoryLog::new();
        let (harness, seen) = harness_seeing(&dir, answers, NoOneToAsk, log, 1, planning()).await;
        let mut thread = harness.start_thread(scope()).await.unwrap();
        let cancel = Cancel::new();
        harness
            .run_turn(&mut thread, "add a --verbose flag", &cancel, &mut |_| {})
            .await
            .unwrap();
        let edited = ["Read the parser".to_owned(), "Add -v too".to_owned()];
        harness
            .edit_plan(&mut thread, &Caller::default(), &edited)
            .await
            .unwrap();
        harness
            .run_turn(&mut thread, "drop the second step", &cancel, &mut |_| {})
            .await
            .unwrap();
        let sent = sent(&seen);
        assert_eq!(sent.len(), 2, "each turn ended with its plan");
        let said = said(&sent[1]);
        let note = &said[said.len() - 2];
        assert!(note.starts_with("[The person changed the plan."), "{note}");
        assert!(note.contains("2. [pending] Add -v too"), "{note}");
        assert_eq!(said.last().unwrap(), "drop the second step");
        let revised = harness.plan(scope(), thread.id).await.unwrap().unwrap();
        assert!(revised.proposed && revised.steps.len() == 1, "{revised:?}");
    });
}

#[test]
fn building_runs_in_the_highest_mode_the_policy_leaves_and_nothing_where_it_leaves_plan() {
    for (highest, built) in [
        (Permissions::FullAccess, Some(Permissions::Auto)),
        (Permissions::Ask, Some(Permissions::Ask)),
        (Permissions::Plan, None),
    ] {
        let dir = folder("plan-cap");
        let answers = vec![
            proposal(),
            answer(Some("Built."), vec![], StopReason::EndTurn),
        ];
        let mut settings = planning();
        settings.highest = highest;
        block_on!(async {
            let log = InMemoryLog::new();
            let harness = harness_on(&dir, answers, NoOneToAsk, log, 1, settings).await;
            let mut thread = harness.start_thread(scope()).await.unwrap();
            let cancel = Cancel::new();
            harness
                .run_turn(&mut thread, "plan it", &cancel, &mut |_| {})
                .await
                .unwrap();
            assert_eq!(harness.builds_in(), built);
            let ran = harness
                .build_plan(&mut thread, &Caller::default(), &cancel, &mut |_| {})
                .await;
            let events = events(harness.log(), thread.id).await;
            if let Some(mode) = built {
                ran.unwrap();
                assert_eq!(thread.permissions(), Some(mode), "under {highest:?}");
            } else {
                assert!(matches!(ran, Err(HarnessError::Invalid(_))), "{ran:?}");
                assert!(of_kind(&events, "plan.approved").is_empty());
                assert!(
                    harness
                        .plan(scope(), thread.id)
                        .await
                        .unwrap()
                        .is_some_and(|plan| plan.proposed),
                    "still waits"
                );
            }
        });
    }
}

#[test]
fn only_a_proposal_is_edited_or_built() {
    let dir = folder("plan-none");
    let answers = vec![
        calls(vec![plan(&[("Fix it", "in_progress")])]),
        answer(Some("Done."), vec![], StopReason::EndTurn),
    ];
    let mut settings = in_the_sandbox();
    settings.plans = true;
    block_on!(async {
        let log = InMemoryLog::new();
        let harness = harness_on(&dir, answers, NoOneToAsk, log, 1, settings).await;
        let mut thread = harness.start_thread(scope()).await.unwrap();
        let cancel = Cancel::new();
        harness
            .run_turn(&mut thread, "fix it", &cancel, &mut |_| {})
            .await
            .unwrap();
        assert!(
            harness
                .plan(scope(), thread.id)
                .await
                .unwrap()
                .is_some_and(|plan| !plan.proposed),
            "a checklist"
        );
        let person = Caller::default();
        let edited = harness
            .edit_plan(&mut thread, &person, &["x".to_owned()])
            .await;
        assert!(matches!(edited, Err(HarnessError::NoPlan(_))), "{edited:?}");
        let built = harness
            .build_plan(&mut thread, &person, &cancel, &mut |_| {})
            .await;
        assert!(matches!(built, Err(HarnessError::NoPlan(_))), "{built:?}");
    });
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

/// What a checklist kept outside plan mode came to, with plans on or off: whether
/// `update_plan` was offered, the call ids the model was sent, how many `plan.updated` were
/// recorded, and the plan.
fn checklist(plans: bool) -> (bool, Vec<String>, usize, Option<glave_harness::Plan>) {
    let answers = vec![
        calls(vec![plan(&[("Look", "in_progress"), ("Fix", "pending")])]),
        one("bash", json!({"command": "echo hi"})),
        calls(vec![plan(&[("Look", "completed"), ("Fix", "completed")])]),
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
        let sent = sent(&seen);
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
fn outside_plan_mode_the_checklist_is_kept_and_the_model_is_sent_the_same_ids() {
    let (offered, kept, updates, plan) = checklist(true);
    assert!(offered);
    assert_eq!(updates, 2);
    let plan = plan.unwrap();
    let done = plan
        .steps
        .iter()
        .all(|step| step.status == PlanStatus::Completed);
    assert!(!plan.proposed && done);
    let (offered, without, updates, plan) = checklist(false);
    assert!(!offered, "off, outside plan mode");
    assert_eq!((updates, plan), (0, None));
    assert_eq!(kept, without, "recording the plan changes no call's id");
}

#[test]
fn a_goal_does_not_run_in_plan_mode_which_stops_at_a_plan() {
    let dir = folder("plan-goal");
    block_on!(async {
        let log = InMemoryLog::new();
        let (harness, seen) =
            harness_seeing(&dir, vec![proposal()], NoOneToAsk, log, 1, planning()).await;
        let mut thread = harness.start_thread(scope()).await.unwrap();
        let goal = glave_harness::Goal {
            objective: "add a --verbose flag".to_owned(),
            token_budget: 1_000_000,
        };
        let caller = Caller::default();
        let started = harness
            .start_goal(&mut thread, goal, &caller, &Cancel::new(), &mut |_| {})
            .await;
        assert!(
            matches!(started, Err(HarnessError::GoalInPlan(_))),
            "{started:?}"
        );
        let continued = harness
            .continue_goal(&mut thread, &caller, &Cancel::new(), &mut |_| {})
            .await;
        assert!(
            matches!(continued, Err(HarnessError::GoalInPlan(_))),
            "{continued:?}"
        );
        assert!(sent(&seen).is_empty(), "the model is never called");
        let events = events(harness.log(), thread.id).await;
        assert!(of_kind(&events, "goal.set").is_empty(), "no goal is set");
    });
}
