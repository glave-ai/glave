//! What someone not signed in may ask that makes the server keep or read something: beginning
//! to sign in or make a passkey, which a request kept in memory waits for, and what an invite's
//! link opens, which reads its organization's people. Each is counted against the network it
//! came from on this server, as emails asked for are (`mailing.rs`), so no one network fills
//! what every sign-in waits in, nor keeps the server reading. One asked on this computer, in
//! process or over stdio, counts against nothing.

use std::collections::BTreeMap;
use std::sync::{MutexGuard, PoisonError};

use glave_ports::{BlobStore, Caller, EventLog, ModelProvider};
use jiff::{SignedDuration, Timestamp};

use crate::mailing::networks;
use crate::protocol::{ApiError, Request, code};
use crate::server::Server;
use crate::words::Sentence;

/// How many one network may ask within [`WITHIN`]: a household or an office signing in
/// together, not a stranger's way to push everyone else's sign-in out.
const PER_NETWORK: u32 = 20;

/// How many the networks of one IPv6 /48 together may ask within [`WITHIN`]: one person may
/// be given a whole /48, and with it 65,536 /64s.
const PER_BLOCK: u32 = 40;

/// The while each count runs over, from a network's first ask in it.
const WITHIN: SignedDuration = SignedDuration::from_mins(1);

/// The networks counted at once: past it, one not counted yet is let through uncounted, so
/// what is counted stays bounded and no flood of networks stops anyone else.
const MOST_NETWORKS: usize = 4_096;

/// What each network asked within its while: when the while began, and how many.
#[derive(Debug, Default)]
pub(crate) struct Strangers(BTreeMap<String, (Timestamp, u32)>);

impl Strangers {
    /// Counts one ask from the networks `counted`, each with the most it may ask, at `now`:
    /// false, counting nothing, when one has asked its most.
    fn count(&mut self, counted: &[(String, u32)], now: Timestamp) -> bool {
        self.0
            .retain(|_, (began, _)| now.duration_since(*began) < WITHIN);
        let full = counted
            .iter()
            .any(|(network, most)| self.0.get(network).is_some_and(|(_, asked)| asked >= most));
        if full {
            return false;
        }
        for (network, _) in counted {
            if let Some((_, asked)) = self.0.get_mut(network) {
                *asked = asked.saturating_add(1);
            } else if self.0.len() < MOST_NETWORKS {
                self.0.insert(network.clone(), (now, 1));
            }
        }
        true
    }
}

/// Whether `request`, asked by someone not signed in, is one counted.
fn counted(request: &Request) -> bool {
    matches!(
        request,
        Request::PasskeyCreateOptions { .. }
            | Request::PasskeyGetOptions {}
            | Request::InviteShow { .. }
    )
}

impl<P, L, B> Server<P, L, B>
where
    P: ModelProvider + 'static,
    L: EventLog + 'static,
    B: BlobStore + 'static,
{
    /// Counts `request` against the network of `caller`, when no one signed in asks one that
    /// is counted: refused once that network asked [`PER_NETWORK`] within the minute, or an
    /// IPv6 address's /48 [`PER_BLOCK`].
    pub(crate) fn count_stranger(
        &self,
        caller: &Caller,
        request: &Request,
    ) -> Result<(), ApiError> {
        let Some(from) = caller
            .from
            .filter(|_| caller.is_no_one() && counted(request))
        else {
            return Ok(());
        };
        let (network, block) = networks(from);
        let mut counts = vec![(network, PER_NETWORK)];
        counts.extend(block.map(|block| (block, PER_BLOCK)));
        let now = self.inner.harness.clock().now();
        if self.strangers().count(&counts, now) {
            Ok(())
        } else {
            Err(ApiError::new(code::LIMIT, Sentence::SignInsTooOften))
        }
    }

    fn strangers(&self) -> MutexGuard<'_, Strangers> {
        self.inner
            .strangers
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(second: i64) -> Timestamp {
        Timestamp::from_second(1_800_000_000 + second).unwrap()
    }

    #[test]
    fn a_network_asks_its_most_in_a_minute_without_holding_back_another() {
        let mut strangers = Strangers::default();
        let one = [("203.0.113.7".to_owned(), 3)];
        let other = [("198.51.100.1".to_owned(), 3)];
        for _ in 0..3 {
            assert!(strangers.count(&one, at(0)));
        }
        assert!(!strangers.count(&one, at(59)), "its most within the minute");
        assert!(strangers.count(&other, at(59)), "another network asks on");
        assert!(strangers.count(&one, at(60)), "a minute on, asked again");
    }

    #[test]
    fn a_block_full_refuses_its_networks_and_counts_nothing_refused() {
        let mut strangers = Strangers::default();
        let block = "2001:db8:1::/48".to_owned();
        let first = [("2001:db8:1:1::/64".to_owned(), 5), (block.clone(), 2)];
        let second = [("2001:db8:1:2::/64".to_owned(), 5), (block, 2)];
        assert!(strangers.count(&first, at(0)));
        assert!(strangers.count(&second, at(0)));
        assert!(!strangers.count(&first, at(1)), "the /48 asked its most");
        assert_eq!(
            strangers
                .0
                .get("2001:db8:1:1::/64")
                .map(|(_, asked)| *asked),
            Some(1),
            "a refusal counts nothing"
        );
    }

    #[test]
    fn past_the_networks_counted_a_new_one_is_let_through_uncounted() {
        let mut strangers = Strangers::default();
        for n in 0..MOST_NETWORKS {
            assert!(strangers.count(&[(n.to_string(), 1)], at(0)));
        }
        let late = [("late".to_owned(), 1)];
        assert!(strangers.count(&late, at(0)));
        assert!(strangers.count(&late, at(0)), "never counted");
        assert_eq!(strangers.0.len(), MOST_NETWORKS);
    }
}
