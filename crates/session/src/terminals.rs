//! A person's terminals in a thread's folder (`specs/session.md`, "Terminals"): their own shell,
//! on their own computer, outside the agent's sandbox, opened, typed into, read, resized and
//! closed for them alone. Nothing of it is recorded: the event log, the model and the server's
//! log never hold a byte typed or printed. The server keeps which thread each terminal is in,
//! and the computer it is open on, in memory.
//!
//! Who may open one: only the person whose computer it is. On the server's own computer, the
//! person the server works for (as only they leave the sandbox there, `specs/sandbox.md`,
//! "Whose computer"); where people have computers of their own, each on theirs. Never on a
//! worker, which holds a task's credential.

use std::collections::BTreeMap;
use std::sync::{Arc, MutexGuard, PoisonError};
use std::time::Duration;

use glave_event::{Body, ComputerKind, Payload as _, PrincipalId, ThreadId, WorkspaceBound};
use glave_ports::computer::{
    Bytes, ComputerError, ComputerId, Computers, TerminalId, TerminalSize,
};
use glave_ports::{BlobStore, Caller, EventLog, ModelProvider};

use crate::protocol::{ApiError, Reply, TerminalInfo, code};
use crate::server::Server;
use crate::words::Sentence;

/// The most terminals one thread keeps open at once; its computer keeps a few more across them.
const MOST_IN_THREAD: usize = 4;

/// The longest one read waits for something to read: under the quiet a proxy allows a request.
const MOST_WAIT: Duration = Duration::from_secs(25);

/// Each thread's open terminals, oldest first.
pub(crate) type Kept = BTreeMap<ThreadId, Vec<Open>>;

/// One open terminal, and the computer it is open on.
pub(crate) struct Open {
    id: TerminalId,
    folder: String,
    computers: Arc<dyn Computers>,
    computer: ComputerId,
}

impl Open {
    fn info(&self) -> TerminalInfo {
        TerminalInfo {
            id: self.id,
            folder: self.folder.clone(),
        }
    }
}

