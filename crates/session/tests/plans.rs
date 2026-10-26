//! A thread's plan over the session API (`specs/session.md`, "Plans"): kept by the model, read
//! with `thread/plan` as typed facts and a sentence, `null` before there is one, and read the
//! same by a fresh server from the log.

use glave_event::{PlanStatus, ThreadId};
use glave_ports::Completion;
use glave_session::protocol::{PlanInfo, Reply, Request};
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

#[test]
fn the_plan_the_model_keeps_is_read_as_facts_and_a_sentence_and_a_fresh_server_reads_it_too() {
    let dir = folder("plans-read");
    block_on!(async {
        let log = Shared::default();
        let mut setup = Setup::new(vec![
            plan(&[
                ("Read the parser", "in_progress"),
                ("Add --verbose", "pending"),
            ]),
            plan(&[
                ("Read the parser", "completed"),
                ("Add --verbose", "in_progress"),
            ]),
            answer(Some("Half done."), None),
        ]);
        setup.settings.titles = false;
        setup.settings.plans = true;
        let (server, _seen) = setup.build_on(&dir, log.clone()).await;
        let thread = start(&server, false).await;
        assert_eq!(plan_of(&server, thread).await, None, "none yet");
        turn(&server, thread, "add a --verbose flag").await.unwrap();
        let kept = plan_of(&server, thread).await.unwrap();
        let steps: Vec<_> = kept
            .steps
            .iter()
            .map(|step| (step.step.as_str(), step.status))
            .collect();
        assert_eq!(
            steps,
            [
                ("Read the parser", PlanStatus::Completed),
                ("Add --verbose", PlanStatus::InProgress),
            ],
            "the latest update"
        );
        assert_eq!(&*kept.sentence, "Plan: 1 of 2 steps done");
        server.shutdown().await;
        let (fresh, _) = Setup::new(Vec::new()).build_on(&dir, log).await;
        assert_eq!(
            plan_of(&fresh, thread).await,
            Some(kept),
            "read from the log"
        );
        fresh.shutdown().await;
    });
}
