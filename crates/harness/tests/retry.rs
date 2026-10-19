//! Retrying model calls against a model that fails on cue: failures that may pass are tried
//! again and the person sees each retry; failures that will not pass end the turn at once; and
//! a cancel during the wait between tries ends it cleanly.

use std::path::Path;
use std::time::Duration;

use glave_event::{Outcome, StopReason};
use glave_harness::{AllowAll, Cancel, Harness, HarnessError, Live, Parts, Retries};
use glave_ports::testing::{CountingEntropy, ManualClock};
use glave_ports::{InMemoryBlobs, InMemoryLog, ProviderError};

use crate::support::*;

fn unavailable() -> ProviderError {
    ProviderError::Http {
        status: 503,
        body: "overloaded".into(),
    }
}

async fn harness(
    dir: &Path,
    failures: Vec<ProviderError>,
    first_wait: Duration,
) -> Harness<ScriptedModel, InMemoryLog, InMemoryBlobs, AllowAll> {
    let mut settings = settings();
    settings.retries = Retries {
        attempts: 4,
        first_wait,
    };
    let parts = Parts {
        // Fails with each of `failures` in turn, then answers.
        provider: ScriptedModel::answering(
            failures
                .into_iter()
                .map(Err)
                .chain([Ok(answer(Some("Answered."), vec![], StopReason::EndTurn))])
                .collect(),
        ),
        log: InMemoryLog::new(),
        blobs: InMemoryBlobs::new(),
        approver: AllowAll,
        clock: Box::new(ManualClock::ticking(1_791_000_000_000)),
        entropy: Box::new(CountingEntropy::starting_at(1)),
        workspace: workspace(dir).await,
    };
    Harness::new(parts, settings)
}

fn retries(shown: &[Live]) -> Vec<(u32, u32)> {
    shown
        .iter()
        .filter_map(|live| match live {
            Live::Retrying { retry, of, .. } => Some((*retry, *of)),
            _ => None,
        })
        .collect()
}

#[test]
fn failures_that_may_pass_are_tried_again_and_shown() {
    block_on!(async {
        let failures = vec![
            unavailable(),
            ProviderError::Timeout { seconds: 300 },
            ProviderError::Transport("connection reset".into()),
        ];
        let dir = folder("retry");
        let harness = harness(&dir, failures, Duration::ZERO).await;
        let mut thread = harness.start_thread(scope()).await.unwrap();
        let mut shown = Vec::new();
        let summary = harness
            .run_turn(&mut thread, "hi", &Cancel::new(), &mut |live| {
                shown.push(live);
            })
            .await
            .unwrap();
        assert_eq!(summary.outcome, Outcome::Completed);
        assert_eq!(retries(&shown), [(1, 4), (2, 4), (3, 4)]);
        assert!(
            shown.iter().any(|live| matches!(
                live,
                Live::Retrying { error, .. } if error.contains("503")
            )),
            "the person sees why: {shown:?}"
        );
        assert_eq!(
            shown.iter().filter(|live| **live == Live::Waiting).count(),
            4,
            "each try says the model is being waited for"
        );
        let recorded = kinds(&events(harness.log(), thread.id).await);
        assert_eq!(
            recorded
                .iter()
                .filter(|kind| **kind == "model.responded")
                .count(),
            1,
            "failed tries leave nothing in the log"
        );
    });
}

fn empty_error() -> ProviderError {
    ProviderError::NoOutput("finish reason error".into())
}

#[test]
fn a_response_that_errors_before_any_output_is_tried_again() {
    block_on!(async {
        let dir = folder("retry");
        let harness = harness(&dir, vec![empty_error()], Duration::ZERO).await;
        let mut thread = harness.start_thread(scope()).await.unwrap();
        let mut shown = Vec::new();
        let summary = harness
            .run_turn(&mut thread, "hi", &Cancel::new(), &mut |live| {
                shown.push(live);
            })
            .await
            .unwrap();
        assert_eq!(summary.outcome, Outcome::Completed);
        assert_eq!(retries(&shown), [(1, 4)]);
    });
}

