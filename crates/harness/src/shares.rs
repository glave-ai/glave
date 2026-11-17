//! Work of many organizations sharing a few places on one server: at most so many run at once
//! in all, and so many for one organization, so no one's work holds up everyone's. An
//! organization's own share is taken first; the server's places are then given in the order
//! asked for, so a place freed goes to whoever has waited longest, never to a late arrival.

use std::collections::HashMap;
use std::sync::{Mutex, PoisonError};

use glave_ports::OrgId;
use tokio::sync::{Notify, Semaphore, SemaphorePermit};

/// Places for work, at most `most` taken at once in all and `per_org` by one organization.
pub(crate) struct Shares {
    /// The server's places, given in the order asked for.
    places: Semaphore,
    per_org: usize,
    /// How many of each organization's share are taken.
    taken: Mutex<HashMap<OrgId, usize>>,
    /// Tells an organization's work waiting for its share that one was given back.
    freed: Notify,
}

/// A place taken among [`Shares`], given back as it drops: the server's, then its
/// organization's share.
pub(crate) struct Share<'a> {
    _place: SemaphorePermit<'a>,
    _mine: Mine<'a>,
}

impl Shares {
    /// Places for work, at most `most` at once in all and `per_org` for one organization.
    pub(crate) fn new(most: usize, per_org: usize) -> Self {
        Self {
            places: Semaphore::new(most),
            per_org,
            taken: Mutex::default(),
            freed: Notify::new(),
        }
    }

    /// A place for work of `org`'s: once fewer than `per_org` are `org`'s, then a server's place,
    /// in the order asked for. Waits as long as that takes: its caller holds the wait to a limit.
    pub(crate) async fn share(&self, org: OrgId) -> Option<Share<'_>> {
        loop {
            let freed = self.freed.notified();
            {
                let mut taken = self.taken.lock().unwrap_or_else(PoisonError::into_inner);
                let theirs = taken.get(&org).copied().unwrap_or(0);
                if theirs < self.per_org {
                    taken.insert(org, theirs.saturating_add(1));
                    break;
                }
            }
            freed.await;
        }
        // Its own share taken, it is given back if this is dropped while it waits for a place.
        let mine = Mine { shares: self, org };
        let place = self.places.acquire().await.ok()?;
        Some(Share {
            _place: place,
            _mine: mine,
        })
    }

    /// Gives back one of `org`'s share.
    fn give_back(&self, org: OrgId) {
        let mut taken = self.taken.lock().unwrap_or_else(PoisonError::into_inner);
        let left = taken.get(&org).copied().unwrap_or(0);
        if left <= 1 {
            taken.remove(&org);
        } else {
            taken.insert(org, left.saturating_sub(1));
        }
        drop(taken);
        self.freed.notify_waiters();
    }
}

/// One of an organization's share, taken: given back as it drops, with its work's place, or as
/// its wait for a place ends without one.
struct Mine<'a> {
    shares: &'a Shares,
    org: OrgId,
}

impl Drop for Mine<'_> {
    fn drop(&mut self) {
        self.shares.give_back(self.org);
    }
}

#[cfg(test)]
mod tests {
    use std::future::Future;
    use std::pin::{Pin, pin};
    use std::task::{Context, Poll, Waker};

    use super::*;

    fn org(n: u8) -> OrgId {
        OrgId::from_uuid(uuid::Uuid::from_bytes([n; 16]))
    }

    /// `asking` polled once: the share, when it is given now.
    fn now<'a>(asking: Pin<&mut impl Future<Output = Option<Share<'a>>>>) -> Option<Share<'a>> {
        let mut context = Context::from_waker(Waker::noop());
        match asking.poll(&mut context) {
            Poll::Ready(share) => share,
            Poll::Pending => None,
        }
    }

    #[test]
    fn one_organization_takes_its_share_and_another_still_gets_a_place() {
        let shares = Shares::new(4, 2);
        let first = now(pin!(shares.share(org(1)))).unwrap();
        let second = now(pin!(shares.share(org(1)))).unwrap();
        assert!(
            now(pin!(shares.share(org(1)))).is_none(),
            "past its share, it waits"
        );
        assert!(
            now(pin!(shares.share(org(2)))).is_some(),
            "another's goes ahead"
        );
        drop((first, second));
        assert!(
            now(pin!(shares.share(org(1)))).is_some(),
            "a place given back is its again"
        );
    }

    #[test]
    fn a_place_freed_goes_to_whoever_waited_longest_never_a_late_arrival() {
        let shares = Shares::new(4, 2);
        // Two organizations keep the server's four places.
        let mut held: Vec<Share<'_>> = [1, 1, 2, 2]
            .into_iter()
            .map(|n| now(pin!(shares.share(org(n)))).unwrap())
            .collect();
        // A third asks, and waits for a place.
        let mut third = pin!(shares.share(org(3)));
        assert!(now(third.as_mut()).is_none());
        // The first gives one back, and asks again at once.
        held.pop();
        let mut late = pin!(shares.share(org(2)));
        assert!(
            now(late.as_mut()).is_none(),
            "the late arrival waits its turn"
        );
        assert!(now(third.as_mut()).is_some(), "the third goes first");
    }

    #[test]
    fn a_wait_given_up_gives_its_organization_s_share_back() {
        let shares = Shares::new(1, 1);
        let only = now(pin!(shares.share(org(1)))).unwrap();
        {
            let mut waiting = pin!(shares.share(org(2)));
            assert!(now(waiting.as_mut()).is_none());
        }
        drop(only);
        assert!(
            now(pin!(shares.share(org(2)))).is_some(),
            "its share is its again"
        );
    }
}
