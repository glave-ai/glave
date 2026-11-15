//! An answer that repeats itself as it streams, as DeepSeek V4.1 Flash's "Emit. Done. Now."
//! did for 195 KB: stopped, recorded as used with none of its words, and asked once more; a
//! second that repeats ends the turn, saying so. Every call asks for a bounded answer.

use std::sync::Arc;

use glave_event::{Body, Event, ModelResponded, Outcome, StopReason};
use glave_harness::{Cancel, HarnessError};
use glave_ports::InMemoryLog;
use tokio::sync::Notify;

use crate::support::*;

/// An answer that falls into a loop after a good start.
fn looping() -> glave_ports::Completion {
    let text = format!(
        "The codename is **Saffron Marten**.\n\n{}",
        "Emit.\n\nDone.\n\nNow.\n\nAlso — emit.\n\n".repeat(200)
    );
    answer(Some(&text), vec![], StopReason::EndTurn)
}

fn responded(events: &[Event]) -> Vec<&ModelResponded> {
    events
        .iter()
        .filter_map(|event| match &event.body {
            Body::ModelResponded(responded) => Some(responded),
            _ => None,
        })
        .collect()
}

fn said_emit(events: &[Event]) -> bool {
    events.iter().any(|event| {
        matches!(&event.body, Body::MessageAdded(message) if format!("{:?}", message.parts).contains("Emit."))
    })
}

#[test]
fn an_answer_that_repeats_is_stopped_recorded_and_asked_once_more() {
    block_on!(async {
        let dir = folder("repeating-once");
        let answers = vec![
            looping(),
            answer(Some("Saffron Marten."), vec![], StopReason::EndTurn),
        ];
        let (harness, seen) = harness_seeing(
            &dir,
            answers,
            glave_harness::AllowAll,
            InMemoryLog::new(),
            1,
            settings(),
        )
        .await;
        let mut thread = harness.start_thread(scope()).await.unwrap();
        let ended = turn(&harness, &mut thread, "what is the codename?").await;
        assert_eq!(ended.outcome, Outcome::Completed);
        assert_eq!(ended.model_calls, 2);
        let seen = seen.lock().unwrap().clone();
        assert_eq!(
            format!("{:?}", seen[0]),
            format!("{:?}", seen[1]),
            "asked once more with the same request"
        );
        assert_eq!(seen[0].max_output_tokens, Some(32_000), "a bounded answer");
        let events = events(harness.log(), thread.id).await;
        let calls = responded(&events);
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0].stop, StopReason::Repeating);
        assert!(calls[0].usage.estimated(), "what it used, estimated");
        let used = calls[0].usage.counts();
        assert!(used.input > 0 && used.output > 0, "{used:?}");
        assert!(ended.estimated && ended.tokens.output >= used.output);
        assert!(!said_emit(&events), "none of its words are kept");
    });
}

#[test]
fn an_answer_that_repeats_again_ends_the_turn_saying_so() {
    block_on!(async {
        let dir = folder("repeating-twice");
        // Never finishes by itself, as the stream that ran to the deadline.
        let never = Arc::new(Notify::new());
        let script = ScriptedModel::new(vec![looping(), looping()]).gated(never);
        let (harness, seen) = scripted(
            workspace(&dir).await,
            script,
            glave_harness::AllowAll,
            InMemoryLog::new(),
            1,
            settings(),
        );
        let mut thread = harness.start_thread(scope()).await.unwrap();
        let ran = harness
            .run_turn(
                &mut thread,
                "what is the codename?",
                &Cancel::new(),
                &mut |_| {},
            )
            .await;
        let Err(error) = ran else {
            panic!("the turn went on: {ran:?}");
        };
        assert!(matches!(error, HarnessError::Repeating), "{error:?}");
        assert_eq!(
            error.to_string(),
            "the model started repeating itself, so glave stopped it: try again, or pick \
             another model"
        );
        assert_eq!(seen.lock().unwrap().len(), 2, "asked once more, no more");
        let events = events(harness.log(), thread.id).await;
        let stops: Vec<StopReason> = responded(&events).iter().map(|r| r.stop).collect();
        assert_eq!(stops, [StopReason::Repeating, StopReason::Repeating]);
        let outcome = events.iter().find_map(|event| match &event.body {
            Body::TurnEnded(ended) => Some(ended.outcome),
            _ => None,
        });
        assert_eq!(outcome, Some(Outcome::Failed));
        assert!(!said_emit(&events));
    });
}
