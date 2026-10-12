//! The memory store: each space's memory ledger in the event log (`specs/memory.md`, "Events"),
//! read into a view per space and appended to by any glave process. Every read and write takes
//! the caller and the space, and goes only to the ledgers the rules allow them.

mod housekeep;
mod recall;
mod tidy;
mod write;

use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use glave_event::{At, EventId, Header, MemoryId, MemoryScope, Payload, PrincipalId, Seq, encode};
use glave_harness::{Clock, Entropy, MemoryContext, MemoryError, Remembered, Shown, memory_ledger};
use glave_ports::{EventLog, LogError, Scope};

use crate::book::{Book, Entry};
use crate::rules::Rules;
use crate::shred::Shredder;

pub use housekeep::Housekept;
pub(crate) use recall::DEFAULT_HITS;
pub use recall::{MOST_HITS, Recalled, Source};
pub use tidy::{ModelTidier, Tidied, Tidier};
pub use write::{Remembering, Writer};

/// Ledger events read per page.
const PAGE: usize = 500;
/// Appends tried before giving up on a ledger other writers keep moving.
const ATTEMPTS: usize = 8;
/// Spaces whose views are kept in memory at once.
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
    shredder: Arc<dyn Shredder>,
    rules: Rules,
    clock: Box<dyn Clock>,
    entropy: Box<dyn Entropy>,
    books: Mutex<HashMap<Scope, Book>>,
}

impl<L: EventLog + 'static> Memories<L> {
    /// Memory kept in `log`, erased through `shredder`, by `rules`.
    pub fn new(
        log: L,
        shredder: Arc<dyn Shredder>,
        rules: Rules,
        clock: Box<dyn Clock>,
        entropy: Box<dyn Entropy>,
    ) -> Self {
        Self {
            inner: Arc::new(Inner {
                log,
                shredder,
                rules,
                clock,
                entropy,
                books: Mutex::new(HashMap::new()),
            }),
        }
    }

    pub fn rules(&self) -> &Rules {
        &self.inner.rules
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
        let ledger = ledger()?;
        loop {
            let next = {
                let mut books = self.books();
                if !books.contains_key(&scope) && books.len() >= MOST_BOOKS {
                    books.clear();
                }
                books.entry(scope).or_default().next_seq()
            };
            let after = Seq::new(next.saturating_sub(1)).ok();
            let page = self
                .inner
                .log
                .read(&scope, ledger, after, PAGE)
                .await
                .map_err(storage)?;
            let full = page.len() == PAGE;
            let mut books = self.books();
            let book = books.entry(scope).or_default();
            for raw in page {
                book.apply(raw);
            }
            if !full {
                return Ok(());
            }
        }
    }

    /// Appends `payload` to `scope`'s ledger, after anything others wrote meanwhile.
    pub(crate) async fn append<T: Payload>(
        &self,
        scope: Scope,
        payload: &T,
    ) -> Result<(), MemoryError> {
        let ledger = ledger()?;
        for _ in 0..ATTEMPTS {
            let next = self.with_book(scope, Book::next_seq).await?;
            let now = self.inner.clock.now();
            let header = Header {
                id: EventId::new(now, self.inner.entropy.bytes()).map_err(invalid)?,
                thread: ledger,
                seq: Seq::new(next).map_err(invalid)?,
                at: At::from(now),
            };
            let raw = encode(header, payload).map_err(invalid)?;
            match self
                .inner
                .log
                .append(&scope, std::slice::from_ref(&raw))
                .await
            {
                Ok(()) => {
                    self.books().entry(scope).or_default().apply(raw);
                    return Ok(());
                }
                Err(LogError::Conflict { .. } | LogError::Gap { .. }) => {}
                Err(error) => return Err(storage(error)),
            }
        }
        Err(MemoryError::Storage(
            "the memory ledger kept changing; try again".into(),
        ))
    }

    /// The memories a thread of `on` reads now: the person's, then the space's, oldest first.
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
        for (kind, ledger) in self.inner.rules.readable(person, scope) {
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
        for (_, ledger) in self.inner.rules.readable(on.person, on.scope) {
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
        for (kind, ledger) in self.inner.rules.readable(on.person, on.scope) {
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
        memory: glave_event::MemoryId,
    ) -> Result<(Scope, Entry), MemoryError> {
        for (kind, ledger) in self.inner.rules.readable(person, scope) {
            let entry = self
                .with_book(ledger, |book| book.entries.get(&memory).cloned())
                .await?;
            if let Some(entry) = entry.filter(|entry| entry.scope == kind) {
                return Ok((ledger, entry));
            }
        }
        Err(MemoryError::NotFound(format!(
            "there is no memory {memory} here; list the memories to see their ids"
        )))
    }
}

fn ledger() -> Result<glave_event::ThreadId, MemoryError> {
    memory_ledger().ok_or_else(|| MemoryError::Storage("the memory ledger has no id".into()))
}

fn storage(error: impl std::fmt::Display) -> MemoryError {
    MemoryError::Storage(error.to_string())
}

fn invalid(error: impl std::fmt::Display) -> MemoryError {
    MemoryError::Storage(format!("could not record the change: {error}"))
}
