//! A thread's commands in the background (`specs/computers.md`, "Background commands"): each
//! one's number, as the model names it, the computer and workspace it runs in, and how it ended
//! once glave knows. The thread and the tools running in it share one record; what changes in
//! it waits there to be recorded as `command.started` and `command.ended`, glave's own records,
//! the next time the harness writes to the thread. Rebuilt from those events when a thread
//! resumes, and checked against the computers then and at each turn's start, so the thread
//! knows what still runs, and what ended while glave was away.

mod record;

use std::sync::Arc;

use glave_event::{Body, CommandEnding, Event, Principal};
use glave_ports::computer::{
    CommandId, ComputerError, Computers, Ending, MAX_WAIT, Printed, WorkspaceId,
};
use glave_ports::{BlobStore, EventLog, ModelProvider};
use tokio::task::JoinSet;

use crate::approval::Approver;
use crate::harness::{Harness, HarnessError, Thread};

pub use record::{Commands, RunningCommand};
pub(crate) use record::{Entry, ending_of};

/// What a batch staged of a thread's command changes: how many, and when each command it
/// records as started started.
#[derive(Debug, Default)]
#[must_use = "the changes are recorded only once `recorded` says so"]
pub(crate) struct Staged {
    count: usize,
    started: Vec<(u32, glave_event::At)>,
}

impl Staged {
    /// Notes that the batch these changes were staged in was written: they wait no more, and
    /// count among glave's own records.
    pub(crate) fn recorded(self, thread: &mut Thread) {
        thread.commands.recorded(self.count);
        for (number, at) in self.started {
            thread.commands.started_at(number, at);
        }
        let count = u64::try_from(self.count).unwrap_or(u64::MAX);
        thread.unnumbered = thread.unnumbered.saturating_add(count);
    }
}

impl Commands {
    /// The record `events` leave, its commands reached through `computers`.
    pub(crate) fn rebuilt(events: &[Event], computers: &Arc<dyn Computers>) -> Self {
        let commands = Self::default();
        for event in events {
            match &event.body {
                Body::CommandStarted(started) => commands.restore(
                    CommandId::from_uuid(started.command),
                    WorkspaceId::from_uuid(started.workspace),
                    Arc::clone(computers),
                    started.script.clone(),
                    event.header.at,
                ),
                Body::CommandEnded(ended) => {
                    commands.restore_end(CommandId::from_uuid(ended.command), &ended.ending);
                }
                _ => {}
            }
        }
        commands
    }

    /// What `entry` printed since the last read, waiting up to `wait` for it to end; recorded
    /// as ended once it has, or once its computer no longer has it.
    pub(crate) async fn read(
        &self,
        entry: &Entry,
        wait: std::time::Duration,
    ) -> Result<Printed, ComputerError> {
        let read = entry
            .computers
            .output(entry.workspace, entry.id, wait)
            .await;
        self.settle(entry, read)
    }

    /// Stops `entry` and everything it started, for `by` (the thread's agent when `None`),
    /// answering what it printed last.
    pub(crate) async fn stop_entry(
        &self,
        entry: &Entry,
        by: Option<Principal>,
    ) -> Result<Printed, ComputerError> {
        let stopped = entry.computers.stop(entry.workspace, entry.id).await;
        if let Ok(printed) = &stopped
            && printed
                .ended
                .is_some_and(|ended| ended.ending == Ending::Cancelled)
        {
            let millis = printed.ended.map(|ended| ended.millis);
            self.end(entry.id, CommandEnding::Stopped { by }, millis);
            return stopped;
        }
        self.settle(entry, stopped)
    }

    /// Records what a read or a stop of `entry` showed: how it ended, or that it is gone.
    fn settle(
        &self,
        entry: &Entry,
        read: Result<Printed, ComputerError>,
    ) -> Result<Printed, ComputerError> {
        match &read {
            Ok(printed) => {
                if let Some(ended) = printed.ended {
                    self.end(entry.id, ending_of(ended.ending), Some(ended.millis));
                }
            }
            Err(ComputerError::NotFound(_)) => self.end(entry.id, CommandEnding::Gone, None),
            Err(_) => {}
        }
        read
    }

    /// Asks the computers how the commands still running are, and notes each one that ended,
    /// or that its computer no longer has. A computer that cannot be reached changes nothing.
    pub(crate) async fn check(&self) {
        let running = self.running();
        let mut workspaces: Vec<(WorkspaceId, Arc<dyn Computers>)> = Vec::new();
        for entry in &running {
            if !workspaces.iter().any(|(at, _)| *at == entry.workspace) {
                workspaces.push((entry.workspace, Arc::clone(&entry.computers)));
            }
        }
        for (workspace, computers) in workspaces {
            let listed = match computers.commands(workspace).await {
                Ok(listed) => listed,
                Err(ComputerError::NotFound(_)) => Vec::new(),
                Err(_) => continue,
            };
            for entry in running.iter().filter(|entry| entry.workspace == workspace) {
                match listed.iter().find(|listed| listed.id == entry.id) {
                    Some(listed) => {
                        if let Some(ending) = listed.ended {
                            self.end(entry.id, ending_of(ending), Some(listed.millis));
                        }
                    }
                    None => self.end(entry.id, CommandEnding::Gone, None),
                }
            }
        }
    }

