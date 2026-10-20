//! Commands kept running in the background on this machine (`specs/computers.md`, "Background
//! commands"). Each is watched by a task of its own, which this registry owns: it keeps the
//! command's unread output (the most recent [`glave_ports::computer::KEPT_OUTPUT`]), and stops its process
//! group whole when asked, at its time limit, or once nothing has read its output for
//! [`glave_ports::computer::IDLE`]; when the
//! command exits by itself, it stops whatever the command left running. Dropping the registry
//! drops every watcher, and with it each command's `Child`, which kills its group, so nothing
//! outlives this machine's computers.

mod unread;

use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use glave_ports::computer::{
    Background, CommandId, ComputerError, ComputerId, Ending, Finished, IDLE, MAX_BACKGROUND,
    MAX_WAIT, Printed, WorkspaceId,
};
use tokio::sync::{Notify, watch};
use tokio::task::JoinHandle;
use tokio::time::Instant;

use super::command::{DRAIN, Spawned, exited};
use unread::Unread;

/// The most commands this machine keeps running in the background, across its workspaces.
const MAX_RUNNING: usize = 64;
/// The most it keeps at all, ended ones waiting for their last read included; past it, the
/// oldest that ended is forgotten.
const MAX_KEPT: usize = 128;
/// How long a stopped command's watcher may take to finish: the grace before SIGKILL, twice,
/// and the last of its output.
const STOPPING: Duration = Duration::from_secs(10);

/// This machine's commands in the background.
pub(crate) struct Backgrounds {
    idle: Duration,
    held: Mutex<HashMap<CommandId, Entry>>,
}

impl std::fmt::Debug for Backgrounds {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Backgrounds").finish_non_exhaustive()
    }
}

impl Default for Backgrounds {
    fn default() -> Self {
        Self::new(IDLE)
    }
}

/// One command in the background.
struct Entry {
    computer: ComputerId,
    workspace: WorkspaceId,
    script: String,
    started: Instant,
    shared: Arc<Shared>,
    watcher: JoinHandle<()>,
}

/// What a command's watcher and its readers share.
struct Shared {
    unread: Mutex<Unread>,
    /// When its output was last read, or it started.
    read_at: Mutex<Instant>,
    ended: watch::Sender<Option<Ended>>,
    stop: Notify,
}

/// How a command ended, and when.
#[derive(Clone, Copy)]
struct Ended {
    finished: Finished,
    at: Instant,
}

impl Shared {
    fn idle_at(&self, idle: Duration) -> Instant {
        let read_at = *self.read_at.lock().unwrap_or_else(PoisonError::into_inner);
        read_at + idle
    }

    fn touch(&self) {
        *self.read_at.lock().unwrap_or_else(PoisonError::into_inner) = Instant::now();
    }

    fn push(&self, text: &str) {
        if !text.is_empty() {
            let mut unread = self.unread.lock().unwrap_or_else(PoisonError::into_inner);
            unread.push(text);
        }
    }

    fn ended(&self) -> Option<Ended> {
        *self.ended.borrow()
    }

    /// What is unread, and how it ended if it has: the ending first, so output pushed before
    /// it is never left behind.
    fn take(&self) -> Printed {
        let ended = self.ended().map(|ended| ended.finished);
        let (text, dropped) = self
            .unread
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
        Printed {
            text,
            dropped,
            ended,
        }
    }

    async fn until_ended(&self) {
        let mut ended = self.ended.subscribe();
        let _ = ended.wait_for(Option::is_some).await;
    }
}

impl Backgrounds {
    /// No commands yet; each stopped after `idle` with nothing reading it.
    pub(crate) fn new(idle: Duration) -> Self {
        Self {
            idle,
            held: Mutex::new(HashMap::new()),
        }
    }

    fn held(&self) -> std::sync::MutexGuard<'_, HashMap<CommandId, Entry>> {
        self.held.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Refuses one more in `workspace` when it, or this machine, keeps as many as it may.
    pub(crate) fn room(&self, workspace: WorkspaceId) -> Result<(), ComputerError> {
        full(&self.held(), workspace).map_or(Ok(()), Err)
    }

