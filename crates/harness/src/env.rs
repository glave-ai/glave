//! Time and randomness, injected so every run can be reproduced in tests. Binaries wire in the
//! real clock and the operating system's randomness; tests wire in their own.

use jiff::Timestamp;

/// The current time.
pub trait Clock: Send + Sync {
    fn now(&self) -> Timestamp;
}

/// Random bytes for the random part of new ids.
pub trait Entropy: Send + Sync {
    fn bytes(&self) -> [u8; 10];
}

/// The computer's clock.
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> Timestamp {
        Timestamp::now()
    }
}

/// The operating system's random source.
pub struct OsEntropy;

impl Entropy for OsEntropy {
    fn bytes(&self) -> [u8; 10] {
        let mut bytes = [0u8; 10];
        // The OS random source does not fail in practice; if it ever did, ids would still be
        // unique within a millisecond only by their sequence, which the log enforces.
        let _ = getrandom::fill(&mut bytes);
        bytes
    }
}
