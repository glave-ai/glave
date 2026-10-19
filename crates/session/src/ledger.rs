//! Each space's task ledger: glave's own stream in the event log, under a reserved thread id,
//! holding the space's tasks (`task.started`, `task.ended`) and schedules (`schedule.created`,
//! `schedule.deleted`). It is never a conversation: lists and reads of threads leave it out.
//! Any glave process of the deployment may append to it; an append that meets another's
//! `(thread, seq)` conflict reads what was written and tries again after it.

use std::collections::{BTreeMap, HashMap, VecDeque};

use glave_event::{
    At, Body, EventId, Header, Payload, RawEvent, ScheduleCreated, ScheduleId, Seq, TaskEnded,
    TaskStarted, ThreadId, decode, encode,
};
use glave_ports::{BlobStore, Caller, EventLog, LogError, ModelProvider, SpaceId, reserved_ledger};

use crate::protocol::{ApiError, code};
use crate::server::Server;

/// Finished tasks kept in memory for `tasks/list`, newest last, in each space.
const FINISHED: usize = 100;
/// Spaces whose ledgers a server keeps in memory at once.
const LEDGERS: usize = 1_024;
/// Ledger events read per page.
const PAGE: usize = 500;
/// Appends tried before giving up on a ledger other writers keep moving.
const ATTEMPTS: usize = 8;

/// The ledger's reserved id: the UUID v7 at time zero with nothing random,
/// `00000000-0000-7000-8000-000000000000`. No thread can have it: a thread's id carries the
/// time it started.
pub(crate) fn ledger_id() -> Option<ThreadId> {
    reserved_ledger(0)
}

/// The accounts ledger's reserved id, `00000000-0000-7000-8000-000000000002`: the model
/// accounts (`provider.connected`, `provider.removed`) and git hosts (`credential.connected`,
/// `credential.removed`) a person connected, in their personal space.
pub fn accounts_ledger() -> Option<ThreadId> {
    reserved_ledger(2)
}

/// The space ledger's reserved id, `00000000-0000-7000-8000-000000000003`: a space's own
/// settings, today a project's folder (`space.folder.set`).
pub fn space_ledger() -> Option<ThreadId> {
    reserved_ledger(3)
}

/// The trust ledger's reserved id, `00000000-0000-7000-8000-000000000004`: what a person
/// decided about the projects that ask to be trusted (`trust.decided`), in their personal
/// space.
pub fn trust_ledger() -> Option<ThreadId> {
    reserved_ledger(4)
}

/// Whether `thread` is one of the space's ledgers (tasks, memory, accounts, trust, browsers,
/// or the space's own) rather than a thread.
pub fn is_ledger(thread: ThreadId) -> bool {
    [
        ledger_id(),
        glave_ports::memories::memory_ledger(),
        accounts_ledger(),
        space_ledger(),
        trust_ledger(),
        glave_identity::signin::browsers_ledger(),
    ]
    .contains(&Some(thread))
}

/// A schedule that has not been deleted.
#[derive(Debug, Clone)]
pub(crate) struct Scheduled {
    pub(crate) created: ScheduleCreated,
    pub(crate) at: At,
    /// When the latest run started by it was due.
    pub(crate) last: Option<At>,
}

/// A task as the ledger has it: when it started, and how it ended once it has.
#[derive(Debug, Clone)]
pub(crate) struct Recorded {
    pub(crate) started: TaskStarted,
    pub(crate) at: At,
    pub(crate) ended: Option<(TaskEnded, At)>,
}

/// What the ledger says so far.
#[derive(Debug, Default)]
pub(crate) struct Ledger {
    /// The `seq` the next event will take.
    next: u64,
    pub(crate) schedules: BTreeMap<ScheduleId, Scheduled>,
    /// Tasks started and not ended.
    pub(crate) open: HashMap<ThreadId, Recorded>,
    /// Tasks ended, newest last.
    pub(crate) finished: VecDeque<Recorded>,
}

impl Ledger {
    fn next_seq(&self) -> u64 {
        self.next.max(1)
    }

