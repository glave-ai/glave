//! What a service asks the person mid-call: a durable wait the turn stops at, holding nothing,
//! and goes on from with the answer; what they typed reaches the source alone; and where no one
//! can answer, or the turn is stopped, nothing waits.

use std::path::Path;
use std::sync::Arc;

use glave_event::{
    Body, CallId, Event, Outcome, Seq, ServiceAnswer, ServiceOutcome, StopReason, Verdict,
};
use glave_harness::{Approver, Cancel, Decision, Harness, HarnessError, NoOneToAsk, Thread};
use glave_ports::testing::{ASKING_TOOL, AskingTool, ScriptedModel};
use glave_ports::{
    BlobStore, Caller, EventLog, InMemoryBlobs, InMemoryLog, ModelProvider, ServiceGiven,
    ToolSource, WaitKind, block_on,
};
use serde_json::json;

use crate::network::InTheLog;
use crate::support::*;

const TYPED: &str = "only the service reads this";

fn calling(then: &str) -> Vec<glave_ports::Completion> {
    vec![
        answer(None, vec![(ASKING_TOOL, json!({}))], StopReason::ToolUse),
        answer(Some(then), vec![], StopReason::EndTurn),
    ]
}

/// A tool that asks one thing, a harness whose model calls it, and a thread there.
async fn asked_once<A: Approver>(
    dir: &Path,
    approver: A,
) -> (
    Arc<AskingTool>,
    Harness<ScriptedModel, InMemoryLog, InMemoryBlobs, A>,
    Thread,
) {
    let source = AskingTool::asking(&["Which repository?"]);
    let harness = harness(dir, calling("Filed."), approver).await;
    let harness = harness.with_tools(vec![Arc::clone(&source) as Arc<dyn ToolSource>]);
    let thread = harness.start_thread(scope()).await.unwrap();
    (source, harness, thread)
}

/// The thread's turn run to where it stops: it must stop at a wait.
async fn stops_at_a_wait<P, L, B, A>(harness: &Harness<P, L, B, A>, thread: &mut Thread)
where
    P: ModelProvider,
    L: EventLog,
    B: BlobStore,
    A: Approver,
{
    let ran = harness
        .run_turn(thread, "file it", &Cancel::new(), &mut |_| {})
        .await;
    assert!(matches!(ran, Err(HarnessError::Waits(_))), "{ran:?}");
}

/// What the latest `service.asked` in `events` waits at, and its call.
fn asked_at(events: &[Event]) -> (CallId, Seq) {
    events
        .iter()
        .rev()
        .find_map(|event| match &event.body {
            Body::ServiceAsked(asked) => Some((asked.call_id.clone(), event.header.seq)),
            _ => None,
        })
        .expect("it asked")
}

fn typed() -> ServiceAnswer {
    ServiceAnswer::Answered {
        values: json!({"note": TYPED}).as_object().unwrap().clone(),
    }
}

