//! Counting each browser's pushes that failed in a row through its own fault, so one its push
//! service keeps refusing is forgotten as if it were gone, while an outage of this server's
//! network or of the push service forgets no one.

use std::collections::BTreeMap;
use std::sync::{Mutex, PoisonError};

use glave_event::DeviceId;
use glave_push::{PushError, Sent};

/// How many pushes in a row may fail through the subscription's own fault before a browser is
/// forgotten as if it were gone.
const FAILURES: u8 = 3;

/// The browsers whose last pushes failed that are counted, past which the oldest count goes.
const MOST_COUNTED: usize = 4_096;

/// Each browser's pushes that failed in a row through its own fault, for at most
/// [`MOST_COUNTED`] browsers.
#[derive(Default)]
pub(super) struct Failing(Mutex<BTreeMap<DeviceId, u8>>);

impl Failing {
    /// `sent`, a push to `device`, counted: gone from the [`FAILURES`]th failure in a row its push
    /// service put down to the subscription, until the browser is forgotten or a push gets
    /// through. Any other failure is said as it is and counts for nothing.
    pub(super) fn counted(
        &self,
        device: DeviceId,
        sent: Result<Sent, PushError>,
    ) -> Result<Sent, PushError> {
        let mut failing = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        let error = match sent {
            Ok(sent) => {
                failing.remove(&device);
                return Ok(sent);
            }
            Err(error) if error.of_the_subscription() => error,
            Err(error) => return Err(error),
        };
        let count = failing.entry(device).or_default();
        *count = count.saturating_add(1);
        if *count >= FAILURES {
            return Ok(Sent::Gone);
        }
        if failing.len() > MOST_COUNTED {
            failing.pop_first();
        }
        Err(error)
    }

    /// Stops counting `device`, forgotten now.
    pub(super) fn forgot(&self, device: DeviceId) {
        let mut failing = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        failing.remove(&device);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use reqwest::StatusCode;

    fn device(n: u8) -> DeviceId {
        DeviceId::new(jiff::Timestamp::UNIX_EPOCH, [n; 10]).unwrap()
    }

    fn refused(status: StatusCode) -> Result<Sent, PushError> {
        Err(PushError::Refused(status))
    }

    #[test]
    fn a_browser_its_push_service_refuses_three_times_in_a_row_is_gone() {
        let failing = Failing::default();
        let (one, other) = (device(1), device(2));
        for _ in 1..FAILURES {
            assert!(
                failing
                    .counted(one, refused(StatusCode::FORBIDDEN))
                    .is_err()
            );
        }
        let other_failed = failing.counted(other, refused(StatusCode::FORBIDDEN));
        assert!(other_failed.is_err(), "counted apart");
        let gone = failing.counted(one, refused(StatusCode::BAD_REQUEST));
        assert!(matches!(gone, Ok(Sent::Gone)));
        // Until it is forgotten, it stays gone; a push taken starts the count again.
        let again = failing.counted(one, refused(StatusCode::FORBIDDEN));
        assert!(matches!(again, Ok(Sent::Gone)));
        failing.forgot(one);
        assert!(
            failing
                .counted(one, refused(StatusCode::FORBIDDEN))
                .is_err()
        );
        assert!(failing.counted(other, Ok(Sent::Taken)).is_ok());
        for _ in 1..FAILURES {
            assert!(
                failing
                    .counted(other, refused(StatusCode::FORBIDDEN))
                    .is_err()
            );
        }
    }

    #[test]
    fn an_outage_of_the_network_or_the_push_service_forgets_no_one() {
        let failing = Failing::default();
        let one = device(1);
        let outages = || {
            [
                Err(PushError::TimedOut),
                Err(PushError::Unreached("no route to host".into())),
                refused(StatusCode::TOO_MANY_REQUESTS),
                refused(StatusCode::REQUEST_TIMEOUT),
                refused(StatusCode::SERVICE_UNAVAILABLE),
                refused(StatusCode::INTERNAL_SERVER_ERROR),
            ]
        };
        for _ in 0..FAILURES {
            for outage in outages() {
                assert!(failing.counted(one, outage).is_err(), "never gone");
            }
        }
    }
}
