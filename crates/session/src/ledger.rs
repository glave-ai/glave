//! Each project's task ledger: glave's own stream in the event log, under a reserved thread id,
//! holding the project's tasks (`task.started`, `task.ended`) and schedules (`schedule.created`,
//! `schedule.deleted`). It is never a conversation: lists and reads of threads leave it out.
//! Any glave process of the deployment may append to it; an append that meets another's
//! `(thread, seq)` conflict reads what was written and tries again after it.

use std::collections::{BTreeMap, HashMap, VecDeque};

use glave_event::{
    At, Body, EventId, Header, Payload, RawEvent, ScheduleCreated, ScheduleId, Seq, TaskEnded,
    TaskStarted, ThreadId, decode, encode,
};
use glave_ports::stream::{Reserved, append_retrying, catch_up};
use glave_ports::{BlobStore, Caller, EventLog, ModelProvider, ProjectId, Scope};

use crate::protocol::ApiError;
use crate::server::Server;

/// Finished tasks kept in memory for `tasks/list`, newest last, in each project.
const FINISHED: usize = 100;
/// Projects whose ledgers a server keeps in memory at once.
const LEDGERS: usize = 1_024;

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
        let Some((at, body)) = in_order(&mut self.next, raw) else {
            return;
        };
        match body {
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

/// `raw` when it is the event a stream read so far takes next, after `next` (0 before the
/// first), and when it was written: `next` then moves past it. Any other is already in, or not
/// yet; one this reader cannot read is passed over.
pub(crate) fn in_order(next: &mut u64, raw: RawEvent) -> Option<(At, Body)> {
    if raw.seq.get() != (*next).max(1) {
        return None;
    }
    *next = (*next).max(1).saturating_add(1);
    let at = raw.at;
    decode(raw).ok().map(|event| (at, event.body))
}

/// What each project's ledger says, as far as this server has read it.
pub(crate) type Ledgers = HashMap<ProjectId, Ledger>;

/// `project`'s ledger in `ledgers`, empty until it is read. Past [`LEDGERS`] projects, the others
/// are forgotten and read again when next asked for.
fn kept(ledgers: &mut Ledgers, project: ProjectId) -> &mut Ledger {
    if ledgers.len() >= LEDGERS && !ledgers.contains_key(&project) {
        ledgers.clear();
    }
    ledgers.entry(project).or_default()
}

impl<P, L, B> Server<P, L, B>
where
    P: ModelProvider + 'static,
    L: EventLog + 'static,
    B: BlobStore + 'static,
{
    /// Reads whatever was added to `project`'s ledger since this server last looked.
    pub(crate) async fn catch_up(&self, project: ProjectId) -> Result<(), ApiError> {
        // Glave's own: the project's task ledger.
        let scope = Scope::glaves_own(self.inner.org, project);
        let next = self.ledgers().get(&project).map_or(1, Ledger::next_seq);
        let after = Seq::new(next.saturating_sub(1)).ok();
        let log = self.inner.harness.log();
        catch_up(log, &scope, Reserved::Tasks.id(), after, |page| {
            let mut held = self.ledgers();
            let read = kept(&mut held, project);
            for raw in page {
                read.apply(raw);
            }
        })
        .await?;
        Ok(())
    }

    /// Appends `payload` to `project`'s ledger, after anything others wrote meanwhile.
    pub(crate) async fn record_in_ledger<T: Payload>(
        &self,
        project: ProjectId,
        payload: &T,
    ) -> Result<(), ApiError> {
        let ledger = Reserved::Tasks.id();
        // Glave's own: the project's task ledger.
        let scope = Scope::glaves_own(self.inner.org, project);
        let this = self;
        let log = self.inner.harness.log();
        let appended = append_retrying(log, &scope, ledger, move || async move {
            this.catch_up(project).await?;
            let now = this.inner.harness.clock().now();
            let next = this.ledgers().get(&project).map_or(1, Ledger::next_seq);
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
        let mut held = self.ledgers();
        let read = kept(&mut held, project);
        for raw in appended.into_iter().flat_map(|(events, ())| events) {
            read.apply(raw);
        }
        Ok(())
    }

    /// The projects whose tasks and schedules this server runs: every project its person works
    /// in, and any other whose ledger it has read for a caller.
    pub(crate) async fn task_projects(&self) -> Result<Vec<ProjectId>, ApiError> {
        let mut projects = match self.inner.harness.person() {
            Some(_) => self.projects_of(&Caller::default()).await?,
            None => Vec::new(),
        };
        let known: Vec<ProjectId> = self.ledgers().keys().copied().collect();
        for project in known {
            if !projects.contains(&project) {
                projects.push(project);
            }
        }
        Ok(projects)
    }

    pub(crate) fn ledgers(&self) -> std::sync::MutexGuard<'_, Ledgers> {
        self.inner
            .ledgers
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}
