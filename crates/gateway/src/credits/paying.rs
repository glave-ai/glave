//! One call paying from its organization's credits: its worst case held before it is sent,
//! then what it cost counted in the hold's place. A call that ends without an answer after
//! its answer began to come, broken off or cancelled, is counted at what it read and what came
//! of it, at its price, since its provider charges for that too; one that ends before any of
//! it came is counted at nothing.

use std::sync::atomic::{AtomicU64, Ordering};

use glave_event::{Money, TokenCounts};
use glave_ports::{Delta, Payer, ProviderError};

use super::Charging;
use super::book::micros;
use crate::price::Price;

/// Bytes of output to a token, as a charge for a call cut short counts what came of it.
const BYTES_PER_TOKEN: u64 = 4;

/// A call's worst case held against its organization's credits until the call ends. Dropped
/// before it settles (the call failed or was cancelled), it lets go of the hold and counts what
/// the call cost so far.
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

    /// Counts what the call cost, `cost` when its provider or its price says, in the hold's
    /// place, and writes it to the store.
    pub(crate) async fn settle(self, cost: Option<Money>) {
        let cost = cost.and_then(micros).unwrap_or(0);
        self.counted(cost).await;
    }

    /// Counts what a call that ended without an answer cost so far, in the hold's place, and
    /// writes it to the store.
    pub(crate) async fn broke(self) {
        let cost = self.so_far();
        self.counted(cost).await;
    }

    async fn counted(mut self, cost: i64) {
        self.settled = true;
        self.credits.settle(self.payer, self.held, cost);
        self.credits.write(self.payer.org).await;
    }

    /// What the call cost so far: nothing before its answer began to come; after, what it read
    /// and what came, at its price.
    fn so_far(&self) -> i64 {
        let seen = self.seen.load(Ordering::Relaxed);
        if seen == 0 {
            return 0;
        }
        let counts = TokenCounts {
            input: self.input,
            output: seen.div_ceil(BYTES_PER_TOKEN),
            ..TokenCounts::default()
        };
        i64::try_from(self.price.micros(&counts)).unwrap_or(i64::MAX)
    }
}

impl Drop for Paying<'_> {
    fn drop(&mut self) {
        if !self.settled {
            self.credits.settle(self.payer, self.held, self.so_far());
        }
    }
}
