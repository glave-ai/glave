//! A thread read back from its end, turn by turn (`thread/turns`), as a client opens it: a
//! thread of any length opens with its latest turns alone, and what came before is read when
//! the person goes back to it.

use glave_event::{EffortChosen, ModelChosen, Payload as _, RawEvent, Seq, ThreadId, TurnStarted};
use glave_ports::stream::Reserved;
use glave_ports::{BlobStore, EventLog, ModelProvider, Scope};

use crate::protocol::{ApiError, Reply};
use crate::server::{MAX_PAGE, Server, no_thread};
use crate::thought::served;

/// The turns one page of `thread/turns` holds: more than a screen shows, so a thread opens
/// filling it.
const TURNS: usize = 20;

/// The choices a thread makes for every turn after them, which a client reads from the latest
/// of each it has: read from the end, each comes with the turns when it was made before them.
const CHOICES: [&str; 2] = [ModelChosen::TYPE, EffortChosen::TYPE];

impl<P, L, B> Server<P, L, B>
where
    P: ModelProvider + 'static,
    L: EventLog + 'static,
    B: BlobStore + 'static,
{
    /// `thread`'s latest turns before `before` (up to its end when absent), whole, at most
    /// [`MAX_PAGE`] events, and the `before` of what came before them, when anything did.
    pub(crate) async fn turns(
        &self,
        thread: ThreadId,
        before: Option<Seq>,
    ) -> Result<Reply, ApiError> {
        if Reserved::of(thread).is_some() {
            return Err(no_thread(thread));
        }
        self.ensure_not_deleted(thread).await?;
        let scope = self.scope_of(thread)?;
        let log = self.inner.harness.log();
        let last = log.read_back(&scope, thread, None, before, 1).await?;
        let Some(last) = last.first().map(|event| event.seq) else {
            return match before {
                None => Err(no_thread(thread)),
                Some(_) => Ok(Reply::Events {
                    events: Vec::new(),
                    next: None,
                    earlier: None,
                }),
            };
        };
        let (from, more) = self.first_of(&scope, thread, last).await?;
        let count = usize::try_from(last.get() - from.get() + 1).unwrap_or(MAX_PAGE);
        let after = Seq::new(from.get() - 1).ok();
        let mut events = log.read(&scope, thread, after, count).await?;
        let earlier = events.first().map(|event| event.seq).filter(|_| more);
        if let (None, Some(first)) = (before, earlier) {
            let mut choices = self.choices(&scope, thread, first).await?;
            choices.append(&mut events);
            events = choices;
        }
        Ok(Reply::Events {
            events: events.into_iter().map(served).collect(),
            next: None,
            earlier,
        })
    }

    /// Where a page of turns that ends at `last` begins: the start of the [`TURNS`]th turn
    /// back, else the thread's beginning, at most [`MAX_PAGE`] events back; and whether
    /// anything comes before it.
    async fn first_of(
        &self,
        scope: &Scope,
        thread: ThreadId,
        last: Seq,
    ) -> Result<(Seq, bool), ApiError> {
        let starts = self
            .inner
            .harness
            .log()
            .read_back(
                scope,
                thread,
                Some(&[TurnStarted::TYPE]),
                last.next(),
                TURNS + 1,
            )
            .await?;
        let turn = starts
            .get(TURNS - 1)
            .filter(|_| starts.len() > TURNS)
            .map(|start| start.seq);
        Ok(first(last, turn, MAX_PAGE))
    }

    /// The latest of each of [`CHOICES`] before `first`, in order.
    async fn choices(
        &self,
        scope: &Scope,
        thread: ThreadId,
        first: Seq,
    ) -> Result<Vec<RawEvent>, ApiError> {
        let log = self.inner.harness.log();
        let mut choices = Vec::with_capacity(CHOICES.len());
        for kind in CHOICES {
            let latest = log
                .read_back(scope, thread, Some(&[kind]), Some(first), 1)
                .await?;
            choices.extend(latest);
        }
        choices.sort_by_key(|event| event.seq);
        Ok(choices)
    }
}

/// Where a page that ends at `last` begins, given the start of its earliest turn when a turn
/// comes before it (else it begins with the thread), holding at most `page` events; and
/// whether anything comes before it.
fn first(last: Seq, turn: Option<Seq>, page: usize) -> (Seq, bool) {
    let page = u64::try_from(page).unwrap_or(u64::MAX).max(1);
    let lowest = Seq::new(last.get().saturating_sub(page - 1)).unwrap_or(Seq::FIRST);
    match turn {
        Some(turn) if turn >= lowest => (turn, true),
        Some(_) => (lowest, true),
        None if lowest > Seq::FIRST => (lowest, true),
        None => (Seq::FIRST, false),
    }
}

#[cfg(test)]
mod tests {
    use glave_event::Seq;

    use super::first;

    fn seq(n: u64) -> Seq {
        Seq::new(n).unwrap()
    }

    #[test]
    fn a_page_begins_with_its_earliest_turn_or_the_thread_and_never_holds_more_than_it_may() {
        assert_eq!(first(seq(100), Some(seq(60)), 5_000), (seq(60), true));
        assert_eq!(first(seq(100), None, 5_000), (Seq::FIRST, false));
        // A turn longer than a page: the page holds its end alone.
        assert_eq!(first(seq(9_000), Some(seq(10)), 5_000), (seq(4_001), true));
        assert_eq!(first(seq(9_000), None, 5_000), (seq(4_001), true));
        assert_eq!(first(seq(5_000), None, 5_000), (Seq::FIRST, false));
    }
}
