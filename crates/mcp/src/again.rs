//! When a server that failed to start is started again (#1199): at the first ask a while after
//! its failure was seen, the while doubling with each failure from [`FIRST`] to [`MOST`], and
//! in the background, so no turn waits for it. A server that is truly broken costs a process
//! now and then, never a turn's time; one that stalled for a moment is back at a later turn.

use std::sync::Arc;
use std::time::Duration;

use glave_ports::{Clock, SystemClock};

/// How long after its first failure is seen a server is started again.
const FIRST: Duration = Duration::from_secs(10);

/// The longest a server that keeps failing waits between starts.
const MOST: Duration = Duration::from_secs(600);

/// How a server's failed starts stand.
pub(crate) struct Again {
    clock: Arc<dyn Clock>,
    /// The failed starts seen so far.
    failures: u32,
    /// When the last failed start was first seen, in milliseconds since 1970: none while a
    /// start is under way or the server is ready.
    seen: Option<i64>,
    /// Whether the server was closed, after which it never starts again.
    closed: bool,
}

impl Default for Again {
    fn default() -> Self {
        Self::new(Arc::new(SystemClock))
    }
}

impl Again {
    /// None failed yet, timed by `clock`.
    pub(crate) fn new(clock: Arc<dyn Clock>) -> Self {
        Self {
            clock,
            failures: 0,
            seen: None,
            closed: false,
        }
    }

    /// Whether a start under way is one tried again, which nothing waits for; the first is.
    pub(crate) fn retrying(&self) -> bool {
        self.failures > 0
    }

    /// Sees that the last start failed, and says whether to start again now: not the first
    /// time it is seen, which starts the wait, and never once closed.
    pub(crate) fn failed(&mut self) -> bool {
        if self.closed {
            return false;
        }
        let now = self.clock.now().as_millisecond();
        let Some(seen) = self.seen else {
            self.failures = self.failures.saturating_add(1);
            self.seen = Some(now);
            return false;
        };
        let wait = i64::try_from(wait_after(self.failures).as_millis()).unwrap_or(i64::MAX);
        let due = now >= seen.saturating_add(wait);
        if due {
            self.seen = None;
        }
        due
    }

    /// Never starts again.
    pub(crate) fn close(&mut self) {
        self.closed = true;
    }
}

/// How long a server waits to start again after its `failures`th failed start.
fn wait_after(failures: u32) -> Duration {
    let doublings = failures.saturating_sub(1).min(16);
    FIRST.saturating_mul(1 << doublings).min(MOST)
}

#[cfg(test)]
mod tests {
    use super::*;
    use glave_ports::testing::ManualClock;

    #[test]
    fn the_wait_doubles_from_ten_seconds_to_ten_minutes() {
        let waits: Vec<u64> = (1..=8).map(|n| wait_after(n).as_secs()).collect();
        assert_eq!(waits, [10, 20, 40, 80, 160, 320, 600, 600]);
        assert_eq!(wait_after(u32::MAX), MOST);
    }

    #[test]
    fn a_failure_is_tried_again_only_once_its_wait_is_over_and_never_once_closed() {
        let clock = Arc::new(ManualClock::at_millis(0));
        let mut again = Again::new(Arc::clone(&clock) as Arc<dyn Clock>);
        assert!(!again.retrying());
        assert!(
            !again.failed(),
            "the first sight of a failure starts the wait"
        );
        assert!(again.retrying());
        clock.advance_seconds(9);
        assert!(!again.failed());
        clock.advance_seconds(1);
        assert!(again.failed(), "ten seconds on, it starts again");
        // That start failed too: twenty seconds this time.
        assert!(!again.failed());
        clock.advance_seconds(19);
        assert!(!again.failed());
        clock.advance_seconds(1);
        assert!(again.failed());
        assert!(!again.failed());
        again.close();
        clock.advance_seconds(3_600);
        assert!(!again.failed());
    }
}
