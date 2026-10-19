//! The browsers ledger in whichever store a server runs on: its one stream, read a page at a
//! time and appended to, behind one object so a server's sign-in need not name its store.

use glave_event::{RawEvent, Seq, ThreadId};
use glave_ports::{BoxFuture, EventLog, LogError, Scope};

/// Ledger events read per page.
pub(super) const PAGE: usize = 500;

/// One stream of an event log.
pub(super) trait Ledger: Send + Sync {
    /// Up to [`PAGE`] events after `after`, in order.
    fn read(&self, after: Option<Seq>) -> BoxFuture<'_, Result<Vec<RawEvent>, LogError>>;

    /// Appends `events`, which continue the stream's numbering.
    fn append<'a>(&'a self, events: &'a [RawEvent]) -> BoxFuture<'a, Result<(), LogError>>;
}

/// The stream `thread` in `scope` of `log`.
pub(super) struct Stream<L> {
    pub(super) log: L,
    pub(super) scope: Scope,
    pub(super) thread: ThreadId,
}

impl<L: EventLog> Ledger for Stream<L> {
    fn read(&self, after: Option<Seq>) -> BoxFuture<'_, Result<Vec<RawEvent>, LogError>> {
        Box::pin(self.log.read(&self.scope, self.thread, after, PAGE))
    }

    fn append<'a>(&'a self, events: &'a [RawEvent]) -> BoxFuture<'a, Result<(), LogError>> {
        Box::pin(self.log.append(&self.scope, events))
    }
}