    /// Keeps `spawned` running `script` in the background as `id`, in `workspace` on
    /// `computer`, until `limit` when there is one; hands it back when there is no room for it.
    pub(crate) fn adopt(
        &self,
        (id, script): (CommandId, String),
        (computer, workspace): (ComputerId, WorkspaceId),
        spawned: Spawned,
        limit: Option<Duration>,
    ) -> Result<CommandId, Box<Spawned>> {
        let mut held = self.held();
        prune(&mut held);
        if full(&held, workspace).is_some() {
            return Err(Box::new(spawned));
        }
        let started = spawned.started;
        let (ended, _) = watch::channel(None);
        let shared = Arc::new(Shared {
            unread: Mutex::new(Unread::default()),
            read_at: Mutex::new(Instant::now()),
            ended,
            stop: Notify::new(),
        });
        let watcher = tokio::spawn(watch_over(
            spawned,
            Arc::clone(&shared),
            self.idle,
            limit.map(|limit| started + limit),
        ));
        held.insert(
            id,
            Entry {
                computer,
                workspace,
                script,
                started,
                shared,
                watcher,
            },
        );
        Ok(id)
    }

    /// The command `command` in `workspace`, or not found: one in another workspace is not
    /// reached from this one.
    fn find(
        &self,
        workspace: WorkspaceId,
        command: CommandId,
    ) -> Result<Arc<Shared>, ComputerError> {
        self.held()
            .get(&command)
            .filter(|entry| entry.workspace == workspace)
            .map(|entry| Arc::clone(&entry.shared))
            .ok_or_else(|| {
                ComputerError::NotFound(format!("no command {command} in the background here"))
            })
    }

    /// Forgets `command`, whose watcher is done or finishing.
    fn forget(&self, command: CommandId) {
        if let Some(entry) = self.held().remove(&command) {
            entry.watcher.abort();
        }
    }

    /// What `command` printed since the last read, waiting up to `wait` for it to end first;
    /// forgotten once a read answers its ending. Dropped while it waits, it takes nothing.
    pub(crate) async fn output(
        &self,
        workspace: WorkspaceId,
        command: CommandId,
        wait: Duration,
    ) -> Result<Printed, ComputerError> {
        let shared = self.find(workspace, command)?;
        shared.touch();
        let _ = tokio::time::timeout(wait.min(MAX_WAIT), shared.until_ended()).await;
        let printed = shared.take();
        if printed.ended.is_some() {
            self.forget(command);
        }
        Ok(printed)
    }

    /// Stops `command` and everything it started, then forgets it.
    pub(crate) async fn stop(
        &self,
        workspace: WorkspaceId,
        command: CommandId,
    ) -> Result<Printed, ComputerError> {
        let shared = self.find(workspace, command)?;
        shared.stop.notify_one();
        if tokio::time::timeout(STOPPING, shared.until_ended())
            .await
            .is_err()
        {
            return Err(ComputerError::Failed(format!(
                "the command {command} did not stop within {} s",
                STOPPING.as_secs()
            )));
        }
        let printed = shared.take();
        self.forget(command);
        Ok(printed)
    }

    /// The commands in `workspace`, oldest first.
    pub(crate) fn list(&self, workspace: WorkspaceId) -> Vec<Background> {
        let mut held = self.held();
        prune(&mut held);
        let mut listed: Vec<(Instant, Background)> = held
            .iter()
            .filter(|(_, entry)| entry.workspace == workspace)
            .map(|(id, entry)| {
                let ended = entry.shared.ended();
                let millis = ended.map_or_else(
                    || millis(entry.started.elapsed()),
                    |ended| ended.finished.millis,
                );
                let listed = Background {
                    id: *id,
                    script: entry.script.clone(),
                    millis,
                    ended: ended.map(|ended| ended.finished.ending),
                };
                (entry.started, listed)
            })
            .collect();
        listed.sort_by_key(|(started, _)| *started);
        listed.into_iter().map(|(_, listed)| listed).collect()
    }

    /// Stops every command `which` picks by its computer and workspace, then forgets them: a
    /// workspace closing, or a computer.
    pub(crate) async fn stop_all(&self, which: impl Fn(ComputerId, WorkspaceId) -> bool) {
        let picked: Vec<(WorkspaceId, CommandId)> = self
            .held()
            .iter()
            .filter(|(_, entry)| which(entry.computer, entry.workspace))
            .map(|(id, entry)| (entry.workspace, *id))
            .collect();
        for (workspace, command) in picked {
            let _ = self.stop(workspace, command).await;
        }
    }
}

