//! A thread's plan over the session API (`specs/session.md`, "Plans"): proposed in plan mode,
//! read with `thread/plan` as typed facts and a sentence, edited with `plan/edit`, and built
//! with `plan/build`, which records the approval, runs the thread in Auto and streams the
//! build; a fresh server reads the same plan from the log. Editing or building with no plan
//! waiting is refused, and so is building where the policy leaves only Plan, an edit a plan
//! cannot hold, and a goal in plan mode.

use glave_event::{PlanStatus, ThreadId};
use glave_ports::Completion;
use glave_session::protocol::{ApiError, Meta, Permissions, PlanInfo, Reply, Request, code};
use serde_json::json;

use crate::support::*;

/// The model's `update_plan` call with `steps`, each with its status.
fn plan(steps: &[(&str, &str)]) -> Completion {
    let plan: Vec<_> = steps
        .iter()
        .map(|(step, status)| json!({"step": step, "status": status}))
        .collect();
    answer(None, Some(("update_plan", json!({ "plan": plan }))))
}

/// A turn in plan mode, saying `prompt`.
fn planning(thread: ThreadId, prompt: &str) -> Request {
    Request::TurnStart {
        thread,
        prompt: prompt.into(),
        background: false,
        budget: None,
        meta: Meta::default(),
        model: None,
        permissions: Some(Permissions::Plan),
        unattended: false,
        files: Vec::new(),
        effort: None,
    }
}

async fn plan_of<L: glave_ports::EventLog + 'static>(
    server: &Tested<L>,
    thread: ThreadId,
) -> Option<PlanInfo> {
    match server
        .request(Request::ThreadPlan { thread })
        .await
        .unwrap()
    {
        Reply::Plan { plan } => plan,
        other => panic!("a plan, not {other:?}"),
    }
}

async fn mode<L: glave_ports::EventLog + 'static>(
    server: &Tested<L>,
    thread: ThreadId,
) -> Permissions {
    match server.request(Request::ThreadGet { thread }).await.unwrap() {
        Reply::Thread { thread } => thread.permissions,
        other => panic!("a thread, not {other:?}"),
    }
}

fn steps(plan: &PlanInfo) -> Vec<(&str, PlanStatus)> {
    plan.steps
        .iter()
        .map(|step| (step.step.as_str(), step.status))
        .collect()
}

/// The proposal, then the build: the model marks its steps as it goes, then is done.
fn proposed_then_built() -> Vec<Completion> {
    vec![
        plan(&[("Read the parser", "pending"), ("Add --verbose", "pending")]),
        plan(&[
            ("Read the parser", "completed"),
            ("Add --verbose to the parser", "in_progress"),
            ("Run the tests", "pending"),
        ]),
        plan(&[
            ("Read the parser", "completed"),
            ("Add --verbose to the parser", "completed"),
            ("Run the tests", "completed"),
        ]),
        answer(Some("Built."), None),
    ]
}

/// Edits `thread`'s proposal to three steps, and checks the plan it answers with.
async fn edit_to_three(server: &Tested<Shared>, thread: ThreadId) {
    let steps_given = vec![
        "Read the parser".into(),
        " Add --verbose to the parser ".into(),
        "Run the tests".into(),
    ];
    let edit = Request::PlanEdit {
        thread,
        steps: steps_given,
    };
    let edited = server.request(edit).await.unwrap();
    let Reply::Plan { plan: Some(edited) } = edited else {
        panic!("the plan, not {edited:?}")
    };
    assert_eq!(&*edited.sentence, "Proposed plan: 3 steps");
    assert_eq!(
        steps(&edited),
        [
            ("Read the parser", PlanStatus::Pending),
            ("Add --verbose to the parser", PlanStatus::Pending),
            ("Run the tests", PlanStatus::Pending),
        ],
        "trimmed, each to do"
    );
}

