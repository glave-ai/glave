//! A person's terminals in a thread's folder (`specs/session.md`, "Terminals"): their own shell,
//! on their own computer, outside the agent's sandbox, opened, typed into, read, resized and
//! closed for them alone. Nothing of it is recorded: the event log, the model and the server's
//! log never hold a byte typed or printed. The server keeps nothing of them either: each call
//! finds the thread's computer from its binding, as opening one does, and the computer keeps
//! which thread's each terminal is, so a restart, or another server of the store, finds them.
//!
//! Who may open one: only the person whose computer it is. On the server's own computer, the
//! person the server works for (as only they leave the sandbox there, `specs/sandbox.md`,
//! "Whose computer"); where people have computers of their own, each on theirs. Never on a
//! worker, which holds a task's credential.

use std::sync::Arc;
use std::time::Duration;

use glave_event::{Body, ComputerKind, Payload as _, ThreadId, WorkspaceBound};
use glave_harness::owner_label;
use glave_ports::computer::{
    Bytes, ComputerError, ComputerId, Computers, TerminalId, TerminalSize,
};
use glave_ports::{BlobStore, Caller, EventLog, ModelProvider, Standing};

use crate::protocol::{ApiError, Reply, code};
use crate::server::Server;
use crate::words::Sentence;

/// The longest one read waits for something to read: under the quiet a proxy allows a request.
const MOST_WAIT: Duration = Duration::from_secs(25);

/// Where a thread's terminals are: its folder, on a computer of the caller's own, and whose
/// they are there.
struct Place {
    folder: String,
    owner: String,
    computers: Arc<dyn Computers>,
    computer: ComputerId,
}

