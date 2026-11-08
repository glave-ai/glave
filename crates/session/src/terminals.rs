//! A person's terminals in a thread's folder (`specs/session.md`, "Terminals"): their own shell,
//! on their own computer, outside the agent's sandbox, opened, typed into, read, resized and
//! closed for them alone. Nothing of it is recorded: the event log, the model and the server's
//! log never hold a byte typed or printed. The server keeps nothing of them either: each call
//! finds the thread's computer from its binding, as opening one does, and the computer keeps
//! which thread's each terminal is and checks it on every call.
//!
//! Who may open one: only the person whose computer it is. On the server's own computer, the
//! person the server works for (as only they leave the sandbox there, `specs/sandbox.md`,
//! "Whose computer"); where people have computers of their own, each on theirs. Never on a
//! worker, which holds a task's credential.

use std::sync::Arc;
use std::time::Duration;

use glave_event::{Body, ComputerKind, Payload as _, PrincipalId, ThreadId, WorkspaceBound};
use glave_harness::owner_label;
use glave_ports::computer::{
    Bytes, ComputerError, ComputerId, Computers, Refusal, TerminalId, TerminalSize,
};
use glave_ports::{BlobStore, Caller, EventLog, ModelProvider, Standing};

use crate::protocol::{ApiError, Reply, Told, code};
use crate::server::Server;
use crate::words::Sentence;

/// The longest one read waits for something to read: under the quiet a proxy allows a request.
const MOST_WAIT: Duration = Duration::from_secs(25);

/// Where a thread's terminals are: its folder, on its own computer, whose they are there, and
/// whose computer that is (none for this server's, its person's).
struct Place {
    folder: String,
    owner: String,
    computers: Arc<dyn Computers>,
    computer: ComputerId,
    person: Option<PrincipalId>,
}

impl Place {
    /// `terminal` there, as its owner names it.
    fn at(&self, terminal: TerminalId) -> (TerminalId, &str) {
        (terminal, &self.owner)
    }
}