#[test]
fn a_proposed_plan_is_read_edited_and_built_in_auto_and_a_fresh_server_reads_it_too() {
    let dir = folder("plans-build");
    block_on!(async {
        let log = Shared::default();
        let mut setup = Setup::new(proposed_then_built());
        setup.settings.titles = false;
        let (server, seen) = setup.build_on(&dir, log.clone()).await;
        let thread = start(&server, false).await;
        server
            .request(planning(thread, "add a --verbose flag"))
            .await
            .unwrap();
        let proposed = plan_of(&server, thread).await.unwrap();
        assert!(proposed.proposed);
        assert_eq!(&*proposed.sentence, "Proposed plan: 2 steps");
        assert_eq!(mode(&server, thread).await, Permissions::Plan);
        edit_to_three(&server, thread).await;
        let built = server.request(Request::PlanBuild { thread }).await.unwrap();
        let Reply::Turn { turn } = built else {
            panic!("the build's turn, not {built:?}")
        };
        assert_eq!(turn.outcome, glave_event::Outcome::Completed);
        assert_eq!(
            mode(&server, thread).await,
            Permissions::Auto,
            "built in Auto"
        );
        let done = plan_of(&server, thread).await.unwrap();
        assert!(!done.proposed);
        assert_eq!(&*done.sentence, "Plan: 3 of 3 steps done");
        let prompt = format!("{:?}", seen.lock().unwrap()[1].messages);
        assert!(
            prompt.contains("Add --verbose to the parser"),
            "built from the edit"
        );
        assert!(
            kinds(&server, thread)
                .await
                .contains(&"plan.approved".to_owned())
        );
        server.shutdown().await;
        let (fresh, _) = Setup::new(Vec::new()).build_on(&dir, log).await;
        assert_eq!(
            plan_of(&fresh, thread).await,
            Some(done),
            "read from the log"
        );
        fresh.shutdown().await;
    });
}

fn refused(answered: Result<Reply, ApiError>) -> ApiError {
    match answered {
        Err(error) => error,
        Ok(reply) => panic!("refused, not {reply:?}"),
    }
}

#[test]
fn editing_or_building_with_no_plan_waiting_is_refused_saying_so() {
    let dir = folder("plans-none");
    block_on!(async {
        let mut setup = Setup::new(vec![
            plan(&[("Fix it", "in_progress")]),
            answer(Some("Fixed."), None),
        ]);
        setup.settings.titles = false;
        setup.settings.plans = true;
        let (server, _seen) = setup.build(&dir).await;
        let thread = start(&server, false).await;
        assert_eq!(plan_of(&server, thread).await, None, "none yet");
        turn(&server, thread, "fix it").await.unwrap();
        let checklist = plan_of(&server, thread).await.unwrap();
        assert!(!checklist.proposed);
        assert_eq!(&*checklist.sentence, "Plan: 0 of 1 step done");
        let edit = Request::PlanEdit {
            thread,
            steps: vec!["x".into()],
        };
        let edit = refused(server.request(edit).await);
        assert_eq!(edit.code, code::NO_PLAN);
        assert!(
            edit.message.contains("plan mode proposes one"),
            "{}",
            edit.message
        );
        let build = refused(server.request(Request::PlanBuild { thread }).await);
        assert_eq!(build.code, code::NO_PLAN);
        server.shutdown().await;
    });
}

#[test]
fn where_the_policy_leaves_only_plan_a_plan_is_proposed_and_never_built_nor_a_goal_run() {
    let dir = folder("plans-capped");
    block_on!(async {
        let mut setup = Setup::new(vec![plan(&[("Look", "pending")])]);
        setup.settings.titles = false;
        setup.settings.permissions = Permissions::Plan;
        setup.settings.highest = Permissions::Plan;
        let (server, _seen) = setup.build(&dir).await;
        let thread = start(&server, false).await;
        turn(&server, thread, "plan it").await.unwrap();
        assert!(plan_of(&server, thread).await.unwrap().proposed);
        let build = refused(server.request(Request::PlanBuild { thread }).await);
        assert_eq!(build.code, code::NOT_ALLOWED);
        assert_eq!(
            build.message.to_string(),
            "a plan can't be built here: only Plan is allowed"
        );
        let edit = Request::PlanEdit {
            thread,
            steps: vec![String::new()],
        };
        let blank = refused(server.request(edit).await);
        assert_eq!(blank.code, code::INVALID_PARAMS, "{blank:?}");
        assert!(
            blank
                .message
                .to_string()
                .starts_with("a plan has 1 to 64 steps"),
            "{blank:?}"
        );
        let goal = Request::GoalStart {
            background: false,
            thread,
            objective: Some("add a --verbose flag".into()),
            budget: None,
            unattended: false,
        };
        let goal = refused(server.request(goal).await);
        assert_eq!(goal.code, code::NOT_ALLOWED, "{goal:?}");
        assert!(
            goal.message
                .to_string()
                .starts_with("a goal doesn't run in Plan"),
            "{goal:?}"
        );
        server.shutdown().await;
    });
}