/// Where a thread's terminal opens: its folder, on a computer of the caller's own.
struct Place {
    folder: String,
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
        self.inner.homes.is_some() || self.works_for(caller.person)
    }

    /// Whether the server works for `person`: the local person, or the one it was set up for.
    fn works_for(&self, person: Option<PrincipalId>) -> bool {
        person.is_none_or(|person| Some(person) == self.person())
    }

    /// Answers `terminal/open`.
    pub(crate) async fn open_terminal(
        &self,
        caller: &Caller,
        thread: ThreadId,
        size: TerminalSize,
    ) -> Result<Reply, ApiError> {
        let place = self.terminal_place(caller, thread).await?;
        if self.kept().get(&thread).map_or(0, Vec::len) >= MOST_IN_THREAD {
            let most = u64::try_from(MOST_IN_THREAD).unwrap_or(u64::MAX);
            return Err(ApiError::new(
                code::LIMIT,
                Sentence::TerminalsInThread { most },
            ));
        }
        let id = place
            .computers
            .open_terminal(place.computer, &place.folder, size)
            .await
            .map_err(refused)?;
        let open = Open {
            id,
            folder: place.folder,
            computers: place.computers,
            computer: place.computer,
        };
        let terminal = open.info();
        self.kept().entry(thread).or_default().push(open);
        Ok(Reply::Terminal { terminal })
    }

    /// Answers `terminal/list`.
    pub(crate) fn list_terminals(&self, thread: ThreadId) -> Reply {
        let terminals = self
            .kept()
            .get(&thread)
            .map(|open| open.iter().map(Open::info).collect())
            .unwrap_or_default();
        Reply::Terminals { terminals }
    }

    /// Answers `terminal/write`.
    pub(crate) async fn write_terminal(
        &self,
        thread: ThreadId,
        terminal: TerminalId,
        input: &Bytes,
    ) -> Result<Reply, ApiError> {
        let (computers, computer) = self.terminal_of(thread, terminal)?;
        let written = computers.write_terminal(computer, terminal, input).await;
        self.settle(thread, terminal, written)?;
        Ok(Reply::Done {})
    }

    /// Answers `terminal/read`.
    pub(crate) async fn read_terminal(
        &self,
        thread: ThreadId,
        terminal: TerminalId,
        from: u64,
        wait: Duration,
    ) -> Result<Reply, ApiError> {
        let (computers, computer) = self.terminal_of(thread, terminal)?;
        let wait = wait.min(MOST_WAIT);
        let read = computers
            .read_terminal(computer, terminal, from, wait)
            .await;
        let screen = self.settle(thread, terminal, read)?;
        Ok(Reply::Screen { screen })
    }

    /// Answers `terminal/resize`.
    pub(crate) async fn resize_terminal(
        &self,
        thread: ThreadId,
        terminal: TerminalId,
        size: TerminalSize,
    ) -> Result<Reply, ApiError> {
        let (computers, computer) = self.terminal_of(thread, terminal)?;
        let resized = computers.resize_terminal(computer, terminal, size).await;
        self.settle(thread, terminal, resized)?;
        Ok(Reply::Done {})
    }

    /// Answers `terminal/close`: one its computer no longer has is closed all the same.
    pub(crate) async fn close_terminal(
        &self,
        thread: ThreadId,
        terminal: TerminalId,
    ) -> Result<Reply, ApiError> {
        let (computers, computer) = self.terminal_of(thread, terminal)?;
        self.forget(thread, terminal);
        match computers.close_terminal(computer, terminal).await {
            Ok(()) | Err(ComputerError::NotFound(_)) => Ok(Reply::Done {}),
            Err(error) => Err(refused(error)),
        }
    }

    /// Closes every terminal open in `thread`, as it is archived or deleted.
    pub(crate) async fn close_terminals(&self, thread: ThreadId) {
        let open = self.kept().remove(&thread).unwrap_or_default();
        for terminal in open {
            let closed = terminal
                .computers
                .close_terminal(terminal.computer, terminal.id)
                .await;
            if let Err(error) = closed {
                tracing::warn!(%error, %thread, "could not close a terminal");
            }
        }
    }

    /// Where `thread`'s terminal opens for `caller`: the folder its latest binding records, on
    /// the computer it is on when that computer is the caller's.
    async fn terminal_place(&self, caller: &Caller, thread: ThreadId) -> Result<Place, ApiError> {
        let scope = self.scope_of(thread)?;
        let latest = self.latest(&scope, thread, WorkspaceBound::TYPE).await?;
        let Some(Body::WorkspaceBound(bound)) = latest else {
            return Err(not_theirs());
        };
        let (computers, computer) = match bound.computer.kind {
            ComputerKind::Local if self.works_for(caller.person) => {
                let here = self.inner.harness.workspace();
                (Arc::clone(here.computers()), here.info().computer)
            }
            ComputerKind::Person => {
                let person = self.person_of(caller)?;
                let home = self.inner.harness.own_home(scope.org(), person);
                let home = home
                    .ok_or_else(not_theirs)?
                    .map_err(|error| ApiError::from(&error))?;
                (Arc::clone(home.computers()), home.info().computer)
            }
            ComputerKind::Worker => {
                return Err(ApiError::new(
                    code::NOT_AVAILABLE,
                    Sentence::TerminalOnWorker,
                ));
            }
            ComputerKind::Local => return Err(not_theirs()),
        };
        if computer.as_uuid() != bound.computer.id {
            return Err(not_theirs());
        }
        Ok(Place {
            folder: bound.root,
            computers,
            computer,
        })
    }

    /// The computer `terminal`, open in `thread`, is open on.
    fn terminal_of(
        &self,
        thread: ThreadId,
        terminal: TerminalId,
    ) -> Result<(Arc<dyn Computers>, ComputerId), ApiError> {
        self.kept()
            .get(&thread)
            .and_then(|open| open.iter().find(|open| open.id == terminal))
            .map(|open| (Arc::clone(&open.computers), open.computer))
            .ok_or_else(no_terminal)
    }

    /// `answered`, from the computer `terminal` of `thread` is open on: one it no longer has
    /// (it went to sleep, or nothing used it for an hour) is forgotten here too.
    fn settle<T>(
        &self,
        thread: ThreadId,
        terminal: TerminalId,
        answered: Result<T, ComputerError>,
    ) -> Result<T, ApiError> {
        answered.map_err(|error| {
            if matches!(error, ComputerError::NotFound(_)) {
                self.forget(thread, terminal);
            }
            refused(error)
        })
    }

    fn forget(&self, thread: ThreadId, terminal: TerminalId) {
        let mut kept = self.kept();
        if let Some(open) = kept.get_mut(&thread) {
            open.retain(|open| open.id != terminal);
            if open.is_empty() {
                kept.remove(&thread);
            }
        }
    }

    fn kept(&self) -> MutexGuard<'_, Kept> {
        self.inner
            .terminals
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }
}

fn not_theirs() -> ApiError {
    ApiError::new(code::NOT_ALLOWED, Sentence::TerminalNotTheirs)
}

fn no_terminal() -> ApiError {
    ApiError::new(code::NO_TERMINAL, Sentence::NoTerminal)
}

/// A computer's refusal of a terminal call, as the session says it.
fn refused(error: ComputerError) -> ApiError {
    match error {
        ComputerError::NotFound(_) => no_terminal(),
        ComputerError::Refused(reason) => ApiError::new(
            code::INVALID_PARAMS,
            Sentence::TerminalFailed {
                reason: reason.to_string(),
            },
        ),
        other => ApiError::new(
            code::INTERNAL,
            Sentence::TerminalFailed {
                reason: other.message().to_owned(),
            },
        ),
    }
}
