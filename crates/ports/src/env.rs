//! Time and randomness, injected so every run can be reproduced in tests. Binaries wire in the
//! real clock and the operating system's randomness; tests wire in their own.

use std::sync::Arc;

use jiff::Timestamp;

/// The current time.
pub trait Clock: Send + Sync {
    fn now(&self) -> Timestamp;
}

/// Random bytes for the random part of new ids.
pub trait Entropy: Send + Sync {
    fn bytes(&self) -> [u8; 10];
}

impl<T: Clock + ?Sized> Clock for Arc<T> {
    fn now(&self) -> Timestamp {
        (**self).now()
    }
}

impl<T: Entropy + ?Sized> Entropy for Arc<T> {
    fn bytes(&self) -> [u8; 10] {
        (**self).bytes()
    }
}

/// The computer's clock.
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> Timestamp {
        Timestamp::now()
    }
}

/// The operating system's random source. Where it fails, which it does not on any system glave
/// runs on, the bytes stay zero: an id then differs only by its time, and sealing, which reads
/// the same source, fails loudly.
pub struct OsEntropy;

impl Entropy for OsEntropy {
    fn bytes(&self) -> [u8; 10] {
        let mut bytes = [0u8; 10];
        let _ = getrandom::fill(&mut bytes);
        bytes
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn os_entropy_gives_new_bytes_each_time() {
        assert_ne!(OsEntropy.bytes(), OsEntropy.bytes());
    }
}
