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
    /// Ten random bytes, or why there are none: an id is never made weaker than it should be.
    fn bytes(&self) -> Result<[u8; 10], EntropyError>;
}

/// The random source gave no bytes, so nothing that needed them was made.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("this computer's random source failed ({0}), so nothing was made; try again")]
pub struct EntropyError(pub String);

impl<T: Clock + ?Sized> Clock for Arc<T> {
    fn now(&self) -> Timestamp {
        (**self).now()
    }
}

impl<T: Entropy + ?Sized> Entropy for Arc<T> {
    fn bytes(&self) -> Result<[u8; 10], EntropyError> {
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
/// runs on, it says so, as sealing, which reads the same source, does.
pub struct OsEntropy;

impl Entropy for OsEntropy {
    fn bytes(&self) -> Result<[u8; 10], EntropyError> {
        let mut bytes = [0u8; 10];
        getrandom::fill(&mut bytes).map_err(|error| EntropyError(error.to_string()))?;
        Ok(bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn os_entropy_gives_new_bytes_each_time() {
        assert_ne!(OsEntropy.bytes().unwrap(), OsEntropy.bytes().unwrap());
    }
}
