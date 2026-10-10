//! A model as the judge: the deterministic checks first, then, for each soft caution, the
//! harness's own judge on a real model. Cases run several at once; each decision is timed and
//! its cost added up.

use std::sync::Arc;
use std::sync::atomic::{AtomicI64, Ordering};
use std::time::{Duration, Instant};

use glave_harness::{JudgeCase, Ruling, judge};
use glave_ports::{Completion, Delta, ModelProvider, ModelRequest, ProviderError};
use tokio::sync::Semaphore;
use tokio::task::JoinSet;

use super::baseline::{Assessed, assess};
use super::case::{Case, Label};

/// A provider that adds up what its calls cost, as the provider reports it.
pub(crate) struct Metered<P> {
    inner: P,
    micros: AtomicI64,
}

impl<P> Metered<P> {
    pub(crate) fn new(inner: P) -> Self {
        Self {
            inner,
            micros: AtomicI64::new(0),
        }
    }

    /// The total cost so far, in millionths of a dollar.
    pub(crate) fn micros(&self) -> i64 {
        self.micros.load(Ordering::Relaxed)
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
            self.micros.fetch_add(cost.amount.micros, Ordering::Relaxed);
        }
        Ok(completion)
    }
}

/// Each case's decision with `model` judging soft cautions, and how long it took, in case
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
        let Assessed::Soft { reason } = assess(&case.action) else {
            continue;
        };
        let (provider, gate, model) = (Arc::clone(&provider), Arc::clone(&gate), model.to_owned());
        let (asked, action) = (case.request.clone(), case.action.summary());
        tasks.spawn(async move {
            let _permit = gate.acquire().await;
            let started = Instant::now();
            let ruling = judge(
                &*provider,
                &model,
                &JudgeCase {
                    asked: &asked,
                    rules: None,
                    action: &action,
                    flagged: &reason,
                },
            )
            .await;
            let label = match ruling {
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
            Assessed::Asks | Assessed::Soft { .. } => (Label::Ask, Duration::ZERO),
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
