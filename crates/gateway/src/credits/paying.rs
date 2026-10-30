//! One call paying from its organization's credits: its worst case held before it is sent,
//! then what it cost counted in the hold's place. A call cancelled once sent is counted at what
//! it read and what came of its answer's text, at its price, as its provider charges for them;
//! one that broke off after its answer began, the same; one that failed before any came, at
//! nothing. What came of a call to a tool is not counted until the answer ends: a gap.

use std::sync::atomic::{AtomicU64, Ordering};

use glave_event::{Money, TokenCounts};
use glave_ports::{Delta, Payer, ProviderError};

use super::Charging;
use super::book::micros;
use crate::estimate::FIT_BYTES_PER_TOKEN;
use crate::price::Price;

/// A call's worst case held against its organization's credits until the call ends. Dropped
/// before it settles (the call was cancelled), it lets go of the hold and counts what the call
/// cost so far, written with the organization's next call.
pub(crate) struct Paying<'a> {
    credits: &'a dyn Charging,
    payer: Payer,
    held: i64,
    price: &'a Price,
    /// The tokens the call reads, as estimated before it was sent.
    input: u64,
    /// Bytes of answer that came so far.
    seen: AtomicU64,
    settled: bool,
}

impl<'a> Paying<'a> {
    /// Holds `worst` of `payer`'s organization's credits for a call at `price` that reads
    /// `input` tokens, or refuses it.
    pub(crate) async fn hold(
        credits: &'a dyn Charging,
        payer: Payer,
        worst: Money,
        price: &'a Price,
        input: u64,
    ) -> Result<Self, ProviderError> {
        let held = micros(worst).unwrap_or(0).max(0);
        credits.hold(payer.org, held).await?;
        Ok(Self {
            credits,
            payer,
            held,
            price,
            input,
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
        let cost = cost.and_then(micros).unwrap_or_else(|| self.priced(counts));
        self.counted(cost).await;
    }

    /// Counts what a call that ended without an answer cost so far, in the hold's place, and
    /// writes it to the store: nothing when none of its answer came.
    pub(crate) async fn broke(self) {
        let cost = if self.seen.load(Ordering::Relaxed) == 0 {
            0
        } else {
            self.so_far()
        };
        self.counted(cost).await;
    }

    async fn counted(mut self, cost: i64) {
        self.settled = true;
        self.credits.settle(self.payer, self.held, cost);
        self.credits.write(self.payer.org).await;
    }

    /// What the call cost so far: what it read, and what of its answer came, at its price.
    fn so_far(&self) -> i64 {
        let counts = TokenCounts {
            input: self.input,
            output: self
                .seen
                .load(Ordering::Relaxed)
                .div_ceil(FIT_BYTES_PER_TOKEN),
            ..TokenCounts::default()
        };
        self.priced(&counts)
    }

    /// `counts` at the call's price, in millionths of a dollar.
    fn priced(&self, counts: &TokenCounts) -> i64 {
        i64::try_from(self.price.micros(counts)).unwrap_or(i64::MAX)
    }
}

impl Drop for Paying<'_> {
    fn drop(&mut self) {
        if !self.settled {
            self.credits.settle(self.payer, self.held, self.so_far());
        }
    }
}
