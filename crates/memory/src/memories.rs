//! The memory store: each project's memory ledger in the event log (`specs/memory.md`, "Events"),
//! read into a view per project and appended to by any glave process. Every read and write takes
//! the caller and the project, and goes only to the ledgers the rules allow them.

mod housekeep;
mod recall;
mod tidy;
mod write;

use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, RwLock};

use glave_event::{At, EventId, Header, MemoryId, MemoryScope, Payload, PrincipalId, Seq, encode};
use glave_ports::memories::{
    MemoryContext, MemoryError, MemoryRefusal, ProjectMemory, Remembered, Shown,
};
use glave_ports::stream::{Reserved, append_retrying, catch_up};
use glave_ports::{Clock, Entropy, Eraser, EventLog, ProjectId, Scope};

use crate::book::{Book, Entry};
use crate::rules::{Personal, Rules};

pub use housekeep::Housekept;
pub(crate) use recall::DEFAULT_HITS;
pub use recall::{MOST_HITS, Recalled, Source};
pub use tidy::{Tidied, Tidier};
pub use write::{BUDGET_TOKENS, Remembering, Writer, tokens};

/// Projects whose views are kept in memory at once.
const MOST_BOOKS: usize = 64;

/// glave's memory over one data store.
pub struct Memories<L> {
    inner: Arc<Inner<L>>,
}

impl<L> Clone for Memories<L> {
    fn clone(&self) -> Self {
        Self {
            inner: Arc::clone(&self.inner),
        }
    }
}

struct Inner<L> {
    log: L,
    eraser: Arc<dyn Eraser>,
    rules: RwLock<Rules>,
    clock: Box<dyn Clock>,
    entropy: Box<dyn Entropy>,
    books: Mutex<HashMap<Scope, Book>>,
}

