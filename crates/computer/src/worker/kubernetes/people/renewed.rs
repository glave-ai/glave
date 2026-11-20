//! Work done on a person's computer under its lease, the lease renewed all the while: removing
//! a computer can take minutes (a Pod on a lost node stays until the cluster lets go of it),
//! and the renewal that found it gone is the one removing it, so nothing else renews meanwhile.
//! A lease left to lapse would let another server wake the computer, a new Pod and policy of the
//! same names, while this one still removes them by name (#1392).

use std::future::Future;
use std::sync::PoisonError;

use tokio::time::Instant;

use super::{Person, lease};

/// Removes what is left of `person`'s computer by `removal`, under the lease its wake `wake`
/// took (`held`), then lets go of that lease; one lost meanwhile is not let go of, being no
/// longer this server's.
pub(super) async fn removed_then_let_go<T>(
    person: &Person,
    wake: u64,
    held: &lease::Held,
    removal: impl Future<Output = T>,
) {
    if renewed_through(person, wake, removal).await.is_some() {
        person.leasing.release(held).await;
    }
}

/// Runs `work` on `person`'s computer, awake from its wake `wake`, renewing the lease that wake
/// took every [`lease::RENEW`] while it goes on: what it came to, or `None` once the lease is no
/// longer surely this server's, `work` then stopped and the computer left to whoever holds it.
/// A renewal the store fails, or doesn't answer in time, leaves the lease surely this server's
/// only until the last one that worked lapses here: `work` stops then, never after, whenever the
/// next renewal is due.
pub(super) async fn renewed_through<T>(
    person: &Person,
    wake: u64,
    work: impl Future<Output = T>,
) -> Option<T> {
    let mut work = std::pin::pin!(work);
    let mut until = kept(person, wake)?;
    loop {
        tokio::select! {
            biased;
            () = tokio::time::sleep_until(until) => return None,
            done = &mut work => return Some(done),
            () = tokio::time::sleep(lease::RENEW) => until = renewed(person, wake).await?,
        }
    }
}

/// Until when the lease of `person`'s computer that its wake `wake` took is surely this server's,
/// as it keeps it; none if it keeps none.
fn kept(person: &Person, wake: u64) -> Option<Instant> {
    let kept = *person.held.lock().unwrap_or_else(PoisonError::into_inner);
    kept.filter(|(of, _)| *of == wake)
        .map(|(_, held)| held.until)
}

/// Renews the lease of `person`'s computer that its wake `wake` took, as this server keeps it:
/// until when it is surely this server's now, or none once it is not.
async fn renewed(person: &Person, wake: u64) -> Option<Instant> {
    let kept = *person.held.lock().unwrap_or_else(PoisonError::into_inner);
    let (_, mut held) = kept.filter(|(of, _)| *of == wake)?;
    let renewed = person.leasing.renew(&mut held).await;
    if let Some((of, kept)) = person
        .held
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .as_mut()
        && *of == wake
    {
        *kept = held;
    }
    renewed.then_some(held.until)
}
