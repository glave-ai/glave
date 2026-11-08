//! Reading a thread again from its log, from its end (`specs/storage.md`, "A thread read from
//! its end"): back to its latest compaction and the turn it fell in, which is all its
//! conversation, and of what came before only what outlasts a compaction.

use std::collections::BTreeMap;

use glave_event::{
    ApprovalDecided, ApprovalRequested, Body, CommandEnded, CommandStarted, EffortChosen, Event,
    MessageAdded, ModelChosen, Payload, PermissionsChosen, PlanApproved, PlanUpdated, Seq,
    ThreadArchived, ThreadCompacted, ThreadDeleted, ThreadForked, ThreadId, ThreadRenamed,
    ThreadStarted, ThreadUnarchived, ToolCallStarted, TurnStarted, WorkspaceBound,
    WorkspaceRemoved, decode,
};
use glave_ports::{BlobStore, EventLog, ModelProvider, Scope};

use crate::approval::Approver;
use crate::harness::{Harness, HarnessError};
use crate::judge::MOST_MESSAGES;

/// Events read per page.
const PAGE: usize = 1_000;

/// What came before a thread's end that still decides how it goes on: how it started, where it
/// works, its standing, title and choices, what the person allowed, its turns, its
/// commands in the background and its plan, and glave's own records, which call ids do not
/// count (`bind::own_records`).
const RECORDS: &[&str] = &[
    ThreadStarted::TYPE,
    TurnStarted::TYPE,
    ApprovalRequested::TYPE,
    ApprovalDecided::TYPE,
    WorkspaceBound::TYPE,
    WorkspaceRemoved::TYPE,
    ThreadArchived::TYPE,
    ThreadUnarchived::TYPE,
    ThreadDeleted::TYPE,
    ThreadRenamed::TYPE,
    ModelChosen::TYPE,
    PermissionsChosen::TYPE,
    EffortChosen::TYPE,
    CommandStarted::TYPE,
    CommandEnded::TYPE,
    PlanUpdated::TYPE,
    PlanApproved::TYPE,
    ToolCallStarted::TYPE,
    ThreadForked::TYPE,
];

/// What is read back from before a thread's end, and how many at a time.
struct Back {
    kinds: &'static [&'static str],
    page: usize,
}

/// Its records, every one.
const EARLIER: Back = Back {
    kinds: RECORDS,
    page: PAGE,
};

/// Its messages, back to the person's latest few: a page holds about as many as are wanted.
const ASKED: Back = Back {
    kinds: &[MessageAdded::TYPE],
    page: MOST_MESSAGES,
};

