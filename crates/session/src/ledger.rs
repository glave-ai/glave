//! Each project's task ledger: glave's own stream in the event log, under a reserved thread id,
//! holding the project's tasks (`task.started`, `task.ended`) and schedules (`schedule.created`,
//! `.paused`, `.skipped`, `.deleted`). It is never a conversation: lists and reads of threads leave it out.
//! Any glave process of the deployment may append to it; an append that meets another's
//! `(thread, seq)` conflict reads what was written and tries again after it.

use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};

use glave_event::{
    At, Body, EventId, Header, Payload, RawEvent, ScheduleCreated, ScheduleId, Seq, Skipped,
    TaskEnded, TaskStarted, ThreadId, When, encode,
};
use glave_ports::stream::{Reserved, append_retrying, catch_up, in_order};
use glave_ports::{BlobStore, Caller, EventLog, Located, ModelProvider, Scope};

use crate::protocol::ApiError;
use crate::server::Server;

/// Finished tasks kept in memory for `tasks/list`, newest last, in each project.
const FINISHED: usize = 100;
/// Projects whose ledgers a server keeps in memory at once: the most whose schedules a tick reads
/// on from where it last read (#1542). Past this, the one read longest ago is forgotten, and
/// read again from its start when next asked for.
const KEPT: usize = 1_024;

/// A schedule that has not been deleted.
#[derive(Debug, Clone)]
pub(crate) struct Scheduled {
    pub(crate) created: ScheduleCreated,
    pub(crate) at: At,
    /// When the latest run started by it, or skipped, was due.
    pub(crate) last: Option<At>,
    /// Why that latest run did not start, when it was skipped.
    pub(crate) skipped: Option<Skipped>,
    /// When the latest run this server held back was due: one skipped again while the run
    /// before still goes, which is recorded once, or one whose skip could not be recorded. In
    /// memory only, so a server started later tries it once more.
    pub(crate) held: Option<At>,
    pub(crate) paused: bool,
    /// When it was last paused.
    paused_at: Option<At>,
    /// When it was last resumed: its runs count from then, none for the time it was paused.
    pub(crate) resumed: Option<At>,
    /// When a one-off's time came, when that was while it was paused: it runs no more.
    pub(crate) missed: Option<At>,
}

impl Scheduled {
    /// Takes in its pausing (`paused`) or resuming at `at`: a one-off that has not run and whose
    /// time came while it was paused is missed.
    fn pause(&mut self, paused: bool, at: At) {
        if paused && !self.paused {
            self.paused_at = Some(at);
        }
        if !paused {
            self.resumed = Some(at);
            if let When::At(due) = &self.created.when
                && self.paused
                && self.last.is_none()
                && self.paused_at.is_some_and(|since| since <= *due)
                && *due < at
            {
                self.missed = Some(*due);
            }
        }
        self.paused = paused;
    }

    /// Takes in a run that was due at `due`: started, or skipped for `skipped`.
    fn ran(&mut self, due: At, skipped: Option<Skipped>) {
        if self.last.is_none_or(|last| due >= last) {
            self.last = Some(due);
            self.skipped = skipped;
        }
    }
}

/// A task as the ledger has it: when it started, and how it ended once it has.
#[derive(Debug, Clone)]
pub(crate) struct Recorded {
    pub(crate) started: TaskStarted,
    pub(crate) at: At,
    pub(crate) ended: Option<(TaskEnded, At)>,
}

impl Recorded {
    /// Whether `ended` is this task's end: one for the run of its schedule that came due when
    /// this did, or one that names no run.
    fn ended_by(&self, ended: &TaskEnded) -> bool {
        ended.due.is_none() || self.started.schedule.map(|fired| fired.due) == ended.due
    }
}

/// What the ledger says so far.
#[derive(Debug, Default)]
pub(crate) struct Ledger {
    /// The `seq` the next event will take.
    next: u64,
    /// When it was last read, in the order of [`Ledgers`]' reads.
    read_at: u64,
    pub(crate) schedules: BTreeMap<ScheduleId, Scheduled>,
    /// When its last schedule went, while it holds none.
    pub(crate) emptied: Option<At>,
    /// Tasks started and not ended.
    pub(crate) open: HashMap<ThreadId, Recorded>,
    /// Tasks ended, newest last.
    pub(crate) finished: VecDeque<Recorded>,
}

impl Ledger {
    fn next_seq(&self) -> u64 {
        self.next.max(1)
    }

    /// Whether `schedule`'s run due at `due` is settled: started (its claim) or skipped, or one
    /// due later is, or the schedule was paused or deleted meanwhile.
    pub(crate) fn settled(&self, schedule: ScheduleId, due: At) -> bool {
        (self.schedules.get(&schedule)).is_none_or(|scheduled| {
            scheduled.paused || scheduled.last.is_some_and(|last| last >= due)
        })
    }

    /// Whether any of it was read yet.
    fn read(&self) -> bool {
        self.next > 0
    }

