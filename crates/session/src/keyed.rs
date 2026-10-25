//! Retrying `turn/start` safely: what a send with an idempotency key did is remembered, so the
//! same key for the same thread is answered as it was the first time: with the turn it started,
//! instead of starting another, or with the id it queued its message under, instead of queueing
//! it again. Each person has a pool of their own, so no one's keys push out anyone else's: only
//! a person's own most recent keys are kept, and a key older than that is sent anew.

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex, PoisonError};

use glave_event::{PrincipalId, ThreadId};
use tokio::sync::watch;

use crate::protocol::{IdempotencyKey, QueuedMessage};
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

/// What a send with a key did first.
#[derive(Clone)]
pub(crate) enum Kept {
    /// It started a turn, or is still deciding whether it does: how that turn ends.
    Turn(Done),
    /// It queued its message to the turn running then.
    Queued(QueuedMessage),
}

/// What each person's sends with a key did.
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

/// What one person's sends with a key did, most recent last.
struct Pool {
    /// The claim, counted across everyone, at which this person last claimed a key.
    used: u64,
    recent: VecDeque<(ThreadId, IdempotencyKey, Kept)>,
}

/// What claiming a key found.
pub(crate) enum Claimed<'a> {
    /// A send with the key came first: what it did.
    Before(Kept),
    /// The key is this send's, until it says what it did.
    Mine(Claim<'a>),
}

/// A send's hold on its key. Once the send says what it did, a retry is answered with that;
/// dropped first, because the send neither started a turn nor queued its message (it was
/// refused, or its request went away), it lets the key go, so a retry is sent anew.
pub(crate) struct Claim<'a> {
    keyed: &'a Keyed,
    person: Person,
    thread: ThreadId,
    key: IdempotencyKey,
    /// The turn the send would start, which its entry names until it says.
    done: Done,
    said: bool,
}

impl Keyed {
    /// What `person`'s send to `thread` with `key` already did, when one came first. Otherwise
    /// the key is this send's, `done` the turn it would start, in the same step, so two retries
    /// racing each other start one turn.
    pub(crate) fn claim(
        &self,
        person: Person,
        thread: ThreadId,
        key: &IdempotencyKey,
        done: &Done,
    ) -> Claimed<'_> {
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
        let before = pool
            .recent
            .iter()
            .find(|(of, kept, _)| *of == thread && kept == key);
        if let Some((_, _, sent)) = before {
            return Claimed::Before(sent.clone());
        }
        if pool.recent.len() >= KEPT {
            pool.recent.pop_front();
        }
        pool.recent
            .push_back((thread, key.clone(), Kept::Turn(done.clone())));
        Claimed::Mine(Claim {
            keyed: self,
            person,
            thread,
            key: key.clone(),
            done: done.clone(),
            said: false,
        })
    }

    /// Keeps that `claim`'s send `queued` its message, for a retry to be answered with.
    fn keep_queued(&self, claim: &Claim<'_>, queued: QueuedMessage) {
        let mut people = self.people.lock().unwrap_or_else(PoisonError::into_inner);
        let entry = people
            .pools
            .get_mut(&claim.person)
            .and_then(|pool| pool.recent.iter_mut().find(|entry| claim.names(entry)));
        if let Some((_, _, sent)) = entry {
            *sent = Kept::Queued(queued);
        }
    }

    /// Lets `claim`'s key go, so a retry is sent anew.
    fn let_go(&self, claim: &Claim<'_>) {
        let mut people = self.people.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(pool) = people.pools.get_mut(&claim.person) {
            pool.recent.retain(|entry| !claim.names(entry));
            if pool.recent.is_empty() {
                people.pools.remove(&claim.person);
            }
        }
    }
}

impl Claim<'_> {
    /// The send started the turn it claimed the key for: a retry follows that turn.
    pub(crate) fn started(mut self) {
        self.said = true;
    }

    /// The send `queued` its message to the turn running instead: a retry is answered with
    /// the same id.
    pub(crate) fn queued(mut self, queued: QueuedMessage) {
        self.keyed.keep_queued(&self, queued);
        self.said = true;
    }

    /// Whether `entry` is this claim's: its key, for the turn it would start.
    fn names(&self, (thread, key, sent): &(ThreadId, IdempotencyKey, Kept)) -> bool {
        *thread == self.thread
            && *key == self.key
            && matches!(sent, Kept::Turn(started) if started.same_channel(&self.done))
    }
}