impl<L: EventLog + 'static> Memories<L> {
    /// Memory kept in `log`, erased through `eraser`, by `rules`.
    pub fn new(
        log: L,
        eraser: Arc<dyn Eraser>,
        rules: Rules,
        clock: Box<dyn Clock>,
        entropy: Box<dyn Entropy>,
    ) -> Self {
        Self {
            inner: Arc::new(Inner {
                log,
                eraser,
                rules: RwLock::new(rules),
                clock,
                entropy,
                books: Mutex::new(HashMap::new()),
            }),
        }
    }

    /// The rules memory runs by now.
    pub fn rules(&self) -> Rules {
        self.inner
            .rules
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// Sets `project`'s memory to `setting` from now on, as its project's settings choose it.
    pub fn set_project(&self, project: ProjectId, setting: ProjectMemory) {
        let mut rules = self
            .inner
            .rules
            .write()
            .unwrap_or_else(PoisonError::into_inner);
        if setting == rules.default {
            rules.projects.remove(&project);
        } else {
            rules.projects.insert(project, setting);
        }
    }

    /// Runs by what the person's settings choose now, from the next read or write: whether
    /// their own memory is read (`personal`), every project's setting where its project chooses
    /// none (`default`), and how long memories are kept.
    pub fn set_person(
        &self,
        personal: Option<Personal>,
        default: ProjectMemory,
        retention_days: Option<u32>,
    ) {
        let mut rules = self
            .inner
            .rules
            .write()
            .unwrap_or_else(PoisonError::into_inner);
        rules.personal = personal;
        rules.default = default;
        rules.retention_days = retention_days;
        rules.projects.retain(|_, setting| *setting != default);
    }

    pub(crate) fn now(&self) -> At {
        At::from(self.inner.clock.now())
    }

    fn books(&self) -> MutexGuard<'_, HashMap<Scope, Book>> {
        self.inner
            .books
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }

    /// Runs `read` on the view of `scope`'s ledger, brought up to date first.
    pub(crate) async fn with_book<T>(
        &self,
        scope: Scope,
        read: impl FnOnce(&Book) -> T,
    ) -> Result<T, MemoryError> {
        self.catch_up(scope).await?;
        let books = self.books();
        Ok(read(books.get(&scope).unwrap_or(&Book::default())))
    }

    /// Forgets the view of `scope`, so the next read rebuilds it from the log: after keys were
    /// destroyed, text cached before must not outlive them.
    pub(crate) fn drop_book(&self, scope: Scope) {
        self.books().remove(&scope);
    }

    /// Reads whatever was added to `scope`'s ledger since this process last looked.
    async fn catch_up(&self, scope: Scope) -> Result<(), MemoryError> {
        let next = {
            let mut books = self.books();
            if !books.contains_key(&scope) && books.len() >= MOST_BOOKS {
                books.clear();
            }
            books.entry(scope).or_default().next_seq()
        };
        let after = Seq::new(next.saturating_sub(1)).ok();
        catch_up(
            &self.inner.log,
            &scope,
            Reserved::Memory.id(),
            after,
            |page| {
                let mut books = self.books();
                let book = books.entry(scope).or_default();
                for raw in page {
                    book.apply(raw);
                }
            },
        )
        .await?;
        Ok(())
    }

    /// Appends `payload` to `scope`'s ledger, after anything others wrote meanwhile.
    pub(crate) async fn append<T: Payload>(
        &self,
        scope: Scope,
        payload: &T,
    ) -> Result<(), MemoryError> {
        let ledger = Reserved::Memory.id();
        let this = self;
        let appended = append_retrying(&self.inner.log, &scope, ledger, move || async move {
            let next = this.with_book(scope, Book::next_seq).await?;
            let now = this.inner.clock.now();
            let header = Header {
                id: EventId::new(now, this.inner.entropy.bytes().map_err(invalid)?)
                    .map_err(invalid)?,
                thread: ledger,
                seq: Seq::new(next).map_err(invalid)?,
                at: At::from(now),
            };
            let raw = encode(header, payload).map_err(invalid)?;
            Ok::<_, MemoryError>(Some((vec![raw], ())))
        })
        .await?;
        let mut books = self.books();
        let book = books.entry(scope).or_default();
        for raw in appended.into_iter().flat_map(|(events, ())| events) {
            book.apply(raw);
        }
        Ok(())
    }

    /// The memories a thread of `on` reads now: the person's, then the project's, oldest first.
    pub async fn list(&self, on: &MemoryContext) -> Result<Vec<Remembered>, MemoryError> {
        self.list_as(on.person, on.scope, None).await
    }

    /// The memories `person` reads in `scope`, of `which` scope or both.
    pub async fn list_as(
        &self,
        person: Option<PrincipalId>,
        scope: Scope,
        which: Option<MemoryScope>,
    ) -> Result<Vec<Remembered>, MemoryError> {
        let mut memories = Vec::new();
        for (kind, ledger) in self.rules().readable(person, scope) {
            if which.is_some_and(|which| which != kind) {
                continue;
            }
            let live = self.with_book(ledger, Book::live).await?;
            memories.extend(live.into_iter().filter(|memory| memory.scope == kind));
        }
        Ok(memories)
    }

    /// The contradictions tidy-up left for the person among the memories a thread of `on`
    /// reads: both are kept until one of them changes.
    pub async fn questions(&self, on: &MemoryContext) -> Result<Vec<[MemoryId; 2]>, MemoryError> {
        let mut open = Vec::new();
        for (_, ledger) in self.rules().readable(on.person, on.scope) {
            open.extend(self.with_book(ledger, Book::open_questions).await?);
        }
        Ok(open)
    }

    /// The memories in `wanted` at those versions, from the ledgers a thread of `on` reads.
    /// Forgotten ones are left out.
    pub async fn versions(
        &self,
        on: &MemoryContext,
        wanted: &[Shown],
    ) -> Result<Vec<Remembered>, MemoryError> {
        let mut found = Vec::new();
        for (kind, ledger) in self.rules().readable(on.person, on.scope) {
            let mut here = self
                .with_book(ledger, |book| {
                    wanted
                        .iter()
                        .filter_map(|shown| {
                            let entry = book.entries.get(&shown.memory)?;
                            (entry.forgot.is_none() && entry.scope == kind)
                                .then(|| entry.at(shown.version))?
                        })
                        .collect::<Vec<_>>()
                })
                .await?;
            found.append(&mut here);
        }
        Ok(found)
    }

    /// The memory `memory` among those `person` may read in `scope`, with the ledger it is in.
    pub(crate) async fn locate(
        &self,
        person: Option<PrincipalId>,
        scope: Scope,
        memory: MemoryId,
    ) -> Result<(Scope, Entry), MemoryError> {
        for (kind, ledger) in self.rules().readable(person, scope) {
            let entry = self
                .with_book(ledger, |book| book.entries.get(&memory).cloned())
                .await?;
            if let Some(entry) = entry.filter(|entry| entry.scope == kind) {
                return Ok((ledger, entry));
            }
        }
        Err(MemoryError::NotFound(MemoryRefusal::NoMemory {
            memory: memory.to_string(),
        }))
    }
}

fn storage(error: impl std::fmt::Display) -> MemoryError {
    MemoryError::Storage(error.to_string())
}

fn invalid(error: impl std::fmt::Display) -> MemoryError {
    MemoryError::Storage(format!("could not record the change: {error}"))
}
