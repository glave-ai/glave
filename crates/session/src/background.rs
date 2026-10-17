//! A thread's commands in the background, for its clients (`specs/session.md`, "Commands in the
//! background"): `command/list` answers those running, and `command/stop` stops one for the
//! person who asks, whether a turn holds the thread or not. What either finds ended is recorded
//! at once when the thread is idle, else by its turn's next record.

use std::sync::Arc;

use glave_event::{Principal, ThreadId};
use glave_harness::{Caller, Commands};
use glave_ports::{BlobStore, CommandId, EventLog, ModelProvider};
use tokio::sync::watch;

use crate::held::{Slot, Use, finished};
use crate::protocol::{ApiError, Reply, code};
use crate::server::{Server, no_thread};

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
            Some(Slot::Idle(held)) => Ok(held.commands()),
            Some(Slot::Busy { commands, .. }) => Ok(Arc::clone(commands)),
            None => Err(no_thread(thread)),
        }
    }

    /// Answers `command/list`.
    pub(crate) async fn list_commands(&self, thread: ThreadId) -> Result<Reply, ApiError> {
        let commands = self.commands_of(thread).await?;
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
                format!("the command could not be stopped: {}", error.message()),
            )
        })?;
        self.record_commands(thread).await;
        if stopped {
            Ok(Reply::Done {})
        } else {
            Err(ApiError::new(
                code::NO_COMMAND,
                "that command is not running in this thread",
            ))
        }
    }

    /// Records what the thread's commands are waiting to have recorded, when no turn holds it;
    /// a turn that does records it with its next event.
    async fn record_commands(&self, thread: ThreadId) {
        if !matches!(self.lock().get(&thread), Some(Slot::Idle(_))) {
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
}
