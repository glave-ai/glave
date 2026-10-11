//! Pages fetched lately, by URL: a few, for a short while, so a model that reads the same page
//! twice in a turn does not fetch it twice. Times are passed in, so tests need no clock.

use std::sync::{Mutex, PoisonError};
use std::time::{Duration, Instant};

/// The most pages kept.
pub(crate) const MOST: usize = 32;
/// How long a page is kept, as Claude Code keeps fetched pages.
pub(crate) const FRESH: Duration = Duration::from_mins(15);

/// Recent pages, oldest first. Never more than `MOST`.
pub(crate) struct Cache<T> {
    entries: Mutex<Vec<(String, Instant, T)>>,
}

impl<T: Clone> Cache<T> {
    pub(crate) fn new() -> Self {
        Self {
            entries: Mutex::new(Vec::new()),
        }
    }

    /// The page kept for `key`, if it is still fresh at `now`.
    pub(crate) fn get(&self, key: &str, now: Instant) -> Option<T> {
        let mut entries = self.entries.lock().unwrap_or_else(PoisonError::into_inner);
        entries.retain(|(_, at, _)| now.saturating_duration_since(*at) < FRESH);
        entries
            .iter()
            .find(|(k, _, _)| k == key)
            .map(|(_, _, value)| value.clone())
    }

    /// Keeps `value` for `key` from `now`, dropping the oldest page when full.
    pub(crate) fn put(&self, key: String, value: T, now: Instant) {
        let mut entries = self.entries.lock().unwrap_or_else(PoisonError::into_inner);
        entries.retain(|(k, _, _)| *k != key);
        if entries.len() >= MOST {
            entries.remove(0);
        }
        entries.push((key, now, value));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pages_are_kept_briefly_and_few() {
        let cache = Cache::new();
        let start = Instant::now();
        cache.put("a".into(), 1, start);
        assert_eq!(cache.get("a", start + Duration::from_secs(60)), Some(1));
        assert_eq!(cache.get("a", start + FRESH), None);
        for n in 0..=MOST {
            cache.put(format!("k{n}"), n, start);
        }
        assert_eq!(cache.get("k0", start), None, "the oldest went");
        assert_eq!(cache.get(&format!("k{MOST}"), start), Some(MOST));
        assert_eq!(cache.entries.lock().unwrap().len(), MOST);
    }
}
