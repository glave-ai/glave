//! A log in memory that misbehaves when told, as a store does: one that blips fails the next
//! write or read holding an event of a given kind, and a slow one holds an append until the
//! test lets it go.

use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use glave_event::{RawEvent, Seq, ThreadId};
use tokio::sync::Notify;

use crate::in_memory::InMemoryLog;
use crate::log::{EventLog, LogError};
use crate::scope::{Located, Scope};
use crate::search::Found;
use crate::view::{ThreadQuery, ThreadView};

/// A log in memory that fails or holds when told, and is otherwise a log in memory. Clones
/// share the log and what they were told.
#[derive(Clone, Default)]
pub struct FaultyLog {
    log: Arc<InMemoryLog>,
    faults: Arc<Faults>,
}

/// What a [`FaultyLog`] was told.
#[derive(Default)]
struct Faults {
    /// The kind of event whose next write or read fails, until one does.
    blip: Mutex<Option<&'static str>>,
    /// The kind of event whose next append is held, until one is.
    hold: Mutex<Option<&'static str>>,
    /// Told when an append is held.
    held: Notify,
    /// Lets the held append go.
    released: Notify,
}

impl FaultyLog {
    /// Fails the next write or read that holds an event of `kind`, writing or reading nothing,
    /// as a store that blips does.
    pub fn fail_with(&self, kind: &'static str) {
        *lock(&self.faults.blip) = Some(kind);
    }

    /// Holds the next append that holds an event of `kind` before it lands, until
    /// [`FaultyLog::release`]; [`FaultyLog::held`] says when one is.
    pub fn hold(&self, kind: &'static str) {
        *lock(&self.faults.hold) = Some(kind);
    }

    /// Waits until an append is held.
    pub async fn held(&self) {
        self.faults.held.notified().await;
    }

    /// Lets the held append go on and land.
    pub fn release(&self) {
        self.faults.released.notify_one();
    }
}

/// Takes `told`'s kind if `events` hold an event of it, saying whether they did.
fn strikes(told: &Mutex<Option<&'static str>>, events: &[RawEvent]) -> bool {
    let holds = |kind: &mut &'static str| events.iter().any(|event| event.kind.as_str() == *kind);
    lock(told).take_if(holds).is_some()
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

fn blipped() -> LogError {
    LogError::Storage("the store did not answer".into())
}

/// What it does is decided as each call is made, as the log in memory does its work then: a
/// held append alone waits to land.
#[deny(clippy::missing_trait_methods)]
impl EventLog for FaultyLog {
    fn append(
        &self,
        scope: &Scope,
        events: &[RawEvent],
    ) -> impl Future<Output = Result<(), LogError>> + Send {
        let blips = strikes(&self.faults.blip, events);
        let held = !blips && strikes(&self.faults.hold, events);
        let now = (!blips && !held).then(|| self.log.append(scope, events));
        async move {
            if blips {
                return Err(blipped());
            }
            if let Some(appended) = now {
                return appended.await;
            }
            self.faults.held.notify_one();
            self.faults.released.notified().await;
            self.log.append(scope, events).await
        }
    }

    fn read(
        &self,
        scope: &Scope,
        thread: ThreadId,
        after: Option<Seq>,
        limit: usize,
    ) -> impl Future<Output = Result<Vec<RawEvent>, LogError>> + Send {
        let read = self.log.read(scope, thread, after, limit);
        async move {
            let events = read.await?;
            if strikes(&self.faults.blip, &events) {
                return Err(blipped());
            }
            Ok(events)
        }
    }

    fn search(
        &self,
        scope: &Scope,
        query: &str,
        limit: usize,
    ) -> impl Future<Output = Result<Vec<Found>, LogError>> + Send {
        self.log.search(scope, query, limit)
    }

    fn locate(
        &self,
        thread: ThreadId,
    ) -> impl Future<Output = Result<Option<Located>, LogError>> + Send {
        self.log.locate(thread)
    }

    fn thread_views(
        &self,
        scope: &Scope,
        query: &ThreadQuery,
    ) -> impl Future<Output = Result<Vec<ThreadView>, LogError>> + Send {
        self.log.thread_views(scope, query)
    }

    fn thread_view(
        &self,
        scope: &Scope,
        thread: ThreadId,
    ) -> impl Future<Output = Result<Option<ThreadView>, LogError>> + Send {
        self.log.thread_view(scope, thread)
    }

    fn rebuild_thread_view(
        &self,
        scope: &Scope,
        thread: ThreadId,
    ) -> impl Future<Output = Result<Option<ThreadView>, LogError>> + Send {
        self.log.rebuild_thread_view(scope, thread)
    }

    fn latest_of(
        &self,
        scope: &Scope,
        thread: ThreadId,
        kinds: &[&str],
    ) -> impl Future<Output = Result<Option<RawEvent>, LogError>> + Send {
        self.log.latest_of(scope, thread, kinds)
    }
}

#[cfg(test)]
mod tests {
    use glave_event::{At, EventId, Header, Outcome, TurnResult, encode};
    use jiff::Timestamp;

    use super::*;
    use crate::scope::{OrgId, ProjectId};
    use crate::{block_on, conformance};

    fn scope() -> Scope {
        Scope::glaves_own(
            OrgId::from_uuid(uuid::Uuid::from_bytes([1; 16])),
            ProjectId::from_uuid(uuid::Uuid::from_bytes([2; 16])),
        )
    }

    fn thread() -> ThreadId {
        ThreadId::new(Timestamp::UNIX_EPOCH, [3; 10]).unwrap()
    }

    /// The thread's first event: a turn's end.
    fn ended() -> Vec<RawEvent> {
        let header = Header {
            id: EventId::new(Timestamp::UNIX_EPOCH, [1; 10]).unwrap(),
            thread: thread(),
            seq: Seq::new(1).unwrap(),
            at: At::try_from("2026-10-07T21:14:03.512Z".to_owned()).unwrap(),
        };
        let ended = TurnResult {
            outcome: Outcome::Completed,
            ..TurnResult::begun()
        };
        vec![encode(header, &ended).unwrap()]
    }

    async fn kept(log: &FaultyLog) -> usize {
        log.read(&scope(), thread(), None, 10).await.unwrap().len()
    }

    #[test]
    fn told_nothing_it_is_a_log_in_memory() {
        assert_eq!(
            block_on!(conformance::event_log(&FaultyLog::default())),
            Ok(())
        );
    }

    #[test]
    fn a_blip_fails_one_write_or_read_of_its_kind() {
        let log = FaultyLog::default();
        block_on!(async {
            log.fail_with("turn.ended");
            assert!(log.append(&scope(), &ended()).await.is_err());
            assert_eq!(kept(&log).await, 0, "nothing landed");
            log.append(&scope(), &ended()).await.unwrap();
            log.fail_with("turn.ended");
            assert!(log.read(&scope(), thread(), None, 10).await.is_err());
            assert_eq!(kept(&log).await, 1, "once");
        });
    }

    #[test]
    fn a_held_append_lands_once_it_is_released() {
        let log = FaultyLog::default();
        block_on!(async {
            log.hold("turn.ended");
            let held = log.clone();
            let appending = tokio::spawn(async move { held.append(&scope(), &ended()).await });
            log.held().await;
            assert_eq!(kept(&log).await, 0, "held, not landed");
            log.release();
            appending.await.unwrap().unwrap();
            assert_eq!(kept(&log).await, 1);
        });
    }
}
