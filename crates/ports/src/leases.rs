//! The leases port (`specs/invariants.md`, "Where state lives"): work in flight runs under a
//! lease recorded in the store, a row with a holder and an expiry on the store's own clock, the
//! same on SQLite and Postgres. A lease keeps one piece of work to one holder at a time: a
//! thread's turn to one server, an organization's schedules to one firer. Its holder renews it
//! while it works and lets go when it is done; one that stops without letting go loses it when
//! it lapses, and another may take it then. The log's `(thread, seq)` check stays the last guard,
//! so a lease lost late never corrupts a thread.

use std::fmt;
use std::time::Duration;

use glave_event::ThreadId;
use jiff::Timestamp;
use thiserror::Error;
use uuid::{Builder, Uuid};

use crate::env::{Clock, Entropy, EntropyError};
use crate::scope::OrgId;
use crate::tools::BoxFuture;

/// What a lease keeps to one holder at a time, within an organization.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Leased {
    /// A thread's turn: the one server that runs or continues it.
    Thread(ThreadId),
    /// The organization's schedules: the one server that fires them.
    Scheduler,
}

impl Leased {
    /// The name stores keep it under: `thread:<id>`, or `scheduler`.
    pub fn key(&self) -> String {
        match self {
            Self::Thread(thread) => format!("thread:{thread}"),
            Self::Scheduler => "scheduler".to_owned(),
        }
    }
}

/// Who holds a lease: one server, by an id it makes as it starts and never shares.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Holder(Uuid);

impl Holder {
    /// A new holder, its id a UUID v7 made from `clock` and `entropy`.
    pub fn new(clock: &dyn Clock, entropy: &dyn Entropy) -> Result<Self, EntropyError> {
        let random = entropy.bytes()?;
        let millis = u64::try_from(clock.now().as_millisecond()).unwrap_or(0);
        Ok(Self(
            Builder::from_unix_timestamp_millis(millis, &random).into_uuid(),
        ))
    }

    pub fn from_uuid(uuid: Uuid) -> Self {
        Self(uuid)
    }

    pub fn as_uuid(self) -> Uuid {
        self.0
    }
}

impl fmt::Display for Holder {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

/// A lease as the store holds it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Lease {
    pub org: OrgId,
    pub on: Leased,
    pub holder: Holder,
    /// When it lapses unless it is renewed first, on the store's clock: from then on it is held
    /// by no one.
    pub expires: Timestamp,
}

/// Why a leases call failed.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum LeaseError {
    #[error("lease storage failed: {0}")]
    Storage(String),
}

/// The longest a lease is taken for, whatever is asked: about 34 years, so an expiry stays a
/// number every store adds to its clock without overflowing.
const LONGEST: i64 = 1 << 40;

/// `ttl` in milliseconds as stores add it to their clock, at most about 34 years.
pub fn ttl_millis(ttl: Duration) -> i64 {
    i64::try_from(ttl.as_millis()).map_or(LONGEST, |millis| millis.min(LONGEST))
}

/// Where leases are kept: the data store, on its own clock, scoped by organization. A lease
/// whose expiry is not after the store's now has lapsed, and is held by no one.
pub trait Leases: Send + Sync {
    /// Takes the lease on `on` in `org` for `holder`, to lapse `ttl` from now on the store's
    /// clock: when no one holds it, when it lapsed, or when `holder` holds it already, which
    /// renews it. `None` when another holds it. Of holders racing for one lease, one gets it.
    fn acquire(
        &self,
        org: OrgId,
        on: Leased,
        holder: Holder,
        ttl: Duration,
    ) -> BoxFuture<'_, Result<Option<Lease>, LeaseError>>;

    /// Extends `lease` to lapse `ttl` from now while its holder still holds it, lapsed or not:
    /// `None` once it was let go, or another took it after it lapsed.
    fn renew<'a>(
        &'a self,
        lease: &'a Lease,
        ttl: Duration,
    ) -> BoxFuture<'a, Result<Option<Lease>, LeaseError>>;

    /// Lets go of `lease` while its holder still holds it, so another may take it at once; a
    /// lease another holds now stays theirs.
    fn release<'a>(&'a self, lease: &'a Lease) -> BoxFuture<'a, Result<(), LeaseError>>;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{CountingEntropy, ManualClock};

    #[test]
    fn keys_name_what_is_leased() {
        let thread = ThreadId::new(Timestamp::UNIX_EPOCH, [7; 10]).unwrap();
        assert_eq!(Leased::Thread(thread).key(), format!("thread:{thread}"));
        assert_eq!(Leased::Scheduler.key(), "scheduler");
    }

    #[test]
    fn a_ttl_is_capped_so_an_expiry_never_overflows() {
        assert_eq!(ttl_millis(Duration::from_secs(30)), 30_000);
        assert_eq!(ttl_millis(Duration::MAX), LONGEST);
    }

    #[test]
    fn each_holder_is_new() {
        let clock = ManualClock::at_millis(1_791_000_000_000);
        let entropy = CountingEntropy::starting_at(1);
        let first = Holder::new(&clock, &entropy).unwrap();
        assert_ne!(first, Holder::new(&clock, &entropy).unwrap());
        assert_eq!(first.as_uuid().get_version_num(), 7);
    }
}