    /// Waits until one of the commands running ends, however it ends (by itself, or killed by
    /// another command), and notes how, so the thread records it at once rather than at its
    /// next turn; as Codex's exit watcher does for each of its sessions. One stopped through
    /// glave is noted by whoever stopped it. Waits on while none runs, until one starts; it
    /// runs until it is dropped, which its holder does when the thread is let go.
    pub async fn until_one_ends(&self) {
        loop {
            let mut kept = self.watch_kept();
            let mut waits = JoinSet::new();
            for entry in self.running() {
                waits.spawn(ending_of_entry(entry));
            }
            loop {
                tokio::select! {
                    // Another started: wait for it too.
                    _ = kept.changed() => break,
                    Some(joined) = waits.join_next() => {
                        if let Ok(Some((id, ending, millis))) = joined {
                            self.end(id, ending, millis);
                            return;
                        }
                    }
                }
            }
        }
    }

    /// Stops every command still running, as glave's own decision: its thread was put away,
    /// or its subagent is done.
    pub(crate) async fn release(&self) {
        for entry in self.running() {
            match entry.computers.stop(entry.workspace, entry.id).await {
                // It had ended by itself first: that is how it ended.
                Ok(Printed {
                    ended: Some(ended), ..
                }) if ended.ending != Ending::Cancelled => {
                    self.end(entry.id, ending_of(ended.ending), Some(ended.millis));
                }
                Ok(printed) => {
                    let millis = printed.ended.map(|ended| ended.millis);
                    self.end(entry.id, CommandEnding::Released, millis);
                }
                Err(ComputerError::NotFound(_)) => self.end(entry.id, CommandEnding::Gone, None),
                Err(_) => {}
            }
        }
    }

    /// The commands running now, after asking their computers.
    pub async fn running_now(&self) -> Vec<RunningCommand> {
        self.check().await;
        self.running()
            .into_iter()
            .map(|entry| RunningCommand {
                id: entry.id,
                command: entry.script,
                started_at: entry.started,
                summary: record::RUNNING.to_owned(),
            })
            .collect()
    }

    /// Stops the command `id` for `by`, the person who asked; false when it is not running.
    pub async fn stop_for(&self, id: CommandId, by: Principal) -> Result<bool, ComputerError> {
        let Some(entry) = self.running().into_iter().find(|entry| entry.id == id) else {
            return Ok(false);
        };
        self.stop_entry(&entry, Some(by)).await?;
        Ok(true)
    }
}

/// How `entry` ends, as its computer tells it: nothing when it was stopped through glave,
/// whose stopper notes it, or when its computer cannot tell, which the next turn's check does.
async fn ending_of_entry(entry: Entry) -> Option<(CommandId, CommandEnding, Option<u64>)> {
    loop {
        match entry
            .computers
            .ended(entry.workspace, entry.id, MAX_WAIT)
            .await
        {
            Ok(None) => {}
            Ok(Some(finished)) if finished.ending == Ending::Cancelled => return None,
            Ok(Some(finished)) => {
                return Some((entry.id, ending_of(finished.ending), Some(finished.millis)));
            }
            Err(ComputerError::NotFound(_)) => return Some((entry.id, CommandEnding::Gone, None)),
            Err(_) => return None,
        }
    }
}

impl<P, L, B, A> Harness<P, L, B, A>
where
    P: ModelProvider,
    L: EventLog,
    B: BlobStore,
    A: Approver,
{
    /// Stages what the thread's commands are waiting to have recorded, in order, as glave's
    /// own records, which call ids do not count. They wait still until the batch is written
    /// ([`Staged::recorded`]), so one that is not loses none.
    pub(crate) fn stage_commands(
        &self,
        thread: &Thread,
        events: &mut Vec<glave_event::RawEvent>,
    ) -> Result<Staged, HarnessError> {
        let mut staged = Staged::default();
        for change in thread.commands.unrecorded() {
            match change {
                record::Change::Started(number, started) => {
                    self.stage(thread, events, &started)?;
                    if let Some(at) = events.last().map(|event| event.at) {
                        staged.started.push((number, at));
                    }
                }
                record::Change::Ended(ended) => self.stage(thread, events, &ended)?,
            }
            staged.count = staged.count.saturating_add(1);
        }
        Ok(staged)
    }

    /// Records what the thread's commands are waiting to have recorded, now.
    pub async fn record_commands(&self, thread: &mut Thread) -> Result<(), HarnessError> {
        let mut events = Vec::new();
        let staged = self.stage_commands(thread, &mut events)?;
        self.commit(thread, events).await?;
        staged.recorded(thread);
        Ok(())
    }

    /// Asks the computers how the thread's commands in the background are, and records what
    /// ended meanwhile: as a turn starts, and as a thread resumes.
    pub(crate) async fn check_commands(&self, thread: &mut Thread) -> Result<(), HarnessError> {
        if thread.commands.any_running() {
            thread.commands.check().await;
        }
        self.record_commands(thread).await
    }

    /// Stops the thread's commands still running, recording why, as glave's own decision.
    pub(crate) async fn release_commands(&self, thread: &mut Thread) -> Result<(), HarnessError> {
        if thread.commands.any_running() {
            thread.commands.release().await;
        }
        self.record_commands(thread).await
    }
}
