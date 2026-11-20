//! Housekeeping, one pass over every project of an organization that has memories
//! ([`Memories::housekeep_in`]), run daily by the person's server and as glave starts and a
//! session ends: memories past the retention are forgotten by glave, keys whose erasure is due
//! are destroyed, and a project key is rotated once a key was destroyed under it, batching the
//! forgets since the last run, so backups lose them too, wherever they were made (#1486).

use glave_event::{At, MemoryId, Principal};
use glave_ports::memories::MemoryError;
use glave_ports::stream::Reserved;
use glave_ports::{Erasable, EventLog, OrgId, Scope};
use jiff::SignedDuration;

use super::Memories;
use super::write::Erasure;

/// How long glave's own forgets wait before their keys are destroyed, so they can be undone.
const ERASE_AFTER_DAYS: i64 = 30;

/// How many projects one page of an organization's names, while erasure goes through them all.
const PROJECTS_A_PAGE: usize = 100;

/// What one housekeeping run did.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Housekept {
    /// Memories glave forgot for retention.
    pub forgotten: usize,
    /// Memories whose keys were destroyed.
    pub erased: usize,
    /// Project keys rotated.
    pub rotated: usize,
}

fn days_before(at: At, days: i64) -> At {
    At::from(
        at.timestamp()
            .checked_sub(SignedDuration::from_hours(days.saturating_mul(24)))
            .unwrap_or(jiff::Timestamp::UNIX_EPOCH),
    )
}

fn days_after(at: At, days: i64) -> At {
    At::from(
        at.timestamp()
            .checked_add(SignedDuration::from_hours(days.saturating_mul(24)))
            .unwrap_or(at.timestamp()),
    )
}

impl<L: EventLog + 'static> Memories<L> {
    /// Housekeeps every project of `org` that has memories, a page of them at a time, whoever's
    /// they are and whether or not they are in use: memories past the retention forgotten by
    /// glave, keys whose erasure is due destroyed, and each project key rotated once a key was
    /// destroyed under it, so backups lose a forget within a day wherever it was made
    /// (`specs/memory.md`, "Forgetting", #1486).
    pub async fn housekeep_in(&self, org: OrgId) -> Result<Housekept, MemoryError> {
        let mut done = Housekept::default();
        let mut after = None;
        loop {
            let page = self
                .inner
                .log
                .projects_with(org, Reserved::Memory.id(), after, PROJECTS_A_PAGE)
                .await
                .map_err(|error| MemoryError::Storage(error.to_string()))?;
            for project in &page {
                let scope = Scope::glaves_own(org, *project);
                done.forgotten = done.forgotten.saturating_add(self.retain(scope).await?);
                let erased = self.erase_due(scope).await?;
                done.erased = done.erased.saturating_add(erased.erased);
                done.rotated = done.rotated.saturating_add(erased.rotated);
            }
            match page.last() {
                Some(last) if page.len() == PROJECTS_A_PAGE => after = Some(*last),
                _ => return Ok(done),
            }
        }
    }

    /// Forgets, as glave, the memories of `scope` past the retention, to be erased in
    /// [`ERASE_AFTER_DAYS`] unless restored: how many.
    async fn retain(&self, scope: Scope) -> Result<usize, MemoryError> {
        let Some(days) = self.rules().retention_days else {
            return Ok(0);
        };
        let now = self.now();
        let cutoff = days_before(now, i64::from(days));
        let stale: Vec<MemoryId> = self
            .with_book(scope, |book| {
                book.entries
                    .values()
                    .filter(|entry| entry.forgot.is_none())
                    .filter(|entry| entry.touched().is_some_and(|at| at < cutoff))
                    .map(|entry| entry.memory)
                    .collect()
            })
            .await?;
        let erase_at = days_after(now, ERASE_AFTER_DAYS);
        for memory in &stale {
            self.forget_in(scope, *memory, &Principal::glave(), Erasure::At(erase_at))
                .await?;
        }
        Ok(stale.len())
    }

    /// Destroys the keys in `scope` whose erasure is due, then rotates its project key if a key
    /// was destroyed under it.
    async fn erase_due(&self, scope: Scope) -> Result<Housekept, MemoryError> {
        let mut done = Housekept::default();
        let now = self.now();
        let due: Vec<MemoryId> = self
            .with_book(scope, |book| {
                let waited = book.entries.values().filter_map(|entry| {
                    entry
                        .forgot
                        .as_ref()
                        .filter(|forgot| forgot.erase_at <= now)
                        .map(|_| entry.memory)
                });
                waited.chain(book.erasing.iter().copied()).collect()
            })
            .await?;
        for memory in &due {
            self.inner
                .eraser
                .destroy(scope, Erasable::Memory(*memory))
                .await?;
            done.erased = done.erased.saturating_add(1);
        }
        if !due.is_empty() {
            self.drop_book(scope);
        }
        if self.inner.eraser.rotate(scope).await? {
            done.rotated = done.rotated.saturating_add(1);
        }
        Ok(done)
    }
}
