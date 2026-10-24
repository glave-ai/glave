//! Changing memories: saving (with the credential check, the near-duplicate check and the
//! budget), editing, forgetting (the key destroyed at once), moving and restoring. The model
//! and the person both come through here; only the person may restore or move.

use glave_event::{
    At, MemoryEdited, MemoryForgotten, MemoryId, MemoryRestored, MemorySaved, MemoryScope,
    MemorySource, Principal, PrincipalId,
};
use glave_ports::memories::{MemoryError, Remembered};
use glave_ports::secrets::looks_like_credential;
use glave_ports::{Erasable, Scope};
use jiff::{SignedDuration, Timestamp};

use super::Memories;
use crate::book::{Book, Entry, by_glave};
use crate::similar::near;

/// The longest memory, in characters.
pub(super) const MOST_CHARS: usize = 500;
/// About how many tokens each scope's memories may take, all loaded.
pub const BUDGET_TOKENS: usize = 1_000;
/// Characters each memory's line adds beside its text: its id, scope and dates.
const LINE_CHARS: usize = 60;
/// How long glave's own changes can be undone, and its forgets wait before erasing.
const UNDO_DAYS: i64 = 30;

/// When a forget destroys the memory's key.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Erasure {
    /// At once: the person's forget, or the model's for them.
    Now,
    /// Then, so glave's forget can be undone until then.
    At(At),
    /// Then, and tidy-up merged it into this memory.
    Merged(At, MemoryId),
}

/// Who changes memory, and from where: always the session's or the thread's, never the model's.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Writer {
    /// The person it is for: their personal memory is theirs alone.
    pub person: Option<PrincipalId>,
    /// The project it is changed from.
    pub scope: Scope,
    /// Who is recorded as changing it.
    pub by: Principal,
    /// The thread and `seq` a save is made in.
    pub source: Option<MemorySource>,
    /// The thread had read untrusted content or a secrets file.
    pub untrusted_read: bool,
}

/// What `remember` did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Remembering {
    /// Saved, replacing `replaced` when it named one.
    Saved {
        memory: Remembered,
        replaced: Option<MemoryId>,
    },
    /// Not saved: these are close to it. Replace one, or save anyway with `force`.
    Similar(Vec<Remembered>),
}

/// A memory's text, checked: one short line, never a secret.
pub(crate) fn checked(text: &str) -> Result<String, MemoryError> {
    let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if text.is_empty() {
        return Err(MemoryError::Invalid("a memory needs some text".into()));
    }
    let length = text.chars().count();
    if length > MOST_CHARS {
        return Err(MemoryError::Invalid(format!(
            "a memory is one short statement of at most {MOST_CHARS} characters; this is \
             {length}. Say it shorter, or split it"
        )));
    }
    if let Some(reason) = looks_like_credential(&text) {
        return Err(MemoryError::Invalid(format!(
            "not saved: {reason}. Memory never keeps secrets such as keys, tokens or passwords; \
             leave the value out (you may remember where it is kept, without the value)"
        )));
    }
    Ok(text)
}

/// About how many tokens a memory takes in view, as the budget counts it: its text and its
/// line's id, scope and dates.
pub fn tokens(text: &str) -> usize {
    text.chars().count().saturating_add(LINE_CHARS).div_ceil(4)
}