impl<P, L, B> Server<P, L, B>
where
    P: ModelProvider + 'static,
    L: EventLog + 'static,
    B: BlobStore + 'static,
{
    /// Whether a terminal opens in a thread for its person, `whose`, where it works as `bound`
    /// records: shown, in a folder on its person's own computer, this server's for the person
    /// it works for, or theirs where people have computers of their own; never on a worker.
    pub(crate) fn opens_terminals(
        &self,
        standing: Standing,
        bound: Option<&WorkspaceBound>,
        whose: Option<PrincipalId>,
    ) -> bool {
        standing == Standing::Active
            && bound.is_some_and(|bound| match bound.computer.kind {
                ComputerKind::Local => whose.is_none_or(|person| Some(person) == self.person()),
                ComputerKind::Person => true,
                ComputerKind::Worker => false,
            })
    }

    /// Answers `terminal/open`: refused in a thread put away, and closed again when it was put
    /// away as this opened it.
    pub(crate) async fn open_terminal(
        &self,
        caller: &Caller,
        thread: ThreadId,
        size: TerminalSize,
    ) -> Result<Reply, ApiError> {
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
            let _ = computers.close_terminal(computer, place.at(id)).await;
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
            .write_terminal(place.computer, place.at(terminal), input);
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
            .read_terminal(place.computer, place.at(terminal), from, wait);
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
            .resize_terminal(place.computer, place.at(terminal), size);
        resized.await.map_err(refused)?;
        Ok(Reply::Done {})
    }

    /// Answers `terminal/close`, in a thread put away too.
    pub(crate) async fn close_terminal(
        &self,
        caller: &Caller,
        thread: ThreadId,
        terminal: TerminalId,
    ) -> Result<Reply, ApiError> {
        let place = self.on_theirs(caller, self.place_of(thread).await?)?;
        let closed = place
            .computers
            .close_terminal(place.computer, place.at(terminal));
        closed.await.map_err(refused)?;
        Ok(Reply::Done {})
    }

    /// Closes every terminal open in `thread`, as it is archived or deleted, whoever puts it
    /// away: those its own computer keeps for it.
    pub(crate) async fn close_terminals(&self, thread: ThreadId) {
        let Ok(place) = self.place_of(thread).await else {
            return;
        };
        let open = match place
            .computers
            .terminals(place.computer, &place.owner)
            .await
        {
            Ok(open) => open,
            Err(error) => {
                tracing::warn!(%error, %thread, "could not find a thread's terminals to close");
                return;
            }
        };
        for terminal in open {
            let closed = place
                .computers
                .close_terminal(place.computer, place.at(terminal.id))
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

    /// Where a shown `thread`'s terminals are for `caller`, refused unless its computer is
    /// theirs.
    async fn terminal_place(&self, caller: &Caller, thread: ThreadId) -> Result<Place, ApiError> {
        self.shown(thread).await?;
        self.on_theirs(caller, self.place_of(thread).await?)
    }

    /// `place`, when its computer is `caller`'s: this server's for the person it works for, or
    /// the computer of the person it is.
    fn on_theirs(&self, caller: &Caller, place: Place) -> Result<Place, ApiError> {
        let theirs = match place.person {
            None => self.is_theirs(caller),
            Some(person) => self.person_of(caller)? == person,
        };
        if !theirs {
            return Err(ApiError::new(
                code::NOT_ALLOWED,
                Sentence::TerminalNotTheirs,
            ));
        }
        Ok(place)
    }

    /// Where `thread`'s terminals are, whoever asks: the folder its latest binding records, on
    /// the computer it records when this server reaches it: its own for a thread here, its
    /// person's own where people have computers of their own.
    async fn place_of(&self, thread: ThreadId) -> Result<Place, ApiError> {
        let scope = self.scope_of(thread)?;
        let latest = self.latest(&scope, thread, WorkspaceBound::TYPE).await?;
        let Some(Body::WorkspaceBound(bound)) = latest else {
            return Err(ApiError::new(
                code::NOT_AVAILABLE,
                Sentence::TerminalNoFolder,
            ));
        };
        let no_home = || ApiError::new(code::NOT_AVAILABLE, Sentence::TerminalNoHome);
        let (computers, computer, person) = match bound.computer.kind {
            ComputerKind::Local => {
                let here = self.inner.harness.workspace();
                (Arc::clone(here.computers()), here.info().computer, None)
            }
            ComputerKind::Person => {
                let view = self.inner.harness.log().thread_view(&scope, thread).await?;
                let whose = view.and_then(|view| view.whose()).or_else(|| self.person());
                let person = whose.ok_or_else(no_home)?;
                let home = self.inner.harness.own_home(scope.org(), person);
                let home = home.ok_or_else(no_home)?.map_err(|_| no_home())?;
                (
                    Arc::clone(home.computers()),
                    home.info().computer,
                    Some(person),
                )
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
            person,
        })
    }
}

fn no_terminal() -> ApiError {
    ApiError::new(code::NO_TERMINAL, Sentence::NoTerminal)
}

/// A computer's refusal of a terminal call, as the session says it: glave's own words for a
/// limit, a size or a shell, written again in the caller's language, never the operating
/// system's, which go to the log alone.
fn refused(error: ComputerError) -> ApiError {
    match error {
        ComputerError::NotFound(_) => no_terminal(),
        ComputerError::Refused(reason) => match reason.refusal() {
            Some(refusal) => ApiError::new(refusal_code(refusal), Told::from(&reason)),
            None => failed(&reason),
        },
        // For good, so never "try again".
        ComputerError::Unsupported(_) => {
            ApiError::new(code::NOT_AVAILABLE, Sentence::TerminalUnsupported)
        }
        ComputerError::Unreachable(why) => {
            tracing::warn!(error = %why, "a terminal's computer could not be reached");
            ApiError::new(code::NOT_AVAILABLE, Sentence::TerminalUnreachable)
        }
        ComputerError::Failed(why) => failed(&why),
    }
}

/// A terminal call that failed, said plainly; why goes to the log.
fn failed(why: &str) -> ApiError {
    tracing::warn!(error = %why, "a terminal call failed");
    ApiError::new(code::INTERNAL, Sentence::TerminalFailed)
}

/// The code a computer's refusal of a terminal call answers with.
fn refusal_code(refusal: &Refusal) -> i64 {
    match refusal {
        Refusal::TooManyTerminals { .. } | Refusal::TooManyOwnedTerminals { .. } => code::LIMIT,
        Refusal::NoLoginShell => code::NOT_AVAILABLE,
        _ => code::INVALID_PARAMS,
    }
}
