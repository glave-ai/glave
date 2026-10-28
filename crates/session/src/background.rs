//! A thread's commands in the background, for its clients (`specs/session.md`, "Commands in the
//! background"): `command/list` answers those running, and `command/stop` stops one for the
//! person who asks, whether a turn holds the thread or not. What either finds ended is recorded
//! at once when the thread is idle, else by its turn's next record. Each thread held here has
//! its commands watched, so one that ends however it ends (by itself, or killed by another
//! command) is recorded as it does, and its clients hear of it.

use std::collections::HashMap;
use std::sync::{Arc, PoisonError, Weak};

use glave_event::{Principal, ThreadId};
use glave_harness::Commands;
use glave_ports::computer::CommandId;
use glave_ports::{BlobStore, Caller, EventLog, ModelProvider};
use tokio::sync::watch;
use tokio::task::JoinHandle;

use crate::held::{Slot, Use, finished};
use crate::protocol::{ApiError, Reply, code};
use crate::server::{Inner, Server, no_thread};
use crate::words::Sentence;

/// The threads whose commands are watched, each with the record watched and its watcher.
pub(crate) type Watched = HashMap<ThreadId, Watching>;

/// One thread's commands watched: the watcher stops when this is dropped.
pub(crate) struct Watching {
    commands: Arc<Commands>,
    watcher: JoinHandle<()>,
}

impl Drop for Watching {
    fn drop(&mut self) {
        self.watcher.abort();
    }
}

impl<P, L, B> Server<P, L, B>
where
    P: ModelProvider + 'static,
    L: EventLog + 'static,
    B: BlobStore + 'static,
{
    /// The commands of `thread`, held here or read from the log.
    async fn commands_of(&self, thread: ThreadId) -> Result<Arc<Commands>, ApiError> {
        self.load(thread).await?;
        match self.lock().get(&thread) {
            Some(Slot::Idle { thread: held, .. }) => Ok(held.commands()),
            Some(Slot::Busy { commands, .. }) => Ok(Arc::clone(commands)),
            None => Err(no_thread(thread)),
        }
    }

    /// Answers `command/list`: none for a thread whose project's folder isn't there anymore,
    /// where nothing runs.
    pub(crate) async fn list_commands(&self, thread: ThreadId) -> Result<Reply, ApiError> {
        let commands = match self.commands_of(thread).await {
            Err(refused) if crate::gone::is_gone(&refused) => {
                return Ok(Reply::Commands {
                    commands: Vec::new(),
                });
            }
            commands => commands?,
        };
        let running = commands.running_now().await;
        self.record_commands(thread).await;
        Ok(Reply::Commands { commands: running })
    }

    /// Answers `command/stop`.
    pub(crate) async fn stop_command(
        &self,
        caller: &Caller,
        thread: ThreadId,
        command: CommandId,
    ) -> Result<Reply, ApiError> {
        let commands = self.commands_of(thread).await?;
        let by = Principal::person(self.person_of(caller)?);
        let stopped = commands.stop_for(command, by).await.map_err(|error| {
            ApiError::new(
                code::INTERNAL,
                Sentence::CommandNotStopped {
                    reason: error.message().to_owned(),
                },
            )
        })?;
        self.record_commands(thread).await;
        if stopped {
            Ok(Reply::Done {})
        } else {
            Err(ApiError::new(code::NO_COMMAND, Sentence::NoCommand))
        }
    }

    /// Records what the thread's commands are waiting to have recorded, when no turn holds it;
    /// a turn that does records it with its next event.
    async fn record_commands(&self, thread: ThreadId) {
        if !matches!(self.lock().get(&thread), Some(Slot::Idle { .. })) {
            return;
        }
        let (finish, mut done) = watch::channel(None);
        let occupied = self
            .occupy_for(
                thread,
                finish,
                Use::Manage,
                |inner, mut held, _| async move {
                    let recorded = inner.harness.record_commands(&mut held).await;
                    (held, recorded)
                },
            )
            .await;
        if occupied.is_ok() {
            let _ = finished(&mut done).await;
        }
    }

    /// Watches `commands`, those of `thread` held here, unless they already are; in place of
    /// any record of it held before.
    pub(crate) fn watch_commands(&self, thread: ThreadId, commands: &Arc<Commands>) {
        let mut watched = self.watched();
        if watched
            .get(&thread)
            .is_some_and(|watching| Arc::ptr_eq(&watching.commands, commands))
        {
            return;
        }
        let task = tokio::spawn(record_endings(
            Arc::downgrade(&self.inner),
            thread,
            Arc::clone(commands),
        ));
        watched.insert(
            thread,
            Watching {
                commands: Arc::clone(commands),
                watcher: task,
            },
        );
    }

    /// Stops watching `commands`, those of `thread`, let go of here.
    pub(crate) fn unwatch_commands(&self, thread: ThreadId, commands: &Arc<Commands>) {
        let mut watched = self.watched();
        if watched
            .get(&thread)
            .is_some_and(|watching| Arc::ptr_eq(&watching.commands, commands))
        {
            watched.remove(&thread);
        }
    }

    fn watched(&self) -> std::sync::MutexGuard<'_, Watched> {
        self.inner
            .watched
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }

    /// Records the end a watcher noted in `thread`: once the work holding it, if any, is done,
    /// since that work records it itself only with its next event.
    async fn record_ended(&self, thread: ThreadId) {
        let working = match self.lock().get(&thread) {
            Some(Slot::Busy { finished, .. }) => Some(finished.clone()),
            Some(Slot::Idle { .. }) => None,
            None => return,
        };
        if let Some(mut working) = working {
            let _ = working.wait_for(|done| *done).await;
        }
        self.record_commands(thread).await;
    }
}

/// Records each end of `commands`, those of `thread`, as it happens, for as long as the server
/// is there; dropped with its [`Watching`].
async fn record_endings<P, L, B>(
    inner: Weak<Inner<P, L, B>>,
    thread: ThreadId,
    commands: Arc<Commands>,
) where
    P: ModelProvider + 'static,
    L: EventLog + 'static,
    B: BlobStore + 'static,
{
    loop {
        commands.until_one_ends().await;
        let Some(inner) = inner.upgrade() else {
            return;
        };
        Server { inner }.record_ended(thread).await;
    }
}