impl<L: glave_ports::EventLog + 'static> Memories<L> {
    /// Saves `text` to `which` scope, or replaces `replaces` with it. Close matches come back
    /// instead of a save unless `force`; a save over the scope's budget is refused.
    pub async fn remember(
        &self,
        who: &Writer,
        text: &str,
        which: MemoryScope,
        replaces: Option<MemoryId>,
        force: bool,
    ) -> Result<Remembering, MemoryError> {
        let text = checked(text)?;
        let ledger = self.rules().place(who.person, who.scope, which)?;
        let old = match replaces {
            Some(memory) => Some(self.locate(who.person, who.scope, memory).await?),
            None => None,
        };
        let replaced_id = old.as_ref().map(|(_, entry)| entry.memory);
        let others: Vec<Remembered> = self
            .with_book(ledger, Book::live)
            .await?
            .into_iter()
            .filter(|memory| memory.scope == which && Some(memory.memory) != replaced_id)
            .collect();
        if !force {
            let similar: Vec<Remembered> = others
                .iter()
                .filter(|memory| near(&text, &memory.text))
                .cloned()
                .collect();
            if !similar.is_empty() {
                return Ok(Remembering::Similar(similar));
            }
        }
        let used: usize = others.iter().map(|memory| tokens(&memory.text)).sum();
        if used.saturating_add(tokens(&text)) > BUDGET_TOKENS {
            return Err(MemoryError::Invalid(format!(
                "not saved: the {} memories are full (about {BUDGET_TOKENS} tokens). Merge \
                 related memories with edit, or replace one that matters less, then try again",
                which.as_str()
            )));
        }
        let random = (self.inner.entropy.bytes())
            .map_err(|error| MemoryError::Storage(error.to_string()))?;
        let memory = MemoryId::new(self.inner.clock.now(), random)
            .map_err(|error| MemoryError::Storage(error.to_string()))?;
        let saved = MemorySaved {
            memory,
            scope: which,
            text,
            source: who.source,
            untrusted_read: who.untrusted_read,
            by: who.by.clone(),
        };
        self.append(ledger, &saved).await?;
        if let Some((old_ledger, entry)) = old {
            self.forget_in(old_ledger, entry.memory, &who.by, Erasure::Now)
                .await?;
        }
        let memory = self
            .with_book(ledger, |book| {
                book.entries.get(&memory).and_then(Entry::live)
            })
            .await?
            .ok_or_else(|| MemoryError::Storage("the saved memory did not read back".into()))?;
        Ok(Remembering::Saved {
            memory,
            replaced: replaced_id,
        })
    }

    /// Changes a memory's text. Sealed under the same key, so forgetting it erases every
    /// version.
    pub async fn edit(
        &self,
        who: &Writer,
        memory: MemoryId,
        text: &str,
    ) -> Result<Remembered, MemoryError> {
        let text = checked(text)?;
        let (ledger, entry) = self.locate(who.person, who.scope, memory).await?;
        if entry.forgot.is_some() {
            return Err(MemoryError::NotFound(format!(
                "memory {memory} is forgotten; restore it first"
            )));
        }
        let edited = MemoryEdited {
            memory,
            text,
            by: who.by.clone(),
        };
        self.append(ledger, &edited).await?;
        self.with_book(ledger, |book| {
            book.entries.get(&memory).and_then(Entry::live)
        })
        .await?
        .ok_or_else(|| MemoryError::Storage("the edited memory did not read back".into()))
    }

    /// Forgets a memory now: its key is destroyed at once, so no version can be read again, and
    /// so are those of the memories tidy-up merged into it that still wait to be erased. The
    /// memory as it was, unless glave had forgotten it already.
    pub async fn forget(
        &self,
        who: &Writer,
        memory: MemoryId,
    ) -> Result<Option<Remembered>, MemoryError> {
        let (ledger, entry) = self.locate(who.person, who.scope, memory).await?;
        // A memory glave forgot waits to be erased; anyone else's forget erases it now.
        if entry.forgot.is_some() && by_glave(&who.by) {
            return Ok(None);
        }
        // Those merged into it first, so a forget stopped halfway can be asked for again.
        let merged = self
            .with_book(ledger, |book| book.merged_into(memory))
            .await?;
        for merged in merged {
            self.forget_in(ledger, merged, &who.by, Erasure::Now)
                .await?;
        }
        self.forget_in(ledger, memory, &who.by, Erasure::Now)
            .await?;
        Ok(entry.live())
    }

    /// Records the forget in `ledger`, erased as `erasure` says.
    pub(crate) async fn forget_in(
        &self,
        ledger: Scope,
        memory: MemoryId,
        by: &Principal,
        erasure: Erasure,
    ) -> Result<(), MemoryError> {
        let now = self.now();
        let (erase_at, into) = match erasure {
            Erasure::Now => (now, None),
            Erasure::At(at) => (at, None),
            Erasure::Merged(at, into) => (at, Some(into)),
        };
        let forgotten = MemoryForgotten {
            memory,
            erase_at,
            into,
            by: by.clone(),
        };
        self.append(ledger, &forgotten).await?;
        if erasure == Erasure::Now {
            self.inner
                .eraser
                .destroy(ledger, Erasable::Memory(memory))
                .await?;
        }
        Ok(())
    }

    /// Forgets every memory of `which` scope the writer may change, and erases now those glave
    /// forgot that still wait to be erased. The memories that were in force.
    pub async fn forget_all(
        &self,
        who: &Writer,
        which: MemoryScope,
    ) -> Result<Vec<Remembered>, MemoryError> {
        let ledger = self.rules().place(who.person, who.scope, which)?;
        let (all, waiting) = self
            .with_book(ledger, |book| (book.live(), book.waiting()))
            .await?;
        let all: Vec<Remembered> = all
            .into_iter()
            .filter(|memory| memory.scope == which)
            .collect();
        let waiting = waiting.into_iter().filter(|(_, scope)| *scope == which);
        let every = all.iter().map(|memory| memory.memory);
        for memory in every.chain(waiting.map(|(memory, _)| memory)) {
            self.forget_in(ledger, memory, &who.by, Erasure::Now)
                .await?;
        }
        Ok(all)
    }

    /// Moves a memory to `which` scope: saved in the new place, forgotten in the old.
    pub async fn move_to(
        &self,
        who: &Writer,
        memory: MemoryId,
        which: MemoryScope,
    ) -> Result<Remembered, MemoryError> {
        let (_, entry) = self.locate(who.person, who.scope, memory).await?;
        if entry.scope == which {
            return entry
                .live()
                .ok_or_else(|| MemoryError::NotFound(format!("memory {memory} is forgotten")));
        }
        let text = entry
            .live()
            .map(|live| live.text)
            .ok_or_else(|| MemoryError::NotFound(format!("memory {memory} is forgotten")))?;
        let saved = self.remember(who, &text, which, Some(memory), true).await?;
        match saved {
            Remembering::Saved { memory, .. } => Ok(memory),
            Remembering::Similar(_) => Err(MemoryError::Storage(
                "a forced save came back unsaved".into(),
            )),
        }
    }

    /// Undoes glave's latest change to a memory, within 30 days of it.
    pub async fn restore(&self, who: &Writer, memory: MemoryId) -> Result<Remembered, MemoryError> {
        let (ledger, entry) = self.locate(who.person, who.scope, memory).await?;
        let since = At::from(
            self.now()
                .timestamp()
                .checked_sub(SignedDuration::from_hours(UNDO_DAYS.saturating_mul(24)))
                .unwrap_or(Timestamp::UNIX_EPOCH),
        );
        let undone = if let Some(forgot) = entry.forgot.as_ref().filter(|f| by_glave(&f.by)) {
            Some(forgot.event)
        } else {
            entry
                .versions
                .iter()
                .skip(1)
                .rev()
                .find(|version| !version.undone)
                .filter(|version| by_glave(&version.by) && version.at >= since)
                .map(|version| version.event)
        };
        let undone = undone.ok_or_else(|| {
            MemoryError::Invalid(format!(
                "glave has not changed memory {memory} in the last {UNDO_DAYS} days; there is \
                 nothing to undo"
            ))
        })?;
        let restored = MemoryRestored {
            memory,
            undone,
            by: who.by.clone(),
        };
        self.append(ledger, &restored).await?;
        self.with_book(ledger, |book| {
            book.entries.get(&memory).and_then(Entry::live)
        })
        .await?
        .ok_or_else(|| MemoryError::Storage("the restored memory did not read back".into()))
    }
}
