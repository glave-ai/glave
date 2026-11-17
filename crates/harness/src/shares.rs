//! Work of many organizations sharing a few places on one server: at most so many run at once
//! in all, and so many for one organization, so no one's work holds up everyone's.

use std::collections::HashMap;
use std::sync::{Mutex, PoisonError};

use tokio::sync::Notify;

use glave_ports::OrgId;

/// Places for work, at most `most` taken at once in all and `per_org` by one organization.
pub(crate) struct Shares {
    most: usize,
    per_org: usize,
    taken: Mutex<Taken>,
    /// Tells those waiting that a place was given back.
    freed: Notify,
}

/// The places taken now, in all and by organization.
#[derive(Default)]
struct Taken {
    all: usize,
    by_org: HashMap<OrgId, usize>,
}

/// A place taken among [`Shares`], given back as it drops.
pub(crate) struct Share<'a> {
    shares: &'a Shares,
    org: OrgId,
}

impl Shares {
    /// Places for work, at most `most` at once in all and `per_org` for one organization.
    pub(crate) fn new(most: usize, per_org: usize) -> Self {
        Self {
            most,
            per_org,
            taken: Mutex::default(),
            freed: Notify::new(),
        }
    }

    /// A place for work of `org`'s, once fewer than `most` are taken in all and fewer than
    /// `per_org` by `org`. Waits as long as that takes: its caller holds the wait to a limit.
    pub(crate) async fn share(&self, org: OrgId) -> Share<'_> {
        loop {
            let freed = self.freed.notified();
            {
                let mut taken = self.taken.lock().unwrap_or_else(PoisonError::into_inner);
                let theirs = taken.by_org.get(&org).copied().unwrap_or(0);
                if taken.all < self.most && theirs < self.per_org {
                    taken.all = taken.all.saturating_add(1);
                    taken.by_org.insert(org, theirs.saturating_add(1));
                    return Share { shares: self, org };
                }
            }
            freed.await;
        }
    }
}

impl Drop for Share<'_> {
    fn drop(&mut self) {
        let mut taken = self
            .shares
            .taken
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        taken.all = taken.all.saturating_sub(1);
        let left = taken.by_org.get(&self.org).copied().unwrap_or(0);
        if left <= 1 {
            taken.by_org.remove(&self.org);
        } else {
            taken.by_org.insert(self.org, left.saturating_sub(1));
        }
        drop(taken);
        self.shares.freed.notify_waiters();
    }
}

#[cfg(test)]
mod tests {
    use std::future::Future;
    use std::pin::pin;
    use std::task::{Context, Poll, Waker};

    use super::*;

    fn org(n: u8) -> OrgId {
        OrgId::from_uuid(uuid::Uuid::from_bytes([n; 16]))
    }

    /// Whether `share` is given at once.
    fn given_now<'a>(share: impl Future<Output = Share<'a>>) -> bool {
        let mut share = pin!(share);
        let mut context = Context::from_waker(Waker::noop());
        matches!(share.as_mut().poll(&mut context), Poll::Ready(_))
    }

    #[test]
    fn one_organization_takes_its_share_and_another_still_gets_a_place() {
        let shares = Shares::new(4, 2);
        let mut context = Context::from_waker(Waker::noop());
        let held: Vec<Share<'_>> = (0..2)
            .map(
                |_| match pin!(shares.share(org(1))).as_mut().poll(&mut context) {
                    Poll::Ready(share) => share,
                    Poll::Pending => panic!("a first organization's first two are given at once"),
                },
            )
            .collect();
        assert!(!given_now(shares.share(org(1))), "past its share, it waits");
        assert!(
            given_now(shares.share(org(2))),
            "another organization's goes ahead"
        );
        drop(held);
        assert!(
            given_now(shares.share(org(1))),
            "a place given back is its again"
        );
    }

    #[test]
    fn no_more_than_the_most_are_taken_in_all() {
        let shares = Shares::new(2, 2);
        let mut context = Context::from_waker(Waker::noop());
        let first = pin!(shares.share(org(1))).as_mut().poll(&mut context);
        let second = pin!(shares.share(org(2))).as_mut().poll(&mut context);
        assert!(matches!(
            (&first, &second),
            (Poll::Ready(_), Poll::Ready(_))
        ));
        assert!(!given_now(shares.share(org(3))));
        drop(first);
        assert!(given_now(shares.share(org(3))));
        drop(second);
    }
}
