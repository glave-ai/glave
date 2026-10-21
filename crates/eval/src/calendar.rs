//! The clock an attempt runs on: the computer's, set back to the day each thread happens on
//! (`days_ago`), so a month of threads runs in minutes while memory dates it as a month and
//! tidy-up comes due as it would over one. A task that names no days runs on the computer's.

use std::sync::Arc;
use std::sync::atomic::{AtomicI64, Ordering};

use glave_ports::Clock;
use jiff::{SignedDuration, Timestamp};

/// Seconds in a day.
const DAY: i64 = 24 * 60 * 60;

/// A clock some whole days behind another. Clones share the setting.
#[derive(Clone)]
pub(crate) struct Calendar {
    clock: Arc<dyn Clock>,
    /// Seconds behind `clock`.
    behind: Arc<AtomicI64>,
}

impl Calendar {
    /// Reads as `clock` does until it is set back.
    pub(crate) fn new(clock: Arc<dyn Clock>) -> Self {
        Self {
            clock,
            behind: Arc::new(AtomicI64::new(0)),
        }
    }

    /// Reads `days` days behind its clock from now on.
    pub(crate) fn set_days_ago(&self, days: u32) {
        self.behind
            .store(i64::from(days).saturating_mul(DAY), Ordering::SeqCst);
    }
}

impl Clock for Calendar {
    fn now(&self) -> Timestamp {
        let now = self.clock.now();
        let behind = SignedDuration::from_secs(self.behind.load(Ordering::SeqCst));
        now.checked_sub(behind).unwrap_or(now)
    }
}

#[cfg(test)]
mod tests {
    use glave_ports::testing::ManualClock;

    use super::*;

    #[test]
    fn it_reads_whole_days_behind_its_clock_and_clones_share_the_day() {
        let clock = ManualClock::at_second(1_800_000_000);
        let calendar = Calendar::new(Arc::new(clock.clone()));
        assert_eq!(calendar.now().as_second(), 1_800_000_000);
        let shared = calendar.clone();
        calendar.set_days_ago(30);
        assert_eq!(shared.now().as_second(), 1_800_000_000 - 30 * DAY);
        clock.advance_seconds(5);
        calendar.set_days_ago(0);
        assert_eq!(shared.now().as_second(), 1_800_000_005, "today again");
    }
}
