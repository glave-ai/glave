//! Work done on a person's computer under its lease, the lease renewed all the while: removing
//! a computer can take minutes (a Pod on a lost node stays until the cluster lets go of it),
//! and the renewal that found it gone is the one removing it, so nothing else renews meanwhile.
//! A lease left to lapse would let another server wake the computer, a new Pod and policy of the
//! same names, while this one still removes them by name (#1392).

use std::future::Future;
use std::sync::PoisonError;

use super::{Person, lease};

/// Runs `work` on `person`'s computer, awake from its wake `wake`, renewing the lease that wake
/// took every [`lease::RENEW`] while it goes on: what it came to, or `None` once the lease is no
/// longer surely this server's, `work` then stopped and the computer left to whoever holds it.
pub(super) async fn renewed_through<T>(
    person: &Person,
    wake: u64,
    work: impl Future<Output = T>,
) -> Option<T> {
    let mut work = std::pin::pin!(work);
    loop {
        tokio::select! {
            done = &mut work => return Some(done),
            () = tokio::time::sleep(lease::RENEW) => {
                if !renewed(person, wake).await {
                    return None;
                }
            }
        }
    }
}

/// Renews the lease of `person`'s computer that its wake `wake` took, as this server keeps it:
/// whether it is still surely this server's.
async fn renewed(person: &Person, wake: u64) -> bool {
    let kept = *person.held.lock().unwrap_or_else(PoisonError::into_inner);
    let Some((_, mut held)) = kept.filter(|(of, _)| *of == wake) else {
        return false;
    };
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
    renewed
}
