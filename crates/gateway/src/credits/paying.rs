//! One call paying from its organization's credits: its worst case held before it is sent,
//! then what it cost counted in the hold's place. A call cancelled once sent is counted at what
//! it read and what came of its answer's text, at its price, as its provider charges for them;
//! one that broke off after its answer began, the same; one that failed before any came, at
//! nothing. What came of a call to a tool is not counted until the answer ends: a gap. A media
//! call is billed whole whether or not its answer is read, so one dropped once sent, or ended
//! with no answer (silence past the provider's timeout, a broken connection, an answer that
//! can't be read), is counted at its hold; one its provider refused with an error, at nothing,
//! as no failed generation is billed.

use std::sync::atomic::{AtomicU64, Ordering};

use glave_event::{CostKind, EventId, Money, Nanos, TokenCounts};
use glave_ports::{Delta, Payer, ProviderError};

use super::Charging;
use super::book::in_dollars;
use crate::estimate::FIT_BYTES_PER_TOKEN;
use crate::price::Price;

/// A call's worst case held against its organization's credits until the call ends. Dropped
/// before it settles (the call was cancelled), it lets go of the hold and counts what the call
/// cost so far, written with the organization's next call.
pub(crate) struct Paying<'a> {
    credits: &'a dyn Charging,
    payer: Payer,
    /// The hold in the ledger, when anything was held.
    hold: Option<EventId>,
    price: &'a Price,
    /// The tokens the call reads, as estimated before it was sent.
    input: u64,
    /// What was held: what a media call that ends unanswered is counted at.
    worst: Nanos,
    /// What the call is, as its charge names it.
    kind: CostKind,
    /// Bytes of answer that came so far.
    seen: AtomicU64,
    settled: bool,
}

impl<'a> Paying<'a> {
    /// Holds `worst` of `payer`'s organization's credits for a call of `kind` at `price` that
    /// reads `input` tokens, or refuses it.
    pub(crate) async fn hold(
        credits: &'a dyn Charging,
        payer: Payer,
        worst: Money,
        (price, input): (&'a Price, u64),
        kind: CostKind,
    ) -> Result<Self, ProviderError> {
        let worst = in_dollars(worst).unwrap_or(Nanos::ZERO).max(Nanos::ZERO);
        let hold = credits.hold(payer.org, worst).await?;
        Ok(Self {
            credits,
            payer,
            hold,
            price,
            input,
            worst,
            kind,
            seen: AtomicU64::new(0),
            settled: false,
        })
    }

    /// Takes note of `delta`, part of the answer as it comes.
    pub(crate) fn saw(&self, delta: &Delta) {
        let (Delta::Text(text) | Delta::Reasoning(text)) = delta;
        let bytes = u64::try_from(text.len()).unwrap_or(u64::MAX);
        self.seen.fetch_add(bytes, Ordering::Relaxed);
    }

    /// Counts what the call cost, `cost` when its provider says it in dollars, else `counts`
    /// at its price, in the hold's place, and writes it to the store.
    pub(crate) async fn settle(self, cost: Option<Money>, counts: &TokenCounts) {
        let cost = cost
            .and_then(in_dollars)
            .unwrap_or_else(|| self.price.nanos(counts));
        self.counted(cost).await;
    }

    /// Counts what a call that ended in `error` without an answer cost so far, in the hold's
    /// place, and writes it to the store: nothing when none of its answer came; a media call,
    /// its hold unless its provider answered with a refusal or it was never sent.
    pub(crate) async fn broke(self, error: &ProviderError) {
        let cost = if self.kind == CostKind::Media {
            if unbilled(error) {
                Nanos::ZERO
            } else {
                self.worst
            }
        } else if self.seen.load(Ordering::Relaxed) == 0 {
            Nanos::ZERO
        } else {
            self.so_far()
        };
        self.counted(cost).await;
    }

    async fn counted(mut self, cost: Nanos) {
        self.settled = true;
        self.credits
            .settle(self.payer, self.hold, (cost, self.kind.clone()));
        self.credits.write(self.payer.org).await;
    }

    /// What the call cost so far: what it read, and what of its answer came, at its price.
    fn so_far(&self) -> Nanos {
        let counts = TokenCounts {
            input: self.input,
            output: self
                .seen
                .load(Ordering::Relaxed)
                .div_ceil(FIT_BYTES_PER_TOKEN),
            ..TokenCounts::default()
        };
        self.price.nanos(&counts)
    }
}

impl Drop for Paying<'_> {
    fn drop(&mut self) {
        if !self.settled {
            let cost = if self.kind == CostKind::Media {
                self.worst
            } else {
                self.so_far()
            };
            self.credits
                .settle(self.payer, self.hold, (cost, self.kind.clone()));
        }
    }
}

/// Whether a media call that ended in `error` was never billed: its provider answered with a
/// refusal, which bills no failed generation, or it was never sent.
fn unbilled(error: &ProviderError) -> bool {
    matches!(
        error,
        ProviderError::Http { .. }
            | ProviderError::Credentials(_)
            | ProviderError::Unserved { .. }
            | ProviderError::Unpaid(_)
    )
}