    /// Takes in the event at the next `seq`; any other is already in, or not yet. Answers the
    /// chat of the schedule it changed, if it did: its next run, its last, or whether it is
    /// paused, which the chat's summary says.
    fn apply(&mut self, raw: RawEvent) -> Option<ThreadId> {
        let (at, body) = in_order(&mut self.next, raw)?;
        match body {
            Body::ScheduleCreated(created) => {
                let chat = created.thread;
                let scheduled = Scheduled {
                    created,
                    at,
                    last: None,
                    skipped: None,
                    held: None,
                    paused: false,
                    paused_at: None,
                    resumed: None,
                    missed: None,
                };
                self.schedules.insert(scheduled.created.schedule, scheduled);
                chat
            }
            Body::ScheduleDeleted(deleted) => {
                let gone = self.schedules.remove(&deleted.schedule)?;
                if self.schedules.is_empty() {
                    self.emptied = Some(at);
                }
                gone.created.thread
            }
            Body::SchedulePaused(paused) => {
                let scheduled = self.schedules.get_mut(&paused.schedule)?;
                scheduled.pause(paused.paused, at);
                scheduled.created.thread
            }
            Body::ScheduleSkipped(skipped) => {
                let scheduled = self.schedules.get_mut(&skipped.schedule)?;
                scheduled.ran(skipped.due, Some(skipped.why));
                scheduled.created.thread
            }
            Body::TaskStarted(started) => {
                if let Some(fired) = started.schedule
                    && let Some(scheduled) = self.schedules.get_mut(&fired.schedule)
                {
                    scheduled.ran(fired.due, None);
                }
                // A schedule's run is a turn in its chat.
                let chat = started.schedule.map(|_| started.thread);
                let recorded = Recorded {
                    started,
                    at,
                    ended: None,
                };
                self.open.insert(recorded.started.thread, recorded);
                chat
            }
            Body::TaskEnded(ended) => {
                // A late end of a run before never closes the one its chat runs since.
                if !self.open.get(&ended.thread)?.ended_by(&ended) {
                    return None;
                }
                let mut recorded = self.open.remove(&ended.thread)?;
                let chat = recorded.started.schedule.map(|_| ended.thread);
                recorded.ended = Some((ended, at));
                if self.finished.len() >= FINISHED {
                    self.finished.pop_front();
                }
                self.finished.push_back(recorded);
                chat
            }
            _ => None,
        }
    }
}

/// What each project's ledger says, as far as this server has read it, by where the project
/// is: two organizations' projects never share one. At most [`KEPT`], the one read longest ago
/// let go of first, so the scheduler's own, read every tick, are kept and read on from where it
/// last read.
#[derive(Debug, Default)]
pub(crate) struct Ledgers {
    held: HashMap<Located, Ledger>,
    /// How many reads there have been, which orders them.
    reads: u64,
    /// Every ledger held, by when it was last read.
    by_read: BTreeMap<u64, Located>,
}

impl std::ops::Deref for Ledgers {
    type Target = HashMap<Located, Ledger>;

    fn deref(&self) -> &Self::Target {
        &self.held
    }
}

impl Ledgers {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// The ledger of the project `at`, empty until it is read, marked read now. A new one past
    /// [`KEPT`] forgets the one read longest ago.
    fn kept(&mut self, at: Located) -> &mut Ledger {
        if let Some(ledger) = self.held.get(&at) {
            self.by_read.remove(&ledger.read_at);
        } else if self.held.len() >= KEPT
            && let Some((_, &oldest)) = self.by_read.first_key_value()
        {
            self.forget(oldest);
        }
        self.reads = self.reads.saturating_add(1);
        let reads = self.reads;
        let ledger = self.held.entry(at).or_default();
        ledger.read_at = reads;
        self.by_read.insert(reads, at);
        ledger
    }

    /// The ledger of the project `at`, if read, to change what it holds in memory alone.
    pub(crate) fn get_mut(&mut self, at: &Located) -> Option<&mut Ledger> {
        self.held.get_mut(at)
    }

    /// Forgets the ledger of the project `at`, read again from its start when next asked for.
    pub(crate) fn forget(&mut self, at: Located) {
        if let Some(gone) = self.held.remove(&at) {
            self.by_read.remove(&gone.read_at);
        }
    }
}

/// The scope of the project `at` for glave's own work on its ledgers, which no person's
/// ownership decides: what a caller asks of them was checked against the project first.
pub(crate) fn own_scope(at: Located) -> Scope {
    Scope::glaves_own(at.org, at.project)
}

