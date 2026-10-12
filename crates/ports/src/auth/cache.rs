//! Credentials kept until shortly before they expire, then fetched again: a bearer token from
//! an identity provider, or a cloud's short-lived keys.

use std::future::Future;
use std::sync::{Arc, Mutex, PoisonError};

use jiff::{SignedDuration, Timestamp};

use crate::auth::AuthError;
use crate::env::Clock;

/// Credentials and when they expire.
#[derive(Debug, Clone)]
pub struct Fresh<T> {
    pub value: T,
    pub expires: Timestamp,
}

/// The last credentials fetched, fetched again once they are within `early` of expiring.
///
/// The lock is held only to read or replace them, never while fetching. Two requests that
/// find them stale at once both fetch, and the later answer is kept: harmless, and simpler
/// than making one wait on the other.
pub struct Refreshing<T> {
    slot: Mutex<Option<Fresh<T>>>,
    clock: Arc<dyn Clock>,
    early: SignedDuration,
}

impl<T: Clone> Refreshing<T> {
    /// Credentials read by `clock`, refreshed `early` before they expire. Azure's and AWS's
    /// own libraries refresh five minutes early.
    pub fn new(clock: Arc<dyn Clock>, early: SignedDuration) -> Self {
        Self {
            slot: Mutex::new(None),
            clock,
            early,
        }
    }

    /// The credentials kept, while they are fresh; otherwise those `fetch` gets, kept for
    /// next time.
    pub async fn get<F, Fut>(&self, fetch: F) -> Result<T, AuthError>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = Result<Fresh<T>, AuthError>>,
    {
        let now = self.clock.now();
        if let Some(kept) = self.kept(now) {
            return Ok(kept);
        }
        let fresh = fetch().await?;
        let value = fresh.value.clone();
        *self.slot.lock().unwrap_or_else(PoisonError::into_inner) = Some(fresh);
        Ok(value)
    }

    /// The credentials kept, if they do not expire within `early` of `now`.
    fn kept(&self, now: Timestamp) -> Option<T> {
        let slot = self.slot.lock().unwrap_or_else(PoisonError::into_inner);
        slot.as_ref()
            .filter(|fresh| {
                now.saturating_add(self.early)
                    .is_ok_and(|soon| soon < fresh.expires)
            })
            .map(|fresh| fresh.value.clone())
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicI64, AtomicU32, Ordering};

    use super::*;

    struct Ticking(AtomicI64);

    impl Clock for Ticking {
        fn now(&self) -> Timestamp {
            Timestamp::from_second(self.0.load(Ordering::SeqCst)).unwrap()
        }
    }

    fn block_on<F: Future>(future: F) -> F::Output {
        tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap()
            .block_on(future)
    }

    #[test]
    fn credentials_are_kept_until_shortly_before_they_expire() {
        let clock = Arc::new(Ticking(AtomicI64::new(1_000)));
        let cache = Refreshing::new(clock.clone(), SignedDuration::from_secs(300));
        let fetches = AtomicU32::new(0);
        let get = || {
            block_on(cache.get(|| async {
                let n = fetches.fetch_add(1, Ordering::SeqCst) + 1;
                Ok(Fresh {
                    value: format!("token-{n}"),
                    expires: Timestamp::from_second(1_000 + 3_600).unwrap(),
                })
            }))
            .unwrap()
        };
        assert_eq!(get(), "token-1");
        clock.0.store(1_000 + 3_000, Ordering::SeqCst);
        assert_eq!(get(), "token-1", "still more than five minutes left");
        clock.0.store(1_000 + 3_301, Ordering::SeqCst);
        assert_eq!(get(), "token-2", "within five minutes of expiring");
    }

    #[test]
    fn a_failed_fetch_keeps_nothing_and_says_why() {
        let clock = Arc::new(Ticking(AtomicI64::new(0)));
        let cache: Refreshing<String> = Refreshing::new(clock, SignedDuration::from_secs(300));
        let error = block_on(cache.get(|| async { Err(AuthError::unreachable("down")) }));
        assert_eq!(error.unwrap_err().to_string(), "down");
        let kept = block_on(cache.get(|| async {
            Ok(Fresh {
                value: "t".to_owned(),
                expires: Timestamp::MAX,
            })
        }));
        assert_eq!(kept.unwrap(), "t");
    }
}
