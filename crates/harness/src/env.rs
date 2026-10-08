//! Time and randomness, injected so every run can be reproduced in tests. The real clock and
//! the operating system's randomness live in the binary.

use jiff::Timestamp;

/// The current time.
pub trait Clock: Send + Sync {
    fn now(&self) -> Timestamp;
}

/// Random bytes for the random part of new ids.
pub trait Entropy: Send + Sync {
    fn bytes(&self) -> [u8; 10];
}
