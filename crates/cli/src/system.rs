//! The real clock and the operating system's randomness. Only the binary uses them; everything
//! else has them injected.

use glave_harness::{Clock, Entropy};
use jiff::Timestamp;

pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> Timestamp {
        Timestamp::now()
    }
}

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

/// A random UUID v4, for this run's local organization and space.
pub fn random_uuid() -> uuid::Uuid {
    let mut bytes = [0u8; 16];
    let _ = getrandom::fill(&mut bytes);
    uuid::Builder::from_random_bytes(bytes).into_uuid()
}
