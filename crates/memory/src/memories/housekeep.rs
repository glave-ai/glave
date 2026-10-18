//! Housekeeping, run from time to time (when glave starts, when a session ends, and on the
//! session server's scheduler tick): memories past the retention are forgotten by glave, keys
//! whose erasure is due are destroyed, and a space key is rotated once a key was destroyed
//! under it, batching the forgets since the last run, so backups lose them too.

use glave_event::{At, MemoryId, Principal};
use glave_ports::memories::MemoryError;
use glave_ports::{EventLog, Scope};
use jiff::SignedDuration;

use super::Memories;

/// How long glave's own forgets wait before their keys are destroyed, so they can be undone.
const ERASE_AFTER_DAYS: i64 = 30;

/// What one housekeeping run did.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Housekept {
    /// Memories glave forgot for retention.
    pub forgotten: usize,
    /// Memories whose keys were destroyed.
    pub erased: usize,
    /// Space keys rotated.
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
    /// Keeps the memory of each of `scopes` (the person's personal space and the spaces in
    /// use) small and erased on time.
    pub async fn housekeep(&self, scopes: &[Scope]) -> Result<Housekept, MemoryError> {
        let mut done = Housekept::default();
        let now = self.now();
        for scope in scopes {
            if let Some(days) = self.rules().retention_days {
                let cutoff = days_before(now, i64::from(days));
                let stale: Vec<MemoryId> = self
                    .with_book(*scope, |book| {
                        book.entries
                            .values()
                            .filter(|entry| entry.forgot.is_none())
                            .filter(|entry| entry.touched().is_some_and(|at| at < cutoff))
                            .map(|entry| entry.memory)
                            .collect()
                    })
                    .await?;
                let erase_at = days_after(now, ERASE_AFTER_DAYS);
                for memory in stale {
                    self.forget_in(*scope, memory, &Principal::glave(), Some(erase_at))
                        .await?;
                    done.forgotten = done.forgotten.saturating_add(1);
                }
            }
            let due: Vec<MemoryId> = self
                .with_book(*scope, |book| {
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
                    .shredder
                    .destroy(*scope, *memory)
                    .await
                    .map_err(MemoryError::Storage)?;
                done.erased = done.erased.saturating_add(1);
            }
            if !due.is_empty() {
                self.drop_book(*scope);
            }
            if self
                .inner
                .shredder
                .rotate(*scope)
                .await
                .map_err(MemoryError::Storage)?
            {
                done.rotated = done.rotated.saturating_add(1);
            }
        }
        Ok(done)
    }
}
