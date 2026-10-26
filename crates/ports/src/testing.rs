//! The test doubles every crate's tests share (AGENTS.md, "Testing"): the runtime they run
//! on, a clock they move, randomness that counts, a cancellation already fired, a fake HTTP
//! server and a client that stalls mid-request, the core of the model providers' fake servers,
//! a fake OAuth authorization server, fake
//! secrets, a scripted model, programs they start, stopped when they end, and folders of their
//! own, removed when they are done. Behind the `conformance` feature, so no production build
//! carries them.

mod git;
mod http;
mod model;
mod oauth;
mod process;
mod scripted;
mod secret;
mod temp;

use std::sync::Arc;
use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};

use jiff::Timestamp;

use crate::env::{Clock, Entropy, EntropyError};
use crate::tools::{BoxFuture, Cancellation};

pub use git::{FAKE_COMMIT, FakeGitHost, FixedGitCredentials};
pub use http::{FakeHttp, Received, sent_in_part};
pub use model::{Answer, ScriptedModel, Seen};
pub use oauth::{FAKE_CLIENT, FakeAuthServer, Grant};
pub use process::{HUNG, Running};
pub use scripted::{ScriptedHttp, Written};
pub use secret::{FAKE_SECRET, fake_secret};
pub use temp::TempDir;

/// Runs a future to its end on a fresh current-thread runtime with every driver enabled.
///
/// A macro rather than a function: starting a runtime can fail, and the panic that follows
/// belongs in the test that asked, where the test exemptions allow it.
#[macro_export]
macro_rules! block_on {
    ($future:expr $(,)?) => {
        $crate::testing::runtime().unwrap().block_on($future)
    };
}

/// The runtime [`block_on!`](crate::block_on) runs on.
pub fn runtime() -> std::io::Result<tokio::runtime::Runtime> {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
}

/// A clock a test controls. A still clock reads the same until the test moves it; a ticking
/// one reads a millisecond later each time, so every event gets an instant of its own. Clones
/// share one time.
#[derive(Debug, Clone)]
pub struct ManualClock {
    millis: Arc<AtomicI64>,
    tick: i64,
}

impl ManualClock {
    /// A still clock at `millis` milliseconds since 1970.
    pub fn at_millis(millis: i64) -> Self {
        Self {
            millis: Arc::new(AtomicI64::new(millis)),
            tick: 0,
        }
    }

    /// A still clock at `second` seconds since 1970.
    pub fn at_second(second: i64) -> Self {
        Self::at_millis(second.saturating_mul(1_000))
    }

    /// A clock that first reads `millis` and a millisecond more at each reading after.
    pub fn ticking(millis: i64) -> Self {
        Self {
            tick: 1,
            ..Self::at_millis(millis)
        }
    }

    /// What the clock reads next, in milliseconds since 1970, without reading it.
    pub fn millis(&self) -> i64 {
        self.millis.load(Ordering::SeqCst)
    }

    /// Sets the clock to `millis` milliseconds since 1970.
    pub fn set_millis(&self, millis: i64) {
        self.millis.store(millis, Ordering::SeqCst);
    }

    /// Moves the clock on by `millis`.
    pub fn advance_millis(&self, millis: i64) {
        self.millis.fetch_add(millis, Ordering::SeqCst);
    }

    /// Moves the clock on by `seconds`.
    pub fn advance_seconds(&self, seconds: i64) {
        self.advance_millis(seconds.saturating_mul(1_000));
    }
}

impl Clock for ManualClock {
    /// A clock moved past the times `jiff` can hold stays at its last one.
    fn now(&self) -> Timestamp {
        let millis = self.millis.fetch_add(self.tick, Ordering::SeqCst);
        Timestamp::from_millisecond(millis).unwrap_or(Timestamp::MAX)
    }
}

/// Randomness that counts: each draw is the next number, in the first eight bytes, so ids
/// come out in order and every run makes the same ones.
#[derive(Debug)]
pub struct CountingEntropy(AtomicU64);

impl CountingEntropy {
    /// Randomness whose first draw is `start`.
    pub fn starting_at(start: u64) -> Self {
        Self(AtomicU64::new(start))
    }
}

impl Entropy for CountingEntropy {
    fn bytes(&self) -> Result<[u8; 10], EntropyError> {
        let mut bytes = [0; 10];
        let n = self.0.fetch_add(1, Ordering::SeqCst).to_be_bytes();
        for (slot, byte) in bytes.iter_mut().zip(n) {
            *slot = byte;
        }
        Ok(bytes)
    }
}

/// A random source that always fails, as a broken operating system's would.
#[derive(Debug)]
pub struct FailingEntropy;

impl Entropy for FailingEntropy {
    fn bytes(&self) -> Result<[u8; 10], EntropyError> {
        Err(EntropyError("the test's source is broken".into()))
    }
}

/// A cancellation that has already fired.
#[derive(Debug)]
pub struct AlreadyCancelled;

impl Cancellation for AlreadyCancelled {
    fn is_cancelled(&self) -> bool {
        true
    }

    fn cancelled(&self) -> BoxFuture<'_, ()> {
        Box::pin(std::future::ready(()))
    }
}
