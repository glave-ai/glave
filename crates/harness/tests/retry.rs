//! Retrying model calls against a model that fails on cue: failures that may pass are tried
//! again and the person sees each retry; failures that will not pass end the turn at once; and
//! a cancel during the wait between tries ends it cleanly.

#[cfg(test)]
#[path = "support/script.rs"]
#[expect(dead_code, reason = "shared helpers; this file uses some of them")]
mod support;

#[cfg(test)]
mod retrying {
    use std::collections::VecDeque;
    use std::future::{Future, ready};
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicI64, AtomicU64};
    use std::time::Duration;

    use glave_event::{Outcome, StopReason};
    use glave_harness::{AllowAll, Cancel, Harness, HarnessError, Live, Parts, Retries, Workspace};
    use glave_ports::{
        Completion, Delta, MemoryBlobs, MemoryLog, ModelProvider, ModelRequest, ProviderError,
    };

    use super::support::*;

    /// A model that fails with each of `failures` in turn, then answers.
    struct Flaky {
        failures: Mutex<VecDeque<ProviderError>>,
    }

    impl ModelProvider for Flaky {
        fn complete(
            &self,
            _request: &ModelRequest,
            _on_delta: &mut (dyn FnMut(Delta) + Send),
        ) -> impl Future<Output = Result<Completion, ProviderError>> + Send {
            ready(match self.failures.lock().unwrap().pop_front() {
                Some(failure) => Err(failure),
                None => Ok(answer(Some("Answered."), vec![], StopReason::EndTurn)),
            })
        }
    }

    fn unavailable() -> ProviderError {
        ProviderError::Http {
            status: 503,
            body: "overloaded".into(),
        }
    }

    async fn harness(
        failures: Vec<ProviderError>,
        first_wait: Duration,
    ) -> Harness<Flaky, MemoryLog, MemoryBlobs, AllowAll> {
        let mut settings = settings();
        settings.retries = Retries {
            attempts: 4,
            first_wait,
        };
        let parts = Parts {
            provider: Flaky {
                failures: Mutex::new(failures.into()),
            },
            log: MemoryLog::new(),
            blobs: MemoryBlobs::new(),
            approver: AllowAll,
            clock: Box::new(Ticking(AtomicI64::new(1_791_000_000_000))),
            entropy: Box::new(Counting(AtomicU64::new(1))),
            workspace: Workspace::open(&folder("retry"), std::env::vars_os())
                .await
                .unwrap(),
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
        block_on(async {
            let failures = vec![
                unavailable(),
                ProviderError::Timeout { seconds: 300 },
                ProviderError::Transport("connection reset".into()),
            ];
            let harness = harness(failures, Duration::ZERO).await;
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

    #[test]
    fn a_failure_that_will_not_pass_ends_the_turn_at_once() {
        block_on(async {
            let refused = ProviderError::Http {
                status: 401,
                body: "bad key".into(),
            };
            let harness = harness(vec![refused.clone()], Duration::ZERO).await;
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
        block_on(async {
            let harness = harness(vec![unavailable(); 5], Duration::ZERO).await;
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
        block_on(async {
            let harness = harness(vec![unavailable()], Duration::from_secs(600)).await;
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
}