/// Every event's stored form, as a client reads it.
async fn raw(log: &impl EventLog, thread: glave_event::ThreadId) -> String {
    let events = log.read(&scope(), thread, None, usize::MAX).await.unwrap();
    events
        .iter()
        .map(|event| event.to_json().unwrap())
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn a_call_that_asks_waits_in_the_log_and_goes_on_with_the_answer() {
    let dir = folder("service-answered");
    block_on!(async {
        let source = AskingTool::asking(&["Which repository?"]);
        let (harness, seen) = harness_seeing(
            &dir,
            calling("Filed."),
            InTheLog,
            InMemoryLog::new(),
            1,
            settings(),
        )
        .await;
        let harness = harness.with_tools(vec![Arc::clone(&source) as Arc<dyn ToolSource>]);
        let mut thread = harness.start_thread(scope()).await.unwrap();
        stops_at_a_wait(&harness, &mut thread).await;
        assert!(thread.waits());

        let events = events(harness.log(), thread.id).await;
        let (call, at) = asked_at(&events);
        let asked = events.iter().find_map(|event| match &event.body {
            Body::ServiceAsked(asked) => Some(asked.clone()),
            _ => None,
        });
        let asked = asked.unwrap();
        assert_eq!(asked.ask, "GitHub asks you, not the model");
        assert_eq!(asked.message, "Which repository?");
        let view = harness
            .log()
            .thread_view(&scope(), thread.id)
            .await
            .unwrap();
        let wait = view.and_then(|view| view.turn).and_then(|turn| turn.wait);
        assert!(wait.is_some_and(|wait| wait.kind == WaitKind::Service && wait.seq == at));
        assert_eq!(seen.lock().unwrap().len(), 1, "the model waits too");

        harness
            .answer_service(scope(), thread.id, &call, at, typed(), None)
            .await
            .unwrap();
        let ended = harness
            .go_on(&mut thread, &Caller::default(), &Cancel::new(), &mut |_| {})
            .await
            .unwrap();
        assert_eq!(ended.outcome, Outcome::Completed);
        assert_eq!(ended.tool_calls, 1);

        // The source had it back, what it kept and what was typed.
        let given = source.given();
        assert_eq!(given.len(), 2);
        // Compared without printing: a failure must not show what was typed.
        let same = given[1]
            == Some(ServiceGiven {
                state: json!({"round": "kept"}),
                answer: typed(),
            });
        assert!(same);
        // No event, and nothing the model is sent, holds what was typed.
        assert!(!raw(harness.log(), thread.id).await.contains(TYPED));
        let sent = format!("{:?}", seen.lock().unwrap().last().unwrap().messages);
        assert!(!sent.contains(TYPED));
        let events = crate::support::events(harness.log(), thread.id).await;
        let told = events.iter().find_map(|event| match &event.body {
            Body::ServiceAnswered(answered) => Some(answered.told.clone()),
            _ => None,
        });
        assert_eq!(told.as_deref(), Some("You answered GitHub"));
    });
}

#[test]
fn an_answer_is_refused_unless_it_is_to_what_waits_and_fits_it() {
    let dir = folder("service-refused");
    block_on!(async {
        let source = AskingTool::asking(&["First?", "Then?"]);
        let harness = harness(&dir, calling("Filed."), InTheLog).await;
        let harness = harness.with_tools(vec![Arc::clone(&source) as Arc<dyn ToolSource>]);
        let mut thread = harness.start_thread(scope()).await.unwrap();
        stops_at_a_wait(&harness, &mut thread).await;
        let (call, first) = asked_at(&events(harness.log(), thread.id).await);
        let empty = ServiceAnswer::Answered {
            values: serde_json::Map::new(),
        };
        let misfit = harness
            .answer_service(scope(), thread.id, &call, first, empty, None)
            .await;
        assert!(
            matches!(misfit, Err(HarnessError::Invalid(_))),
            "{misfit:?}"
        );
        harness
            .answer_service(scope(), thread.id, &call, first, typed(), None)
            .await
            .unwrap();
        let ran = harness
            .go_on(&mut thread, &Caller::default(), &Cancel::new(), &mut |_| {})
            .await;
        assert!(matches!(ran, Err(HarnessError::Waits(_))), "it asked again");
        // A retry of the first answer must not land on the second question.
        let stale = harness
            .answer_service(scope(), thread.id, &call, first, typed(), None)
            .await;
        assert!(
            matches!(stale, Err(HarnessError::NoServiceAsk(_))),
            "{stale:?}"
        );
        let (_, second) = asked_at(&events(harness.log(), thread.id).await);
        harness
            .answer_service(
                scope(),
                thread.id,
                &call,
                second,
                ServiceAnswer::Declined,
                None,
            )
            .await
            .unwrap();
        harness
            .go_on(&mut thread, &Caller::default(), &Cancel::new(), &mut |_| {})
            .await
            .unwrap();
        let given = source.given();
        assert_eq!(
            given.last().unwrap().as_ref().unwrap().answer,
            ServiceAnswer::Declined
        );
    });
}

#[test]
fn an_approved_call_that_then_asks_waits_at_what_it_asks_through_a_restart() {
    let dir = folder("service-approved");
    block_on!(async {
        let source = AskingTool::asking(&["Which repository?"]);
        let harness = asking_harness(&dir, calling("Filed."), InTheLog).await;
        let harness = harness.with_tools(vec![Arc::clone(&source) as Arc<dyn ToolSource>]);
        let mut thread = harness.start_thread(scope()).await.unwrap();
        stops_at_a_wait(&harness, &mut thread).await;
        let approval = events(harness.log(), thread.id)
            .await
            .iter()
            .find_map(|event| match &event.body {
                Body::ApprovalRequested(asked) => Some(asked.call_id.clone()),
                _ => None,
            })
            .expect("it asks before it runs");
        harness
            .decide(scope(), thread.id, &approval, Decision::AllowOnce, None)
            .await
            .unwrap();
        let ran = harness
            .go_on(&mut thread, &Caller::default(), &Cancel::new(), &mut |_| {})
            .await;
        assert!(matches!(ran, Err(HarnessError::Waits(_))), "{ran:?}");

        // A restart reads it as waiting, not as a crash's leftover, and closes nothing.
        assert!(!harness.repair(scope(), thread.id).await.unwrap());
        let mut read_again = harness.resume_thread(scope(), thread.id).await.unwrap();
        assert!(read_again.waits());
        let (call, at) = asked_at(&events(harness.log(), thread.id).await);
        harness
            .answer_service(scope(), thread.id, &call, at, typed(), None)
            .await
            .unwrap();
        harness
            .go_on(
                &mut read_again,
                &Caller::default(),
                &Cancel::new(),
                &mut |_| {},
            )
            .await
            .unwrap();
        // It runs with the answer: not asked about again, nor run from its start.
        let events = events(harness.log(), thread.id).await;
        let verdicts = events.iter().filter_map(|event| match &event.body {
            Body::ApprovalDecided(decided) => Some(decided.decision),
            _ => None,
        });
        assert_eq!(verdicts.collect::<Vec<_>>(), [Verdict::AllowOnce]);
        assert_eq!(source.given().len(), 2);
        let results = events
            .iter()
            .filter(|event| matches!(event.body, Body::ToolCallCompleted(_)))
            .count();
        assert_eq!(results, 1);
    });
}

#[test]
fn an_answer_a_crash_left_unused_is_closed_as_a_leftover() {
    let dir = folder("service-crashed");
    block_on!(async {
        let (_, harness, mut thread) = asked_once(&dir, InTheLog).await;
        stops_at_a_wait(&harness, &mut thread).await;
        let (call, at) = asked_at(&events(harness.log(), thread.id).await);
        harness
            .answer_service(scope(), thread.id, &call, at, typed(), None)
            .await
            .unwrap();
        assert!(harness.repair(scope(), thread.id).await.unwrap());
        let events = events(harness.log(), thread.id).await;
        assert!(events.iter().any(|event| matches!(
            &event.body,
            Body::TurnEnded(ended) if ended.outcome == Outcome::Interrupted
        )));
    });
}

#[test]
fn stopping_a_turn_that_waits_settles_what_was_asked() {
    let dir = folder("service-stopped");
    block_on!(async {
        let (source, harness, mut thread) = asked_once(&dir, InTheLog).await;
        stops_at_a_wait(&harness, &mut thread).await;
        let stop = Cancel::new();
        stop.cancel();
        let ended = harness
            .go_on(&mut thread, &Caller::default(), &stop, &mut |_| {})
            .await
            .unwrap();
        assert_eq!(ended.outcome, Outcome::Cancelled);
        let events = events(harness.log(), thread.id).await;
        let told = events.iter().find_map(|event| match &event.body {
            Body::ServiceAnswered(answered) => Some((answered.outcome, answered.told.clone())),
            _ => None,
        });
        assert_eq!(
            told,
            Some((
                ServiceOutcome::Cancelled,
                "GitHub's question went unanswered".to_owned()
            ))
        );
        assert_eq!(source.given().len(), 1, "it is not called again");
    });
}

#[test]
fn with_no_one_to_answer_the_source_declines_and_nothing_waits() {
    let dir = folder("service-nobody");
    block_on!(async {
        let (source, harness, mut thread) = asked_once(&dir, NoOneToAsk).await;
        let ended = turn(&harness, &mut thread, "file it").await;
        assert_eq!(ended.outcome, Outcome::Completed);
        assert_eq!(source.unasked(), 1);
        assert_eq!(source.given(), Vec::new());
        let events = events(harness.log(), thread.id).await;
        assert!(!kinds(&events).contains(&"service.asked"));
    });
}

#[test]
fn a_subagents_call_never_reaches_the_person() {
    let dir = folder("service-subagent");
    block_on!(async {
        let source = AskingTool::asking(&["Which repository?"]);
        let answers = vec![
            answer(
                None,
                vec![(
                    "agent",
                    json!({"description": "file it", "prompt": "file it"}),
                )],
                StopReason::ToolUse,
            ),
            answer(None, vec![(ASKING_TOOL, json!({}))], StopReason::ToolUse),
            answer(Some("Child filed."), vec![], StopReason::EndTurn),
            answer(Some("Filed."), vec![], StopReason::EndTurn),
        ];
        let mut subagents = settings();
        subagents.subagents = true;
        let harness = harness_on(&dir, answers, InTheLog, InMemoryLog::new(), 1, subagents).await;
        let harness = harness.with_tools(vec![Arc::clone(&source) as Arc<dyn ToolSource>]);
        let mut thread = harness.start_thread(scope()).await.unwrap();
        let ended = turn(&harness, &mut thread, "file it").await;
        assert_eq!(ended.outcome, Outcome::Completed);
        assert_eq!(source.unasked(), 1);
        assert_eq!(source.given(), Vec::new());
    });
}
