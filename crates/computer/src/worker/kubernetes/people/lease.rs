//! A person's computer lease (`specs/computers.md`, decision 2): where several servers share one
//! store, the one that wakes a person's computer holds its lease in the store, renewed while the
//! computer wakes or is awake, and only that server makes, grows or removes its Pod. Any other
//! server reaches the computer awake through a connection of its own, and waits for the lease to
//! wake one that sleeps: until its holder lets go as the computer sleeps, or the lease lapses, 30
//! seconds after its holder stops, so none replaces a Pod in use. A server counts the lease its
//! own only until it lapses after its last renewal, by its own clock: one that cannot renew in
//! time, or finds another holds it, stops at once and leaves the computer alone.

use std::sync::Arc;
use std::time::Duration;

use glave_event::PrincipalId;
use glave_ports::computer::ComputerError;
use glave_ports::{Holder, Lease, Leased, Leases, OrgId};
use tokio::time::Instant;

/// How long a computer's lease lasts unless it is renewed: a server that stops without letting
/// go keeps the computer from the others this long at most.
pub(super) const LEASE: Duration = Duration::from_secs(30);

/// How often a lease is renewed, well within [`LEASE`].
pub(super) const RENEW: Duration = Duration::from_secs(10);

/// How long the store has to answer a take, a renewal or a letting go: seconds, so a store that
/// hangs costs a server its lease, never a stalled renewal.
const ANSWERS_WITHIN: Duration = Duration::from_secs(5);

/// How much sooner than the store a server counts its lease lapsed, for the time a renewal
/// takes to come back and clocks that drift.
const MARGIN: Duration = Duration::from_secs(5);

/// What a call is told when another server held a sleeping computer's lease for as long as
/// [`LEASE_WAIT`]: it neither woke it nor let go.
pub(super) const ELSEWHERE: &str = "your computer is being woken through another of glave's \
     servers, which hasn't finished; try again in a minute";

/// How long a server waits for another to wake a person's computer, or to let go of it, before
/// telling the call so: the holder finds a computer that slept on its own within a minute, and
/// its lease lapses 30 seconds after it stops.
pub(super) const LEASE_WAIT: Duration = Duration::from_mins(2);

/// Where people's computer leases are kept, and who this server holds them as.
#[derive(Clone)]
pub struct Leasing {
    pub leases: Arc<dyn Leases>,
    /// This server, made as it starts and never shared.
    pub holder: Holder,
}

impl std::fmt::Debug for Leasing {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Leasing")
            .field("holder", &self.holder)
            .finish_non_exhaustive()
    }
}

/// A computer's lease as this server holds it: surely its own until `until`.
#[derive(Debug, Clone, Copy)]
pub(super) struct Held {
    lease: Lease,
    pub(super) until: Instant,
}

impl Held {
    /// Whether it is still surely this server's.
    pub(super) fn holds(&self) -> bool {
        Instant::now() < self.until
    }
}

impl Leasing {
    /// Takes the lease of the computer of `person` in `org`, or renews it when this server holds
    /// it already: none while another server holds it.
    pub(super) async fn take(
        &self,
        org: OrgId,
        person: PrincipalId,
    ) -> Result<Option<Held>, ComputerError> {
        let asked = Instant::now();
        let on = Leased::Computer(person);
        let taken = tokio::time::timeout(
            ANSWERS_WITHIN,
            self.leases.acquire(org, on, self.holder, LEASE),
        )
        .await;
        match taken {
            Ok(Ok(lease)) => Ok(lease.map(|lease| held(lease, asked))),
            Ok(Err(_)) | Err(_) => Err(ComputerError::Failed(
                "glave could not reach its records to wake your computer; try again".into(),
            )),
        }
    }

    /// Lets go of `held` once the computer sleeps, so another server may wake it at once
    /// rather than after the lease lapses; one the store does not let go of in time lapses.
    pub(super) async fn release(&self, held: &Held) {
        let _ = tokio::time::timeout(ANSWERS_WITHIN, self.leases.release(&held.lease)).await;
    }

    /// Renews `held`: `false` once it is no longer surely this server's, because another took
    /// it after it lapsed or because the store did not renew it before it would have.
    pub(super) async fn renew(&self, held: &mut Held) -> bool {
        let asked = Instant::now();
        let renewed =
            tokio::time::timeout(ANSWERS_WITHIN, self.leases.renew(&held.lease, LEASE)).await;
        match renewed {
            Ok(Ok(Some(lease))) => *held = self::held(lease, asked),
            Ok(Ok(None)) => return false,
            // Not renewed this time: still its own until it would have lapsed.
            Ok(Err(_)) | Err(_) => {}
        }
        held.holds()
    }
}

/// `lease`, asked for at `asked`: surely held until it lapses, counted from the asking.
fn held(lease: Lease, asked: Instant) -> Held {
    Held {
        lease,
        until: asked + LEASE.saturating_sub(MARGIN),
    }
}

#[cfg(test)]
mod tests;