impl Drop for Claim<'_> {
    fn drop(&mut self) {
        if !self.said {
            self.keyed.let_go(self);
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

    /// Whether the key was this send's, which then started its turn.
    fn started(claimed: Claimed<'_>) -> bool {
        match claimed {
            Claimed::Mine(claim) => {
                claim.started();
                true
            }
            Claimed::Before(_) => false,
        }
    }

    /// The turn the send that came first with the key started.
    fn followed(claimed: Claimed<'_>) -> Done {
        match claimed {
            Claimed::Before(Kept::Turn(done)) => done,
            _ => panic!("a turn sent first"),
        }
    }

    #[test]
    fn a_key_names_one_turn_per_thread_and_a_persons_memory_is_bounded() {
        let keyed = Keyed::default();
        let (_first, done) = watch::channel(None);
        assert!(started(keyed.claim(None, thread(1), &key("a"), &done)));
        let (_second, other) = watch::channel(None);
        let again = followed(keyed.claim(None, thread(1), &key("a"), &other));
        assert!(again.same_channel(&done));
        // The same key in another thread is another turn.
        assert!(started(keyed.claim(None, thread(2), &key("a"), &other)));
        for n in 0..KEPT {
            started(keyed.claim(None, thread(3), &key(&n.to_string()), &other));
        }
        assert!(
            started(keyed.claim(None, thread(2), &key("a"), &done)),
            "the oldest was let go"
        );
    }

    #[test]
    fn a_send_that_neither_started_a_turn_nor_queued_lets_its_key_go() {
        let keyed = Keyed::default();
        let (_first, done) = watch::channel(None);
        // Refused, or its request went away: its claim drops unsaid.
        drop(keyed.claim(None, thread(1), &key("a"), &done));
        assert!(keyed.people.lock().unwrap().pools.is_empty());
        assert!(
            started(keyed.claim(None, thread(1), &key("a"), &done)),
            "a retry is sent anew"
        );
    }

    #[test]
    fn one_persons_flood_never_evicts_anothers_key() {
        let keyed = Keyed::default();
        let (_theirs, done) = watch::channel(None);
        assert!(started(keyed.claim(
            Some(person(1)),
            thread(1),
            &key("a"),
            &done
        )));
        let (_flood, other) = watch::channel(None);
        for n in 0..KEPT * 4 {
            started(keyed.claim(Some(person(2)), thread(1), &key(&n.to_string()), &other));
        }
        let again = followed(keyed.claim(Some(person(1)), thread(1), &key("a"), &other));
        assert!(again.same_channel(&done));
        // Nor does someone else's same key name their turn.
        assert!(started(keyed.claim(
            Some(person(3)),
            thread(1),
            &key("a"),
            &other
        )));
    }

    #[test]
    fn a_key_that_queued_its_message_is_answered_with_the_same_id() {
        let keyed = Keyed::default();
        let (_first, done) = watch::channel(None);
        let Claimed::Mine(claim) = keyed.claim(None, thread(1), &key("a"), &done) else {
            panic!("the first send's key");
        };
        let id = glave_event::EventId::new(jiff::Timestamp::UNIX_EPOCH, [7; 10]).unwrap();
        let queued = QueuedMessage { id };
        claim.queued(queued);
        let (_retried, other) = watch::channel(None);
        for _ in 0..2 {
            let again = keyed.claim(None, thread(1), &key("a"), &other);
            assert!(matches!(again, Claimed::Before(Kept::Queued(first)) if first == queued));
        }
    }

    #[test]
    fn the_person_who_claimed_least_recently_is_let_go_beyond_the_cap() {
        let keyed = Keyed::default();
        let (_held, done) = watch::channel(None);
        for n in 0..PEOPLE {
            started(keyed.claim(Some(person(n)), thread(1), &key("a"), &done));
        }
        // Claimed again, the first person is now the most recent, so the second is let go.
        assert!(!started(keyed.claim(
            Some(person(0)),
            thread(1),
            &key("a"),
            &done
        )));
        started(keyed.claim(Some(person(PEOPLE)), thread(1), &key("a"), &done));
        let people = keyed.people.lock().unwrap();
        assert_eq!(people.pools.len(), PEOPLE);
        assert!(people.pools.contains_key(&Some(person(0))));
        assert!(!people.pools.contains_key(&Some(person(1))));
    }
}