impl<P, L, B> Server<P, L, B>
where
    P: ModelProvider + 'static,
    L: EventLog + 'static,
    B: BlobStore + 'static,
{
    /// Whether `caller` may open terminals here: the person the server works for, or anyone
    /// where people have computers of their own.
    pub(crate) fn opens_terminals(&self, caller: &Caller) -> bool {
        self.inner.homes.is_some() || self.is_theirs(caller)
    }

    /// Answers `terminal/open`: refused in a thread put away, and closed again when it was put
    /// away as this opened it.
    pub(crate) async fn open_terminal(
        &self,
        caller: &Caller,
        thread: ThreadId,
        size: TerminalSize,
    ) -> Result<Reply, ApiError> {
        self.shown(thread).await?;
        let place = self.terminal_place(caller, thread).await?;
        let (computers, computer) = (&place.computers, place.computer);
        let opened = computers.open_terminal(computer, &place.folder, &place.owner, size);
        let id = opened.await.map_err(|error| match error {
            // Its folder isn't there anymore.
            ComputerError::NotFound(_) => {
                ApiError::new(code::NOT_AVAILABLE, Sentence::TerminalNoFolder)
            }
            other => refused(other),
        })?;
        if let Err(away) = self.shown(thread).await {
            let _ = computers.close_terminal(computer, id).await;
            return Err(away);
        }
        let listed = computers
            .terminals(computer, &place.owner)
            .await
            .map_err(refused)?;
        let terminal = listed
            .into_iter()
            .find(|each| each.id == id)
            .ok_or_else(no_terminal)?;
        Ok(Reply::Terminal { terminal })
    }

    /// Answers `terminal/list`, as the thread's computer keeps them.
    pub(crate) async fn list_terminals(
        &self,
        caller: &Caller,
        thread: ThreadId,
    ) -> Result<Reply, ApiError> {
        let place = self.terminal_place(caller, thread).await?;
        let terminals = place
            .computers
            .terminals(place.computer, &place.owner)
            .await;
        Ok(Reply::Terminals {
            terminals: terminals.map_err(refused)?,
        })
    }

    /// Answers `terminal/write`.
    pub(crate) async fn write_terminal(
        &self,
        caller: &Caller,
        thread: ThreadId,
        (terminal, input): (TerminalId, &Bytes),
    ) -> Result<Reply, ApiError> {
        let place = self.terminal_place(caller, thread).await?;
        let written = place
            .computers
            .write_terminal(place.computer, terminal, input);
        written.await.map_err(refused)?;
        Ok(Reply::Done {})
    }

    /// Answers `terminal/read`.
    pub(crate) async fn read_terminal(
        &self,
        caller: &Caller,
        thread: ThreadId,
        (terminal, from): (TerminalId, u64),
        wait: Duration,
    ) -> Result<Reply, ApiError> {
        let place = self.terminal_place(caller, thread).await?;
        let wait = wait.min(MOST_WAIT);
        let read = place
            .computers
            .read_terminal(place.computer, terminal, from, wait);
        Ok(Reply::Screen {
            screen: read.await.map_err(refused)?,
        })
    }

    /// Answers `terminal/resize`.
    pub(crate) async fn resize_terminal(
        &self,
        caller: &Caller,
        thread: ThreadId,
        (terminal, size): (TerminalId, TerminalSize),
    ) -> Result<Reply, ApiError> {
        let place = self.terminal_place(caller, thread).await?;
        let resized = place
            .computers
            .resize_terminal(place.computer, terminal, size);
        resized.await.map_err(refused)?;
        Ok(Reply::Done {})
    }

    /// Answers `terminal/close`.
    pub(crate) async fn close_terminal(
        &self,
        caller: &Caller,
        thread: ThreadId,
        terminal: TerminalId,
    ) -> Result<Reply, ApiError> {
        let place = self.terminal_place(caller, thread).await?;
        let closed = place.computers.close_terminal(place.computer, terminal);
        closed.await.map_err(refused)?;
        Ok(Reply::Done {})
    }

    /// Closes every terminal open in `thread`, as it is archived or deleted: those its computer
    /// keeps for it, wherever it is reached from.
    pub(crate) async fn close_terminals(&self, caller: &Caller, thread: ThreadId) {
        let Ok(place) = self.terminal_place(caller, thread).await else {
            return;
        };
        let Ok(open) = place
            .computers
            .terminals(place.computer, &place.owner)
            .await
        else {
            return;
        };
        for terminal in open {
            let closed = place
                .computers
                .close_terminal(place.computer, terminal.id)
                .await;
            if let Err(error) = closed {
                tracing::warn!(%error, %thread, "could not close a terminal");
            }
        }
    }

    /// Refuses a thread that is not shown: archived, as it says, or deleted, as no thread.
    async fn shown(&self, thread: ThreadId) -> Result<(), ApiError> {
        match self.standing(thread).await? {
            Standing::Active => Ok(()),
            Standing::Archived => Err(ApiError::new(
                code::ARCHIVED,
                Sentence::Archived {
                    thread: thread.to_string(),
                },
            )),
            Standing::Deleted => Err(crate::server::no_thread(thread)),
        }
    }

    /// Where `thread`'s terminals are for `caller`: the folder its latest binding records, on
    /// the computer it records when that computer is the caller's own and this server reaches
    /// it.
    async fn terminal_place(&self, caller: &Caller, thread: ThreadId) -> Result<Place, ApiError> {
        let scope = self.scope_of(thread)?;
        let latest = self.latest(&scope, thread, WorkspaceBound::TYPE).await?;
        let Some(Body::WorkspaceBound(bound)) = latest else {
            return Err(ApiError::new(
                code::NOT_AVAILABLE,
                Sentence::TerminalNoFolder,
            ));
        };
        let (computers, computer) = match bound.computer.kind {
            ComputerKind::Local if self.is_theirs(caller) => {
                let here = self.inner.harness.workspace();
                (Arc::clone(here.computers()), here.info().computer)
            }
            ComputerKind::Local => {
                return Err(ApiError::new(
                    code::NOT_ALLOWED,
                    Sentence::TerminalNotTheirs,
                ));
            }
            ComputerKind::Person => {
                let person = self.person_of(caller)?;
                let home = self.inner.harness.own_home(scope.org(), person);
                let no_home = || ApiError::new(code::NOT_AVAILABLE, Sentence::TerminalNoHome);
                let home = home.ok_or_else(no_home)?.map_err(|_| no_home())?;
                (Arc::clone(home.computers()), home.info().computer)
            }
            ComputerKind::Worker => {
                return Err(ApiError::new(
                    code::NOT_AVAILABLE,
                    Sentence::TerminalOnWorker,
                ));
            }
        };
        if computer.as_uuid() != bound.computer.id {
            return Err(ApiError::new(code::NOT_AVAILABLE, Sentence::TerminalMoved));
        }
        Ok(Place {
            folder: bound.root,
            owner: owner_label(scope, thread),
            computers,
            computer,
        })
    }
}

fn no_terminal() -> ApiError {
    ApiError::new(code::NO_TERMINAL, Sentence::NoTerminal)
}

/// A computer's refusal of a terminal call, as the session says it: its own plain words for a
/// limit or a size, never the operating system's, which go to the log alone.
fn refused(error: ComputerError) -> ApiError {
    match error {
        ComputerError::NotFound(_) => no_terminal(),
        ComputerError::Refused(reason) => ApiError::new(
            code::LIMIT,
            Sentence::TerminalRefused {
                reason: reason.to_string(),
            },
        ),
        other => {
            tracing::warn!(error = %other, "a terminal call failed");
            ApiError::new(code::INTERNAL, Sentence::TerminalFailed)
        }
    }
}
