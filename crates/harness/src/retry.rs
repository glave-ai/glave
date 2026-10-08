//! Trying a model call again after a failure that may pass: a rate limit, a server error, a
//! broken connection or a silence. Codex retries the same way: up to four more times, waiting
//! longer each time.

use std::time::Duration;

/// The longest wait between tries.
const LONGEST_WAIT: Duration = Duration::from_secs(60);

/// How many times, and how patiently, to try a failed model call again.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Retries {
    /// Tries after the first.
    pub attempts: u32,
    /// The wait before the first retry; each later wait doubles.
    pub first_wait: Duration,
}

impl Default for Retries {
    fn default() -> Self {
        Self {
            attempts: 4,
            first_wait: Duration::from_secs(1),
        }
    }
}

impl Retries {
    /// The wait before retry `retry` (from 1): the first wait doubled each time, give or take
    /// a tenth so that many clients do not retry in step, and never over a minute.
    pub(crate) fn wait(&self, retry: u32, randomness: [u8; 10]) -> Duration {
        let doubled = self
            .first_wait
            .saturating_mul(2_u32.saturating_pow(retry.saturating_sub(1)));
        let spread = f64::from(randomness.first().copied().unwrap_or(128)) / 255.0;
        doubled.mul_f64(0.9 + 0.2 * spread).min(LONGEST_WAIT)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn waits_double_within_a_tenth_and_stop_at_a_minute() {
        let retries = Retries::default();
        let low = [0; 10];
        let high = [255; 10];
        for (retry, seconds) in [(1, 1.0), (2, 2.0), (3, 4.0), (4, 8.0)] {
            let shortest = retries.wait(retry, low).as_secs_f64();
            let longest = retries.wait(retry, high).as_secs_f64();
            assert!(
                (shortest - seconds * 0.9).abs() < 1e-9,
                "{retry}: {shortest}"
            );
            assert!((longest - seconds * 1.1).abs() < 1e-9, "{retry}: {longest}");
        }
        assert_eq!(retries.wait(30, high), LONGEST_WAIT);
        let instant = Retries {
            attempts: 4,
            first_wait: Duration::ZERO,
        };
        assert_eq!(instant.wait(3, high), Duration::ZERO);
    }
}