#[test]
fn a_response_that_errors_before_any_output_every_time_fails_after_the_limit() {
    block_on!(async {
        let dir = folder("retry");
        let harness = harness(&dir, vec![empty_error(); 5], Duration::ZERO).await;
        let mut thread = harness.start_thread(scope()).await.unwrap();
        let mut shown = Vec::new();
        let result = harness
            .run_turn(&mut thread, "hi", &Cancel::new(), &mut |live| {
                shown.push(live);
            })
            .await;
        assert!(matches!(result, Err(HarnessError::Provider(_))));
        assert_eq!(retries(&shown).len(), 4);
    });
}

#[test]
fn a_response_that_errors_after_output_is_not_tried_again() {
    let dir = folder("retry-broken");
    block_on!(async {
        let mut settings = settings();
        settings.retries = Retries {
            attempts: 4,
            first_wait: Duration::ZERO,
        };
        let parts = Parts {
            // Its answer streams some text and then ends in an error.
            provider: ScriptedModel::cycling(vec![answer(
                Some("Half an ans"),
                vec![],
                StopReason::Error,
            )]),
            log: InMemoryLog::new(),
            blobs: InMemoryBlobs::new(),
            approver: AllowAll,
            clock: Box::new(ManualClock::ticking(1_791_000_000_000)),
            entropy: Box::new(CountingEntropy::starting_at(1)),
            workspace: workspace(&dir).await,
        };
        let harness = Harness::new(parts, settings);
        let mut thread = harness.start_thread(scope()).await.unwrap();
        let mut shown = Vec::new();
        let summary = harness
            .run_turn(&mut thread, "hi", &Cancel::new(), &mut |live| {
                shown.push(live);
            })
            .await
            .unwrap();
        assert_eq!(summary.outcome, Outcome::Failed);
        assert_eq!(retries(&shown), []);
    });
}

#[test]
fn a_failure_that_will_not_pass_ends_the_turn_at_once() {
    block_on!(async {
        let refused = ProviderError::Http {
            status: 401,
            body: "bad key".into(),
        };
        let dir = folder("retry");
        let harness = harness(&dir, vec![refused.clone()], Duration::ZERO).await;
        let mut thread = harness.start_thread(scope()).await.unwrap();
        let mut shown = Vec::new();
        let error = harness
            .run_turn(&mut thread, "hi", &Cancel::new(), &mut |live| {
                shown.push(live);
            })
            .await
            .unwrap_err();
        assert!(matches!(error, HarnessError::Provider(e) if e == refused));
        assert_eq!(retries(&shown), []);
    });
}

#[test]
fn after_four_retries_the_turn_fails() {
    block_on!(async {
        let dir = folder("retry");
        let harness = harness(&dir, vec![unavailable(); 5], Duration::ZERO).await;
        let mut thread = harness.start_thread(scope()).await.unwrap();
        let mut shown = Vec::new();
        let result = harness
            .run_turn(&mut thread, "hi", &Cancel::new(), &mut |live| {
                shown.push(live);
            })
            .await;
        assert!(result.is_err());
        assert_eq!(retries(&shown).len(), 4);
        let recorded = kinds(&events(harness.log(), thread.id).await);
        assert_eq!(recorded.last(), Some(&"turn.ended"));
    });
}

#[test]
fn a_cancel_during_the_wait_ends_the_turn_cleanly() {
    block_on!(async {
        let dir = folder("retry");
        let harness = harness(&dir, vec![unavailable()], Duration::from_secs(600)).await;
        let mut thread = harness.start_thread(scope()).await.unwrap();
        let cancel = Cancel::new();
        let canceller = cancel.clone();
        let summary = harness
            .run_turn(&mut thread, "hi", &cancel, &mut |live| {
                if matches!(live, Live::Retrying { .. }) {
                    canceller.cancel();
                }
            })
            .await
            .unwrap();
        assert_eq!(summary.outcome, Outcome::Cancelled, "not after ten minutes");
    });
}