impl Drop for Backgrounds {
    fn drop(&mut self) {
        let held = self.held.get_mut().unwrap_or_else(PoisonError::into_inner);
        #[expect(
            clippy::iter_over_hash_type,
            reason = "every watcher is aborted, so the order is never seen"
        )]
        for entry in held.values() {
            entry.watcher.abort();
        }
    }
}

/// Why `workspace` may not start one more, if it may not.
fn full(held: &HashMap<CommandId, Entry>, workspace: WorkspaceId) -> Option<ComputerError> {
    let running = || held.values().filter(|entry| entry.shared.ended().is_none());
    if running()
        .filter(|entry| entry.workspace == workspace)
        .count()
        >= MAX_BACKGROUND
    {
        return Some(ComputerError::Refused(format!(
            "{MAX_BACKGROUND} commands are already running in the background here; stop one first"
        )));
    }
    if running().count() >= MAX_RUNNING {
        return Some(ComputerError::Refused(format!(
            "this computer already keeps {MAX_RUNNING} commands running in the background; \
             stop one first"
        )));
    }
    None
}

/// Forgets what ended [`IDLE`] ago and was never read, and past [`MAX_KEPT`] the oldest that
/// ended.
fn prune(held: &mut HashMap<CommandId, Entry>) {
    let forgotten = |entry: &Entry| {
        entry
            .shared
            .ended()
            .is_some_and(|ended| ended.at.elapsed() >= IDLE)
    };
    held.retain(|_, entry| {
        let keep = !forgotten(entry);
        if !keep {
            entry.watcher.abort();
        }
        keep
    });
    while held.len() > MAX_KEPT {
        let oldest = held
            .iter()
            .filter_map(|(id, entry)| entry.shared.ended().map(|ended| (ended.at, *id)))
            .min();
        let Some(entry) = oldest.and_then(|(_, id)| held.remove(&id)) else {
            break;
        };
        entry.watcher.abort();
    }
}

fn millis(elapsed: Duration) -> u64 {
    u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX)
}

/// Watches one command until it ends: keeps its output for readers, and stops it when asked,
/// once nothing has read it for `idle`, or at `deadline`.
async fn watch_over(
    spawned: Spawned,
    shared: Arc<Shared>,
    idle: Duration,
    deadline: Option<Instant>,
) {
    let Spawned {
        mut child,
        mut stream,
        held,
        started,
    } = spawned;
    let mut open = true;
    let ending = loop {
        let idle_at = shared.idle_at(idle);
        tokio::select! {
            status = child.wait() => break status.map_or(Ending::Signalled, exited),
            more = stream.read(), if open => {
                open = more;
                shared.push(&stream.take());
            }
            () = shared.stop.notified() => {
                child.stop().await;
                break Ending::Cancelled;
            }
            () = tokio::time::sleep_until(idle_at) => {
                if shared.idle_at(idle) <= Instant::now() {
                    child.stop().await;
                    break Ending::Idle { after_secs: idle.as_secs() };
                }
            }
            () = until(deadline) => {
                child.stop().await;
                let after = deadline.map_or(Duration::ZERO, |at| at.duration_since(started));
                break Ending::TimedOut { after_secs: after.as_secs() };
            }
        }
    };
    // The last of its output: what a child it left running still holds open gets a moment.
    let rest = async {
        while open {
            open = stream.read().await;
            shared.push(&stream.take());
        }
    };
    let _ = tokio::time::timeout(DRAIN, rest).await;
    shared.push(&stream.take());
    // What it left running goes with it.
    child.stop_left().await;
    drop((child, held));
    let finished = Finished {
        ending,
        millis: millis(started.elapsed()),
    };
    shared.ended.send_replace(Some(Ended {
        finished,
        at: Instant::now(),
    }));
}

/// Resolves at `deadline`, or never without one.
async fn until(deadline: Option<Instant>) {
    match deadline {
        Some(deadline) => tokio::time::sleep_until(deadline).await,
        None => std::future::pending().await,
    }
}

#[cfg(test)]
mod tests;
