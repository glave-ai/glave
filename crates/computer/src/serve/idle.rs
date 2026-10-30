//! When a computer served over TLS ends on its own (`specs/computers.md`, "A person's computer
//! in the cloud", 4 and 5): once no request has been answered or running for its limit and
//! nothing runs there in the background. A connection held open counts for nothing: a server
//! that keeps a computer awake asks it something at least once a minute.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use tokio::time::Instant;

/// How long a computer may go unasked before it ends on its own, and what else keeps it
/// serving.
pub struct Idle {
    pub after: Duration,
    /// Whether anything still runs there that no request is waiting on, such as a command in
    /// the background.
    pub busy: Box<dyn Fn() -> bool + Send + Sync>,
}

/// How often a computer serving with an [`Idle`] limit looks at whether it is idle.
const CHECK: Duration = Duration::from_secs(30);

/// The requests a computer answers, across its connections: how many run now, and when the
/// last one ended.
#[derive(Debug)]
pub(crate) struct Activity {
    running: AtomicUsize,
    last: Mutex<Instant>,
}

/// One request running, until it is dropped.
pub(crate) struct Answering(Arc<Activity>);

impl Activity {
    pub(crate) fn new() -> Arc<Self> {
        Arc::new(Self {
            running: AtomicUsize::new(0),
            last: Mutex::new(Instant::now()),
        })
    }

    /// Counts one request running, until what it answers is dropped.
    pub(crate) fn begin(activity: &Arc<Self>) -> Answering {
        activity.running.fetch_add(1, Ordering::SeqCst);
        Answering(Arc::clone(activity))
    }

    /// Counts now as a time the computer was in use.
    fn touch(&self) {
        *self.last.lock().unwrap_or_else(PoisonError::into_inner) = Instant::now();
    }

    /// Whether no request runs now.
    pub(crate) fn quiet(&self) -> bool {
        self.running.load(Ordering::SeqCst) == 0
    }

    /// Whether no request runs and none has ended for `after`.
    fn quiet_for(&self, after: Duration) -> bool {
        let last = *self.last.lock().unwrap_or_else(PoisonError::into_inner);
        self.running.load(Ordering::SeqCst) == 0 && last.elapsed() >= after
    }
}

impl Drop for Answering {
    fn drop(&mut self) {
        self.0.touch();
        self.0.running.fetch_sub(1, Ordering::SeqCst);
    }
}

impl Idle {
    /// Waits until `activity` has been quiet for this limit, a time something was busy
    /// counting as use, looking every 30 seconds (or the limit, when shorter): the computer may
    /// then end.
    pub(crate) async fn reached(&self, activity: &Activity) {
        let every = CHECK.min(self.after);
        loop {
            tokio::time::sleep(every).await;
            if (self.busy)() {
                activity.touch();
            } else if activity.quiet_for(self.after) {
                return;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn paused() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .start_paused(true)
            .build()
            .unwrap()
    }

    #[test]
    fn a_request_running_or_just_answered_keeps_it_serving() {
        paused().block_on(async {
            let activity = Activity::new();
            let idle = Idle {
                after: Duration::from_mins(10),
                busy: Box::new(|| false),
            };
            let answering = Activity::begin(&activity);
            let hour = Duration::from_hours(1);
            let running = tokio::time::timeout(hour, idle.reached(&activity)).await;
            assert!(running.is_err(), "a request runs for an hour");
            drop(answering);
            let started = Instant::now();
            tokio::time::timeout(hour, idle.reached(&activity))
                .await
                .unwrap();
            let took = started.elapsed();
            assert!(
                took >= Duration::from_mins(10) && took <= Duration::from_secs(630),
                "ended {took:?} after the last request"
            );
        });
    }
}
