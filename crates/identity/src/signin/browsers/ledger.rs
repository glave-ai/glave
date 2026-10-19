//! The browsers ledger in whichever store a server runs on: its one stream, caught up with and
//! appended to through `glave_ports::stream`, behind one object so a server's sign-in need not
//! name its store.

use glave_event::{RawEvent, Seq, ThreadId};
use glave_ports::stream::{StreamError, append_retrying, catch_up};
use glave_ports::{BoxFuture, EventLog, Scope};

use super::BrowserError;

/// What one try of an append makes: the events, or nothing to write.
pub(super) type Made = Result<Option<(Vec<RawEvent>, ())>, BrowserError>;

/// One stream of an event log.
pub(super) trait Ledger: Send + Sync {
    /// Hands the events after `after` to `take`, a page at a time, to the stream's end.
    fn catch_up<'a>(
        &'a self,
        after: Option<Seq>,
        take: &'a mut (dyn FnMut(Vec<RawEvent>) + Send),
    ) -> BoxFuture<'a, Result<(), StreamError>>;

    /// Appends what `attempt` makes, asking again when another server wrote first.
    fn append_retrying<'a>(
        &'a self,
        attempt: &'a mut (dyn FnMut() -> BoxFuture<'a, Made> + Send),
    ) -> BoxFuture<'a, Made>;
}

/// The stream `thread` in `scope` of `log`.
pub(super) struct Stream<L> {
    pub(super) log: L,
    pub(super) scope: Scope,
    pub(super) thread: ThreadId,
}

impl<L: EventLog> Ledger for Stream<L> {
    fn catch_up<'a>(
        &'a self,
        after: Option<Seq>,
        take: &'a mut (dyn FnMut(Vec<RawEvent>) + Send),
    ) -> BoxFuture<'a, Result<(), StreamError>> {
        Box::pin(catch_up(&self.log, &self.scope, self.thread, after, take))
    }

    fn append_retrying<'a>(
        &'a self,
        attempt: &'a mut (dyn FnMut() -> BoxFuture<'a, Made> + Send),
    ) -> BoxFuture<'a, Made> {
        Box::pin(append_retrying(
            &self.log,
            &self.scope,
            self.thread,
            attempt,
        ))
    }
}
