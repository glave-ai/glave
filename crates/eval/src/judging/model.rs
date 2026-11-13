//! A model as the judge: the deterministic checks first, then, for each caution, the
//! harness's own judge on a real model. Cases run several at once; each decision is timed and
//! its cost added up.

use std::sync::Arc;
use std::sync::atomic::{AtomicI64, Ordering};
use std::time::{Duration, Instant};

use glave_event::Nanos;
use glave_harness::{JudgeCase, Ruling, judge};
use glave_ports::{Completion, Delta, ModelProvider, ModelRequest, ProviderError};
use tokio::sync::Semaphore;
use tokio::task::JoinSet;

use super::baseline::{Assessed, assess};
use super::case::{Action, Case, Label};

/// A provider that adds up what its calls cost, as the provider reports it.
pub(crate) struct Metered<P> {
    inner: P,
    spent: AtomicI64,
}

impl<P> Metered<P> {
    pub(crate) fn new(inner: P) -> Self {
        Self {
            inner,
            spent: AtomicI64::new(0),
        }
    }

    /// The total cost so far, in dollars.
    pub(crate) fn spent(&self) -> Nanos {
        Nanos::new(self.spent.load(Ordering::Relaxed))
    }
}

impl<P: ModelProvider> ModelProvider for Metered<P> {
    async fn complete(
        &self,
        request: &ModelRequest,
        on_delta: &mut (dyn FnMut(Delta) + Send),
    ) -> Result<Completion, ProviderError> {
        let completion = self.inner.complete(request, on_delta).await?;
        if let Some(cost) = &completion.cost {
            let nanos = cost.amount.nanos.get();
            self.spent
                .update(Ordering::Relaxed, Ordering::Relaxed, |sum| {
                    sum.saturating_add(nanos)
                });
        }
        Ok(completion)
    }
}

/// Each case's decision with `model` judging cautions, and how long it took, in case
/// order.
pub(crate) async fn decide_all<P: ModelProvider + 'static>(
    cases: &[Case],
    provider: Arc<P>,
    model: &str,
    at_once: usize,
) -> Vec<(Label, Duration)> {
    let gate = Arc::new(Semaphore::new(at_once.max(1)));
    let mut tasks = JoinSet::new();
    for (index, case) in cases.iter().enumerate() {
        let Assessed::Caution { reason, secret } = assess(&case.action) else {
            continue;
        };
        let (provider, gate, model) = (Arc::clone(&provider), Arc::clone(&gate), model.to_owned());
        let (asked, action) = (case.request.clone(), case.action.summary());
        let after =
            |action: &Action| matches!(action, Action::Remember { .. } | Action::Connector { .. });
        let untrusted = case.untrusted || after(&case.action);
        tasks.spawn(async move {
            let _permit = gate.acquire().await;
            let started = Instant::now();
            let ruling = judge(
                &*provider,
                &model,
                None,
                &JudgeCase {
                    asked: &asked,
                    rules: None,
                    action: &action,
                    flagged: &reason,
                    untrusted,
                    outside_sandbox: None,
                    secret,
                },
            )
            .await;
            let label = match ruling.0 {
                Ruling::Run { .. } => Label::Allow,
                Ruling::Ask { .. } => Label::Ask,
            };
            (index, label, started.elapsed())
        });
    }
    let mut decided: Vec<(Label, Duration)> = cases
        .iter()
        .map(|case| match assess(&case.action) {
            Assessed::Runs => (Label::Allow, Duration::ZERO),
            Assessed::Caution { .. } => (Label::Ask, Duration::ZERO),
        })
        .collect();
    while let Some(joined) = tasks.join_next().await {
        if let Ok((index, label, took)) = joined
            && let Some(slot) = decided.get_mut(index)
        {
            *slot = (label, took);
        }
    }
    decided
}