    /// Takes in the event at the next `seq`; any other is already in, or not yet.
    fn apply(&mut self, raw: RawEvent) {
        if raw.seq.get() != self.next_seq() {
            return;
        }
        self.next = self.next_seq().saturating_add(1);
        let at = raw.at;
        let Ok(event) = decode(raw) else {
            return;
        };
        match event.body {
            Body::ScheduleCreated(created) => {
                let scheduled = Scheduled {
                    created,
                    at,
                    last: None,
                };
                self.schedules.insert(scheduled.created.schedule, scheduled);
            }
            Body::ScheduleDeleted(deleted) => {
                self.schedules.remove(&deleted.schedule);
            }
            Body::TaskStarted(started) => {
                if let Some(fired) = started.schedule
                    && let Some(scheduled) = self.schedules.get_mut(&fired.schedule)
                {
                    scheduled.last = scheduled.last.max(Some(fired.due));
                }
                let recorded = Recorded {
                    started,
                    at,
                    ended: None,
                };
                self.open.insert(recorded.started.thread, recorded);
            }
            Body::TaskEnded(ended) => {
                if let Some(mut recorded) = self.open.remove(&ended.thread) {
                    recorded.ended = Some((ended, at));
                    if self.finished.len() >= FINISHED {
                        self.finished.pop_front();
                    }
                    self.finished.push_back(recorded);
                }
            }
            _ => {}
        }
    }
}

/// What each space's ledger says, as far as this server has read it.
pub(crate) type Ledgers = HashMap<SpaceId, Ledger>;

/// `space`'s ledger in `ledgers`, empty until it is read. Past [`LEDGERS`] spaces, the others
/// are forgotten and read again when next asked for.
fn kept(ledgers: &mut Ledgers, space: SpaceId) -> &mut Ledger {
    if ledgers.len() >= LEDGERS && !ledgers.contains_key(&space) {
        ledgers.clear();
    }
    ledgers.entry(space).or_default()
}

impl<P, L, B> Server<P, L, B>
where
    P: ModelProvider + 'static,
    L: EventLog + 'static,
    B: BlobStore + 'static,
{
    /// Reads whatever was added to `space`'s ledger since this server last looked.
    pub(crate) async fn catch_up(&self, space: SpaceId) -> Result<(), ApiError> {
        let ledger = ledger_id().ok_or_else(|| internal("the ledger has no id"))?;
        let scope = self.scope_in(space);
        loop {
            let next = self.ledgers().get(&space).map_or(1, Ledger::next_seq);
            let after = Seq::new(next.saturating_sub(1)).ok();
            let page = self
                .inner
                .harness
                .log()
                .read(&scope, ledger, after, PAGE)
                .await
                .map_err(|error| internal(&error.to_string()))?;
            let full = page.len() == PAGE;
            let mut held = self.ledgers();
            let read = kept(&mut held, space);
            for raw in page {
                read.apply(raw);
            }
            if !full {
                return Ok(());
            }
        }
    }

    /// Appends `payload` to `space`'s ledger, after anything others wrote meanwhile.
    pub(crate) async fn record_in_ledger<T: Payload>(
        &self,
        space: SpaceId,
        payload: &T,
    ) -> Result<(), ApiError> {
        let ledger = ledger_id().ok_or_else(|| internal("the ledger has no id"))?;
        let scope = self.scope_in(space);
        for _ in 0..ATTEMPTS {
            self.catch_up(space).await?;
            let now = self.inner.harness.clock().now();
            let next = self.ledgers().get(&space).map_or(1, Ledger::next_seq);
            let seq = Seq::new(next).map_err(|e| internal(&e.to_string()))?;
            let header = Header {
                id: EventId::new(now, self.inner.harness.entropy().bytes()?)
                    .map_err(|e| internal(&e.to_string()))?,
                thread: ledger,
                seq,
                at: At::from(now),
            };
            let raw = encode(header, payload).map_err(|e| internal(&e.to_string()))?;
            match self
                .inner
                .harness
                .log()
                .append(&scope, std::slice::from_ref(&raw))
                .await
            {
                Ok(()) => {
                    kept(&mut self.ledgers(), space).apply(raw);
                    return Ok(());
                }
                Err(LogError::Conflict { .. } | LogError::Gap { .. }) => {}
                Err(error) => return Err(internal(&error.to_string())),
            }
        }
        Err(internal("the task ledger kept changing; try again"))
    }

    /// The spaces whose tasks and schedules this server runs: its own, every space its person
    /// works in, and any other whose ledger it has read for a caller.
    pub(crate) async fn task_spaces(&self) -> Result<Vec<SpaceId>, ApiError> {
        let mut spaces = match self.inner.harness.person() {
            Some(_) => self.spaces_of(&Caller::default()).await?,
            None => Vec::new(),
        };
        let known: Vec<SpaceId> = self.ledgers().keys().copied().collect();
        for space in std::iter::once(self.inner.home.space).chain(known) {
            if !spaces.contains(&space) {
                spaces.push(space);
            }
        }
        Ok(spaces)
    }

    pub(crate) fn ledgers(&self) -> std::sync::MutexGuard<'_, Ledgers> {
        self.inner
            .ledgers
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

fn internal(message: &str) -> ApiError {
    ApiError::new(code::INTERNAL, message)
}