impl<P, L, B> Server<P, L, B>
where
    P: ModelProvider + 'static,
    L: EventLog + 'static,
    B: BlobStore + 'static,
{
    /// Reads whatever was added to the ledger of the project `at` since this server last looked.
    /// What was added since it was first read is told to the lists watching; what a first read
    /// finds, after a start or once the ledger was forgotten, is not news.
    pub(crate) async fn catch_up(&self, at: Located) -> Result<(), ApiError> {
        // Glave's own: the project's task ledger.
        let scope = own_scope(at);
        let (next, read_before) = (self.ledgers().get(&at))
            .map_or((1, false), |ledger| (ledger.next_seq(), ledger.read()));
        let after = Seq::new(next.saturating_sub(1)).ok();
        let log = self.inner.harness.log();
        let mut changed = Vec::new();
        catch_up(log, &scope, Reserved::Tasks.id(), after, |page| {
            let mut held = self.ledgers();
            let read = held.kept(at);
            for chat in page.into_iter().filter_map(|raw| read.apply(raw)) {
                if read_before {
                    changed.push(chat);
                }
            }
        })
        .await?;
        self.schedules_changed(at, changed);
        Ok(())
    }

    /// Tells the lists watching them that the chats of schedules in `at` that `changed` say
    /// something new of their schedules.
    fn schedules_changed(&self, at: Located, changed: Vec<ThreadId>) {
        for chat in changed {
            self.inner.hub.changed(at, chat);
        }
    }

    /// Appends `payload` to the ledger of the project `at`, after anything others wrote
    /// meanwhile.
    pub(crate) async fn record_in_ledger<T: Payload>(
        &self,
        at: Located,
        payload: &T,
    ) -> Result<(), ApiError> {
        self.record_unless(at, payload, |_| false).await.map(|_| ())
    }

    /// Appends `payload` to the ledger of the project `at`, after anything others wrote
    /// meanwhile, unless `there` says of the ledger, read up to then, that what it records is
    /// there already: of servers on one store appending it at once, the one whose append lands
    /// first is the only one that does. Says whether this one did.
    pub(crate) async fn record_unless<T: Payload>(
        &self,
        at: Located,
        payload: &T,
        there: impl Fn(&Ledger) -> bool,
    ) -> Result<bool, ApiError> {
        let ledger = Reserved::Tasks.id();
        // Glave's own: the project's task ledger.
        let scope = own_scope(at);
        let (this, there) = (self, &there);
        let log = self.inner.harness.log();
        let appended = append_retrying(log, &scope, ledger, move || async move {
            this.catch_up(at).await?;
            let next = {
                let ledgers = this.ledgers();
                let read = ledgers.get(&at);
                if read.is_some_and(there) {
                    return Ok(None);
                }
                read.map_or(1, Ledger::next_seq)
            };
            let now = this.inner.harness.clock().now();
            let header = Header {
                id: EventId::new(now, this.inner.harness.entropy().bytes()?)?,
                thread: ledger,
                seq: Seq::new(next)?,
                at: At::from(now),
            };
            let raw = encode(header, payload)?;
            Ok::<_, ApiError>(Some((vec![raw], ())))
        })
        .await?;
        let Some((events, ())) = appended else {
            return Ok(false);
        };
        // What this server just wrote is new, whenever the ledger was first read.
        let changed: Vec<ThreadId> = {
            let mut held = self.ledgers();
            let read = held.kept(at);
            events
                .into_iter()
                .filter_map(|raw| read.apply(raw))
                .collect()
        };
        self.schedules_changed(at, changed);
        Ok(true)
    }

    /// The projects whose tasks and schedules this server runs: every project its person works
    /// in, every one the store's index lists with schedules, whoever's they are, and any other
    /// whose ledger it has read for a caller.
    pub(crate) async fn task_projects(&self) -> Result<Vec<Located>, ApiError> {
        let mut projects = self.task_projects_here().await?;
        projects.extend(self.placed().await);
        let mut seen = HashSet::new();
        projects.retain(|project| seen.insert(*project));
        Ok(projects)
    }

    /// Of [`Server::task_projects`], those this server knows of itself: its person's, and any
    /// whose ledger it read for a caller.
    pub(crate) async fn task_projects_here(&self) -> Result<Vec<Located>, ApiError> {
        let mut projects = match self.inner.harness.person() {
            Some(_) => self.projects_of(&Caller::default()).await?,
            None => Vec::new(),
        };
        let known: Vec<Located> = self.ledgers().keys().copied().collect();
        projects.extend(known);
        let mut seen = HashSet::new();
        projects.retain(|project| seen.insert(*project));
        Ok(projects)
    }

    /// What the ledger of the project `at` says, read whole and apart from what this server
    /// keeps of ledgers, which may forget one midway through a read: for what must see every
    /// schedule there, as deleting them all does.
    pub(crate) async fn ledger_apart(&self, at: Located) -> Result<Ledger, ApiError> {
        let mut read = Ledger::default();
        let log = self.inner.harness.log();
        catch_up(log, &own_scope(at), Reserved::Tasks.id(), None, |page| {
            for raw in page {
                read.apply(raw);
            }
        })
        .await?;
        Ok(read)
    }

    pub(crate) fn ledgers(&self) -> std::sync::MutexGuard<'_, Ledgers> {
        self.inner
            .ledgers
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

#[cfg(test)]
mod tests;
