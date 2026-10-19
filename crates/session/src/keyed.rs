//! Retrying `turn/start` safely: a turn started with an idempotency key is remembered, so the
//! same key for the same thread returns that turn instead of starting another. Each person has
//! a pool of their own, so no one's keys push out anyone else's: only a person's own most
//! recent keys are kept, and a key older than that starts a new turn.

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex, PoisonError};

use glave_event::{PrincipalId, ThreadId};
use tokio::sync::watch;

use crate::protocol::IdempotencyKey;
use crate::server::Ended;

/// Keys remembered for each person, across their threads.
const KEPT: usize = 256;

/// People whose keys are remembered, the one who claimed a key least recently let go first.
/// People are signed in, so no caller can make more of them to push others out.
const PEOPLE: usize = 4_096;

/// How a turn ended, once it has.
pub(crate) type Done = watch::Receiver<Option<Arc<Ended>>>;

/// Whose keys a pool holds: a signed-in person, or `None` for the local one.
type Person = Option<PrincipalId>;

/// Each person's turns started with a key.
#[derive(Default)]
pub(crate) struct Keyed {
    people: Mutex<People>,
}

#[derive(Default)]
struct People {
    /// Claims so far, to order people by when they last claimed a key.
    claims: u64,
    pools: HashMap<Person, Pool>,
}

/// One person's turns started with a key, most recent last.
struct Pool {
    /// The claim, counted across everyone, at which this person last claimed a key.
    used: u64,
    recent: VecDeque<(ThreadId, IdempotencyKey, Done)>,
}

impl Keyed {
    /// The turn `person` already started in `thread` with `key`, if there is one. Otherwise
    /// `done` becomes that turn, in the same step, so two retries racing each other start one
    /// turn.
    pub(crate) fn claim(
        &self,
        person: Person,
        thread: ThreadId,
        key: &IdempotencyKey,
        done: &Done,
    ) -> Option<Done> {
        let mut people = self.people.lock().unwrap_or_else(PoisonError::into_inner);
        people.claims = people.claims.saturating_add(1);
        let used = people.claims;
        if !people.pools.contains_key(&person) && people.pools.len() >= PEOPLE {
            let least = people.pools.iter().min_by_key(|(_, pool)| pool.used);
            if let Some(least) = least.map(|(of, _)| *of) {
                people.pools.remove(&least);
            }
        }
        let pool = people.pools.entry(person).or_insert_with(|| Pool {
            used,
            recent: VecDeque::new(),
        });
        pool.used = used;
        let started = pool
            .recent
            .iter()
            .find(|(of, kept, _)| *of == thread && kept == key)
            .map(|(_, _, started)| started.clone());
        if started.is_none() {
            if pool.recent.len() >= KEPT {
                pool.recent.pop_front();
            }
            pool.recent.push_back((thread, key.clone(), done.clone()));
        }
        started
    }

    /// Forgets `person`'s `key` when the turn it named could not start, so a retry may start
    /// it.
    pub(crate) fn release(
        &self,
        person: Person,
        thread: ThreadId,
        key: &IdempotencyKey,
        done: &Done,
    ) {
        let mut people = self.people.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(pool) = people.pools.get_mut(&person) {
            pool.recent.retain(|(of, kept, started)| {
                !(*of == thread && kept == key && started.same_channel(done))
            });
            if pool.recent.is_empty() {
                people.pools.remove(&person);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn thread(n: u8) -> ThreadId {
        ThreadId::new(jiff::Timestamp::UNIX_EPOCH, [n; 10]).unwrap()
    }

    fn key(text: &str) -> IdempotencyKey {
        IdempotencyKey::new(text).unwrap()
    }

    fn person(n: usize) -> PrincipalId {
        let [low, high, ..] = n.to_le_bytes();
        let random = [0, 0, 0, 0, 0, 0, 0, 1, high, low];
        PrincipalId::new(jiff::Timestamp::UNIX_EPOCH, random).unwrap()
    }

    #[test]
    fn a_key_names_one_turn_per_thread_and_a_persons_memory_is_bounded() {
        let keyed = Keyed::default();
        let (_first, done) = watch::channel(None);
        assert!(keyed.claim(None, thread(1), &key("a"), &done).is_none());
        let (_second, other) = watch::channel(None);
        let again = keyed.claim(None, thread(1), &key("a"), &other).unwrap();
        assert!(again.same_channel(&done));
        // The same key in another thread is another turn.
        assert!(keyed.claim(None, thread(2), &key("a"), &other).is_none());
        keyed.release(None, thread(1), &key("a"), &done);
        assert!(keyed.claim(None, thread(1), &key("a"), &other).is_none());
        for n in 0..KEPT {
            let _ = keyed.claim(None, thread(3), &key(&n.to_string()), &other);
        }
        assert!(
            keyed.claim(None, thread(2), &key("a"), &done).is_none(),
            "the oldest was let go"
        );
    }

    #[test]
    fn one_persons_flood_never_evicts_anothers_key() {
        let keyed = Keyed::default();
        let (_theirs, done) = watch::channel(None);
        assert!(
            keyed
                .claim(Some(person(1)), thread(1), &key("a"), &done)
                .is_none()
        );
        let (_flood, other) = watch::channel(None);
        for n in 0..KEPT * 4 {
            let _ = keyed.claim(Some(person(2)), thread(1), &key(&n.to_string()), &other);
        }
        let followed = keyed.claim(Some(person(1)), thread(1), &key("a"), &other);
        assert!(followed.is_some_and(|started| started.same_channel(&done)));
        // Nor does someone else's same key name their turn.
        assert!(
            keyed
                .claim(Some(person(3)), thread(1), &key("a"), &other)
                .is_none()
        );
    }

    #[test]
    fn the_person_who_claimed_least_recently_is_let_go_beyond_the_cap() {
        let keyed = Keyed::default();
        let (_held, done) = watch::channel(None);
        for n in 0..PEOPLE {
            let _ = keyed.claim(Some(person(n)), thread(1), &key("a"), &done);
        }
        // Claimed again, the first person is now the most recent, so the second is let go.
        assert!(
            keyed
                .claim(Some(person(0)), thread(1), &key("a"), &done)
                .is_some()
        );
        let _ = keyed.claim(Some(person(PEOPLE)), thread(1), &key("a"), &done);
        let people = keyed.people.lock().unwrap();
        assert_eq!(people.pools.len(), PEOPLE);
        assert!(people.pools.contains_key(&Some(person(0))));
        assert!(!people.pools.contains_key(&Some(person(1))));
    }
}