impl<P, L, B, A> Harness<P, L, B, A>
where
    P: ModelProvider,
    L: EventLog,
    B: BlobStore,
    A: Approver,
{
    /// Thread `id`'s events as reading it again needs them, in order: every one from the
    /// `turn.started` at or before its latest compaction (all of them when it has none), and
    /// before that its records ([`RECORDS`]), the person's latest messages, which the judge
    /// reads and the latest of which a compaction keeps.
    pub(crate) async fn load(
        &self,
        scope: &Scope,
        id: ThreadId,
    ) -> Result<Vec<Event>, HarnessError> {
        // Boxed: its reads, a store's each, are large futures, and a load is awaited inside
        // many, so unboxed it would grow every one of them on the stack that polls them.
        Box::pin(self.load_now(scope, id)).await
    }

    async fn load_now(&self, scope: &Scope, id: ThreadId) -> Result<Vec<Event>, HarnessError> {
        let from = self.end_of(scope, id).await?;
        let end = self.from(scope, id, from).await?;
        let Some(from) = from.filter(|_| !end.is_empty()) else {
            return Ok(end);
        };
        let mut asked = end.iter().filter(|event| by_person(event)).count();
        let records = self.back(scope, id, &EARLIER, from, |_| false);
        let mut events = in_order(records.await?.into_iter().chain(end));
        if asked < MOST_MESSAGES {
            let messages = self.back(scope, id, &ASKED, from, |event| {
                asked = asked.saturating_add(usize::from(by_person(event)));
                asked >= MOST_MESSAGES
            });
            events.extend(in_order(messages.await?));
        }
        Ok(events.into_values().collect())
    }

    /// Where thread `id`'s end begins: the `turn.started` at or before its latest compaction,
    /// or the compaction itself when no turn came before it; `None`, its start, when it was
    /// never compacted.
    async fn end_of(&self, scope: &Scope, id: ThreadId) -> Result<Option<Seq>, HarnessError> {
        let Some(compacted) = self
            .log
            .latest_of(scope, id, &[ThreadCompacted::TYPE])
            .await?
        else {
            return Ok(None);
        };
        let kinds = [TurnStarted::TYPE];
        let started = self
            .log
            .read_back(scope, id, Some(&kinds), Some(compacted.seq), 1)
            .await?;
        Ok(Some(started.first().map_or(compacted.seq, |turn| turn.seq)))
    }

    /// Thread `id`'s events of `back`'s kinds before `before`, newest first, back to the first
    /// that `last` says is the last wanted, or to its start.
    async fn back(
        &self,
        scope: &Scope,
        id: ThreadId,
        back: &Back,
        before: Seq,
        mut last: impl FnMut(&Event) -> bool,
    ) -> Result<Vec<Event>, HarnessError> {
        let mut events = Vec::new();
        let mut before = before;
        loop {
            let read = self
                .log
                .read_back(scope, id, Some(back.kinds), Some(before), back.page)
                .await?;
            let full = read.len() == back.page;
            before = read.last().map_or(before, |event| event.seq);
            for raw in read {
                let event = decode(raw).map_err(|e| HarnessError::Event(e.to_string()))?;
                let wanted_last = last(&event);
                events.push(event);
                if wanted_last {
                    return Ok(events);
                }
            }
            if !full {
                return Ok(events);
            }
        }
    }

    /// Thread `id`'s events from `from` (or its start), in order.
    pub(crate) async fn from(
        &self,
        scope: &Scope,
        id: ThreadId,
        from: Option<Seq>,
    ) -> Result<Vec<Event>, HarnessError> {
        let mut events = Vec::new();
        let mut after = from.and_then(|from| Seq::new(from.get().saturating_sub(1)).ok());
        loop {
            let read = self.log.read(scope, id, after, PAGE).await?;
            let full = read.len() == PAGE;
            after = read.last().map(|event| event.seq).or(after);
            for raw in read {
                events.push(decode(raw).map_err(|e| HarnessError::Event(e.to_string()))?);
            }
            if !full {
                return Ok(events);
            }
        }
    }
}

/// Events by their place in the thread, each once.
fn in_order(events: impl IntoIterator<Item = Event>) -> BTreeMap<Seq, Event> {
    events
        .into_iter()
        .map(|event| (event.header.seq, event))
        .collect()
}

/// Whether `event` is a message of the person's own.
fn by_person(event: &Event) -> bool {
    matches!(&event.body, Body::MessageAdded(added) if super::by_person(added))
}

#[cfg(test)]
mod tests {
    use glave_ports::InMemoryBlobs;
    use glave_ports::testing::{FaultyLog, ScriptedModel};

    use super::*;
    use crate::approval::AllowAll;

    type Held = Harness<ScriptedModel, FaultyLog, InMemoryBlobs, AllowAll>;

    /// The size of what `call` returns, never calling it.
    fn returned<F, Fut>(_: &F) -> usize
    where
        F: Fn(&'static Held, &'static Scope, ThreadId) -> Fut,
    {
        size_of::<Fut>()
    }

    /// A load is awaited inside many futures, a server's dispatch among them, which a debug
    /// build polls on one thread's stack: boxed, it adds a pointer to each, never its reads,
    /// whose futures are large on Postgres.
    #[test]
    fn loading_a_thread_costs_its_callers_a_pointer() {
        let load = returned(&|harness, scope, id| harness.load(scope, id));
        assert!(load <= 64, "a load's future holds {load} bytes");
    }
}
