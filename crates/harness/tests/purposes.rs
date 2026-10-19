//! Every model call says what it is for and whose budgets it counts in, so a gateway can route
//! and meter it; and `model.responded` records the route and target that served it. The
//! judge's calls are recorded in the thread whose action they judged.

use glave_event::{Body, ModelResponded, StopReason, ThreadId};
use glave_harness::{Cancel, NoOneToAsk};
use glave_ports::{InMemoryLog, ModelRequest, Purpose, Served};
use serde_json::json;

use crate::support::*;

fn asked(seen: &Seen) -> Vec<(Purpose, Vec<ThreadId>)> {
    let seen: Vec<ModelRequest> = seen.lock().unwrap().clone();
    seen.into_iter()
        .map(|request| (request.purpose, request.threads))
        .collect()
}

fn responses(events: &[glave_event::Event]) -> Vec<ModelResponded> {
    events
        .iter()
        .filter_map(|event| match &event.body {
            Body::ModelResponded(responded) => Some(responded.clone()),
            _ => None,
        })
        .collect()
}

fn served(route: &str, target: &str) -> Served {
    Served {
        route: route.into(),
        target: target.into(),
        reasoning_stripped: route == "summary",
    }
}

#[test]
fn a_turn_and_its_summary_say_what_they_are_for_and_who_served_them() {
    let dir = folder("purposes-turn");
    block_on!(async {
        let mut said = answer(Some("We said hello."), vec![], StopReason::EndTurn);
        said.served = Some(served("main", "openrouter/big"));
        let mut summary = answer(Some("They said hello."), vec![], StopReason::EndTurn);
        summary.served = Some(served("summary", "local/small"));
        let (harness, seen) = harness_seeing(
            &dir,
            vec![said, summary],
            NoOneToAsk,
            InMemoryLog::new(),
            1,
            settings(),
        )
        .await;
        let mut thread = harness.start_thread(scope()).await.unwrap();
        harness
            .run_turn(&mut thread, "hello", &Cancel::new(), &mut |_| {})
            .await
            .unwrap();
        harness
            .compact(&mut thread, &Cancel::new(), &mut |_| {})
            .await
            .unwrap();
        assert_eq!(
            asked(&seen),
            [
                (Purpose::Main, vec![thread.id]),
                (Purpose::Summary, vec![thread.id])
            ]
        );
        let recorded = responses(&events(harness.log(), thread.id).await);
        let routes: Vec<_> = recorded
            .iter()
            .map(|r| {
                (
                    r.route.as_deref(),
                    r.target.as_deref(),
                    r.reasoning_stripped,
                )
            })
            .collect();
        assert_eq!(
            routes,
            [
                (Some("main"), Some("openrouter/big"), false),
                (Some("summary"), Some("local/small"), true)
            ]
        );
    });
}

#[test]
fn a_subagent_calls_as_a_subagent_within_its_parents_budget() {
    let dir = folder("purposes-subagent");
    block_on!(async {
        let answers = vec![
            answer(
                None,
                vec![(
                    "agent",
                    json!({"description": "look", "prompt": "Look around.", "subagent_type": "explore"}),
                )],
                StopReason::ToolUse,
            ),
            answer(Some("Nothing here."), vec![], StopReason::EndTurn),
            answer(Some("Done."), vec![], StopReason::EndTurn),
        ];
        let mut settings = settings();
        settings.subagents = true;
        let (harness, seen) =
            harness_seeing(&dir, answers, NoOneToAsk, InMemoryLog::new(), 1, settings).await;
        let mut thread = harness.start_thread(scope()).await.unwrap();
        harness
            .run_turn(&mut thread, "look around", &Cancel::new(), &mut |_| {})
            .await
            .unwrap();
        let calls = asked(&seen);
        assert_eq!(calls[0], (Purpose::Main, vec![thread.id]));
        let (purpose, threads) = &calls[1];
        assert_eq!(*purpose, Purpose::Subagent);
        assert_eq!(threads.len(), 2);
        assert_ne!(threads[0], thread.id, "the child's own thread first");
        assert_eq!(threads[1], thread.id, "then its parent's");
        assert_eq!(calls[2], (Purpose::Main, vec![thread.id]));
    });
}

#[test]
fn the_judges_call_is_recorded_and_counted_in_the_thread_it_judged() {
    let dir = folder("purposes-judge");
    block_on!(async {
        let mut ruling = answer(
            Some(r#"{"decision":"run","reason":"the person asked to push"}"#),
            vec![],
            StopReason::EndTurn,
        );
        ruling.served = Some(served("judge", "openrouter/small"));
        let answers = vec![
            answer(
                None,
                vec![("bash", json!({"command": "git push origin main"}))],
                StopReason::ToolUse,
            ),
            ruling,
            answer(Some("Pushed."), vec![], StopReason::EndTurn),
        ];
        let mut settings = settings();
        settings.judge = glave_ports::settings::Judge::Main;
        let (harness, seen) =
            harness_seeing(&dir, answers, NoOneToAsk, InMemoryLog::new(), 1, settings).await;
        let mut thread = harness.start_thread(scope()).await.unwrap();
        let summary = harness
            .run_turn(&mut thread, "push to main", &Cancel::new(), &mut |_| {})
            .await
            .unwrap();
        assert_eq!(asked(&seen)[1], (Purpose::Judge, vec![thread.id]));
        let recorded = responses(&events(harness.log(), thread.id).await);
        let routes: Vec<_> = recorded.iter().map(|r| r.route.as_deref()).collect();
        assert_eq!(routes, [None, Some("judge"), None]);
        assert_eq!(
            summary.model_calls, 3,
            "the judge's call counts in the turn"
        );
        assert_eq!(summary.tokens.input, 300);
    });
}
