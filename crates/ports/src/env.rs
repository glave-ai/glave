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

/// The operating system's random source.
pub struct OsEntropy;

impl Entropy for OsEntropy {
    fn bytes(&self) -> [u8; 10] {
        let mut bytes = [0u8; 10];
        match getrandom::fill(&mut bytes) {
            Ok(()) => bytes,
            Err(_) => fallback(),
        }
    }
}

/// Bytes for an id when the operating system's random source fails, which it does not in
/// practice: the time, mixed by a hasher std keys anew for every call, so no two calls give
/// the same bytes. Good enough for the random part of ids, which only need to differ; never
/// used for secrets (nonces and keys read the OS source themselves and fail without it).
fn fallback() -> [u8; 10] {
    use std::hash::{BuildHasher as _, Hasher as _};
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_nanos())
        .unwrap_or_default();
    let mut bytes = [0u8; 10];
    let mut words = [0u64; 2];
    for (index, word) in words.iter_mut().enumerate() {
        let mut hasher = std::collections::hash_map::RandomState::new().build_hasher();
        hasher.write_u128(nanos);
        hasher.write_usize(index);
        *word = hasher.finish();
    }
    for (byte, source) in bytes
        .iter_mut()
        .zip(words.iter().flat_map(|word| word.to_le_bytes()))
    {
        *byte = source;
    }
    bytes
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_fallback_never_repeats() {
        let seen: std::collections::HashSet<[u8; 10]> = (0..1000).map(|_| fallback()).collect();
        assert_eq!(seen.len(), 1000);
    }

    #[test]
    fn os_entropy_gives_new_bytes_each_time() {
        assert_ne!(OsEntropy.bytes(), OsEntropy.bytes());
    }
}
