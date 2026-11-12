//! Credits (`specs/gateway.md`, "Credits"): each organization's money, paid ahead, that its
//! model calls spend at the provider's own cost. Kept in its credits ledger, one stream of the
//! organization's own in the store: money added (`credits.added`, once per payment), the
//! month's limit its Owner set (`credits.limit.set`), and each charge (`credits.charged`).
//! Before a call is sent its worst case is held against what is left of the balance and of the
//! month's limit, in the ledger (`credits.held`), so every server on the store counts it; when
//! it ends, what it cost is counted in this server's book at once, in the hold's place, and
//! written to the store after, its charge letting go of the hold, so a charge whose writing
//! fails or is cancelled is still counted, and written with the next. Any number of servers
//! may share one store: a hold is appended only after everything before it was read, so of
//! servers racing for the last of a balance one holds it and the others read its hold first.

mod book;
mod paying;

use std::collections::BTreeMap;
use std::fmt;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use glave_event::{
    At, CostKind, CreditsAdded, CreditsCharged, CreditsHeld, CreditsLimitSet, Currency, EventId,
    Header, Money, Nanos, Payload, Principal, RawEvent, Seq, encode,
};
use glave_ports::stream::{Reserved, StreamError, append_retrying, catch_up};
use glave_ports::{
    BoxFuture, Clock, Entropy, EventLog, Failure, OrgId, Payer, ProviderError, Scope,
};
use thiserror::Error;

pub use book::Account;
use book::{Book, Unwritten, dollars};
pub(crate) use paying::Paying;

/// Charges a server keeps counted for one organization while the store does not take them.
/// Past this, its calls are refused until it does.
const MOST_UNWRITTEN: usize = 10_000;

/// Organizations whose books a server keeps at once. Past this, the least recently used with
/// no call holding anything of it and nothing to write is forgotten, and read again when next
/// asked for.
const BOOKS: usize = 4_096;

/// Why the credits ledger could not be read or written, or a payment added.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum CreditsError {
    #[error(transparent)]
    Stream(#[from] StreamError),
    /// An event for it could not be made.
    #[error("could not record the credits: {0}")]
    Event(String),
    /// The payment was added already, for another amount.
    #[error("the payment {reference} was added already, for {amount:.2}")]
    Added { reference: String, amount: Money },
    /// Credits are US dollars: an amount added is more than nothing, a limit nothing or more.
    #[error("{amount} is not an amount of dollars credits take")]
    Amount { amount: Money },
}

/// Every organization's book this server keeps, and the count each use takes.
#[derive(Debug, Default)]
struct Books {
    kept: BTreeMap<OrgId, Book>,
    uses: u64,
}

/// Every organization's credits, on one store.
pub struct Credits<L> {
    log: L,
    clock: Box<dyn Clock>,
    entropy: Box<dyn Entropy>,
    /// The monthly limit an organization has until its Owner sets one: the server's own.
    limit: Option<Money>,
    books: Mutex<Books>,
    /// Each organization's turn to read its ledger and append to it, while anything of this
    /// server's is: calls on one server queue rather than race each other for the next place,
    /// so appends retry only against other servers. One is kept only while someone holds it.
    turns: Mutex<BTreeMap<OrgId, Arc<tokio::sync::Mutex<()>>>>,
}

impl<L> fmt::Debug for Credits<L> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Credits")
            .field("limit", &self.limit)
            .finish_non_exhaustive()
    }
}

/// The scope of `org`'s credits ledger: glave's own, the organization's, no project's. What a
/// caller asks of it was checked against their role first; a call charges the organization
/// its thread is in.
fn scope(org: OrgId) -> Scope {
    Scope::of_organization(org)
}

/// `payload` as the credits ledger's event `id` at `seq`, written `at`.
fn event<T: Payload>(seq: u64, id: EventId, at: At, payload: &T) -> Result<RawEvent, CreditsError> {
    let failed = |error: &dyn fmt::Display| CreditsError::Event(error.to_string());
    let header = Header {
        id,
        thread: Reserved::Credits.id(),
        seq: Seq::new(seq).map_err(|e| failed(&e))?,
        at,
    };
    encode(header, payload).map_err(|e| failed(&e))
}

impl<L> Credits<L> {
    fn books(&self) -> MutexGuard<'_, Books> {
        self.books.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// `look` at `org`'s book, made empty when it has none; past [`BOOKS`], the least
    /// recently used idle one is forgotten first.
    fn with_book<R>(&self, org: OrgId, look: impl FnOnce(&mut Book) -> R) -> R {
        let mut books = self.books();
        if books.kept.len() >= BOOKS && !books.kept.contains_key(&org) {
            let idle = books
                .kept
                .iter()
                .filter(|(_, book)| book.idle())
                .min_by_key(|(_, book)| book.used)
                .map(|(org, _)| *org);
            if let Some(idle) = idle {
                books.kept.remove(&idle);
            }
        }
        books.uses = books.uses.saturating_add(1);
        let used = books.uses;
        let book = books.kept.entry(org).or_default();
        book.used = used;
        look(book)
    }

    /// Lets go of `payer`'s organization's credits held by `hold`, and counts `cost` in its
    /// place, at once; written, the charge lets go of the hold for every server.
    fn settle(&self, payer: Payer, hold: Option<EventId>, cost: Nanos) {
        let charged = (cost > Nanos::ZERO || hold.is_some()).then(|| Unwritten {
            charged: CreditsCharged {
                kind: CostKind::Model,
                amount: dollars(cost),
                person: payer.person,
                hold,
            },
            at: At::from(self.clock.now()),
            id: None,
        });
        self.with_book(payer.org, |book| book.settle(hold, charged));
    }
}

impl<L: EventLog> Credits<L> {
    /// The credits kept in `log`, each organization limited to `limit` a month until its Owner
    /// sets another.
    pub fn new(
        log: L,
        clock: Box<dyn Clock>,
        entropy: Box<dyn Entropy>,
        limit: Option<Money>,
    ) -> Self {
        Self {
            log,
            clock,
            entropy,
            limit,
            books: Mutex::default(),
            turns: Mutex::default(),
        }
    }

    /// `org`'s credits as its ledger says now, this server's charges not yet written counted
    /// in.
    pub async fn account(&self, org: OrgId) -> Result<Account, CreditsError> {
        self.catch_up(org).await?;
        Ok(self.with_book(org, |book| book.account(self.clock.now(), self.limit)))
    }

    /// Adds `amount` to `org`'s credits, paid by the payment `reference` names, unless that
    /// payment was added already; answers the account then. The same payment for another
    /// amount is refused.
    pub async fn add(
        &self,
        org: OrgId,
        amount: Money,
        reference: &str,
        by: Principal,
    ) -> Result<Account, CreditsError> {
        if amount.currency != Currency::USD || amount.nanos <= Nanos::ZERO {
            return Err(CreditsError::Amount { amount });
        }
        self.record(org, |book| match book.added(reference) {
            Some(added) if added == amount => Ok(None),
            Some(added) => Err(CreditsError::Added {
                reference: reference.to_owned(),
                amount: added,
            }),
            None => Ok(Some(CreditsAdded {
                amount,
                reference: reference.to_owned(),
                by: by.clone(),
            })),
        })
        .await?;
        self.account(org).await
    }

    /// Sets the most `org` may spend in a month; answers the account then.
    pub async fn set_limit(
        &self,
        org: OrgId,
        limit: Money,
        by: Principal,
    ) -> Result<Account, CreditsError> {
        if limit.currency != Currency::USD || limit.nanos < Nanos::ZERO {
            return Err(CreditsError::Amount { amount: limit });
        }
        self.record(org, |_| {
            Ok(Some(CreditsLimitSet {
                limit,
                by: by.clone(),
            }))
        })
        .await?;
        self.account(org).await
    }

    /// Waits for this server's turn to read `org`'s ledger and append to it, which lasts until
    /// the answer is dropped: a read of what is new and one append, each as long as the store
    /// takes.
    async fn turn(&self, org: OrgId) -> Turn<'_, L> {
        let lock = {
            let mut turns = self.turns.lock().unwrap_or_else(PoisonError::into_inner);
            Arc::clone(turns.entry(org).or_default())
        };
        let guard = Arc::clone(&lock).lock_owned().await;
        Turn {
            credits: self,
            org,
            guard: Some(guard),
        }
    }

    /// Reads whatever was added to `org`'s ledger since this server last looked.
    async fn catch_up(&self, org: OrgId) -> Result<(), CreditsError> {
        let next = self.with_book(org, |book| book.next_seq());
        let after = Seq::new(next.saturating_sub(1)).ok();
        let ledger = Reserved::Credits.id();
        catch_up(&self.log, &scope(org), ledger, after, |page| {
            self.with_book(org, |book| page.into_iter().for_each(|raw| book.apply(raw)));
        })
        .await?;
        Ok(())
    }

    /// Appends what `make` makes of `org`'s book as it is then, after anything others wrote
    /// meanwhile: nothing when it makes nothing.
    async fn record<T: Payload + Send>(
        &self,
        org: OrgId,
        make: impl Fn(&Book) -> Result<Option<T>, CreditsError> + Sync,
    ) -> Result<(), CreditsError> {
        let ledger = Reserved::Credits.id();
        let make = &make;
        let _turn = self.turn(org).await;
        let appended = append_retrying(&self.log, &scope(org), ledger, || async move {
            self.catch_up(org).await?;
            let (next, payload) = self.with_book(org, |book| (book.next_seq(), make(book)));
            let Some(payload) = payload? else {
                return Ok(None);
            };
            let now = At::from(self.clock.now());
            let event = event(next, self.fresh_id(now)?, now, &payload)?;
            Ok::<_, CreditsError>(Some((vec![event], ())))
        })
        .await?;
        self.with_book(org, |book| {
            for raw in appended.into_iter().flat_map(|(events, ())| events) {
                book.apply(raw);
            }
        });
        Ok(())
    }

    /// Writes the charges this server counted for `org` that the store does not have yet, in
    /// one append, after anything others wrote meanwhile. One that fails stays counted, for the
    /// next.
    async fn write(&self, org: OrgId) {
        let (scope, ledger) = (scope(org), Reserved::Credits.id());
        let turn = self.turn(org).await;
        let appended = append_retrying(&self.log, &scope, ledger, || async move {
            self.catch_up(org).await?;
            let (_, pending) = self.pending(org)?;
            Ok::<_, CreditsError>((!pending.is_empty()).then_some((pending, ())))
        });
        let appended = appended.await;
        drop(turn);
        match appended {
            Ok(appended) => self.took(org, appended.map(|(events, ())| events)),
            Err(error) => {
                tracing::warn!(%org, "could not write the organization's charges yet: {error}");
            }
        }
    }

    /// The events of the charges this server counted for `org` that the store does not have
    /// yet, numbered from its next `seq`, and the `seq` after them. Each charge keeps the id it
    /// is first written with, so one whose append landed unseen, its writer cancelled or told
    /// it failed, is read back in its count's place rather than written twice.
    fn pending(&self, org: OrgId) -> Result<(u64, Vec<RawEvent>), CreditsError> {
        let (next, pending) = self.with_book(org, |book| {
            for unwritten in &mut book.unwritten {
                if unwritten.id.is_none() {
                    unwritten.id = self.fresh_id(unwritten.at).ok();
                }
            }
            (book.next_seq(), book.unwritten.clone())
        });
        let events = (next..)
            .zip(&pending)
            .map(|(seq, unwritten)| {
                let id = unwritten
                    .id
                    .map_or_else(|| self.fresh_id(unwritten.at), Ok)?;
                event(seq, id, unwritten.at, &unwritten.charged)
            })
            .collect::<Result<Vec<RawEvent>, CreditsError>>()?;
        let after = next.saturating_add(u64::try_from(events.len()).unwrap_or(u64::MAX));
        Ok((after, events))
    }

    /// Takes into `org`'s book the events this server `appended`.
    fn took(&self, org: OrgId, appended: Option<Vec<RawEvent>>) {
        self.with_book(org, |book| {
            appended
                .into_iter()
                .flatten()
                .for_each(|raw| book.apply(raw));
        });
    }

    /// A new event's id, written `at`.
    fn fresh_id(&self, at: At) -> Result<EventId, CreditsError> {
        let failed = |error: &dyn fmt::Display| CreditsError::Event(error.to_string());
        let random = self.entropy.bytes().map_err(|e| failed(&e))?;
        EventId::new(at.timestamp(), random).map_err(|e| failed(&e))
    }

    /// Holds `worst` of `org`'s credits in its ledger, or refuses it: in one append, after
    /// everything others wrote was read, with what this server owes the store before it, so a
    /// hold costs one read of what is new and one small write. The hold's id, when anything
    /// was held. A hold whose append may have landed unseen (the store's answer was lost, or
    /// this was dropped meanwhile) is let go of by a charge of nothing naming it, written at
    /// once or with the organization's next call.
    async fn hold(&self, org: OrgId, worst: Nanos) -> Result<Option<EventId>, ProviderError> {
        let unsaved = self.with_book(org, |book| book.unwritten.len());
        let now = self.clock.now();
        let at = At::from(now);
        let ledger = Reserved::Credits.id();
        let id = self
            .fresh_id(at)
            .map_err(|error| unreachable_credits(org, &error))?;
        let attempt = Attempt::new(self, org, id);
        let turn = self.turn(org).await;
        let appended = if unsaved >= MOST_UNWRITTEN {
            Err(CreditsError::Event(
                "its charges could not be saved yet".to_owned(),
            ))
        } else {
            let attempt = &attempt;
            append_retrying(&self.log, &scope(org), ledger, || async move {
                self.catch_up(org).await?;
                self.with_book(org, |book| book.prune(now));
                let (next, mut events) = self.pending(org)?;
                let may = self.with_book(org, |book| book.may_hold(worst, now, self.limit));
                let hold = match may {
                    Ok(()) if worst > Nanos::ZERO => {
                        let held = CreditsHeld {
                            amount: dollars(worst),
                        };
                        events.push(event(next, id, at, &held)?);
                        attempt.tried.store(true, Ordering::SeqCst);
                        Some(id)
                    }
                    _ => None,
                };
                Ok::<_, CreditsError>((!events.is_empty()).then_some((events, (may, hold))))
            })
            .await
        };
        drop(turn);
        let made = match appended {
            Ok(Some((events, made))) => {
                self.took(org, Some(events));
                made
            }
            // Nothing to write: nothing owed, and nothing to hold or the call refused.
            Ok(None) => (
                self.with_book(org, |book| book.may_hold(worst, now, self.limit)),
                None,
            ),
            Err(error) => {
                // Let go of here, and written at once if the store takes it now.
                drop(attempt);
                self.write(org).await;
                return Err(unreachable_credits(org, &error));
            }
        };
        match made {
            (Ok(()), Some(hold)) => {
                attempt.kept();
                Ok(Some(hold))
            }
            (Ok(()), None) => {
                attempt.never_landed();
                Ok(None)
            }
            (Err(unpaid), _) => {
                attempt.never_landed();
                Err(ProviderError::Unpaid(unpaid))
            }
        }
    }
}

/// Why a call is refused when `org`'s credits could not be read or saved, which pay for
/// nothing; the store may answer later.
fn unreachable_credits(org: OrgId, error: &dyn fmt::Display) -> ProviderError {
    tracing::warn!(%org, "the organization's credits are out of reach: {error}");
    ProviderError::Unserved {
        failure: Failure::Rejected,
        reason: "glave could not reach your credits just now: try again".to_owned(),
    }
}

/// This server's turn at an organization's ledger; let go of when dropped, and forgotten
/// once no one else waits for it.
struct Turn<'a, L> {
    credits: &'a Credits<L>,
    org: OrgId,
    guard: Option<tokio::sync::OwnedMutexGuard<()>>,
}

impl<L> Drop for Turn<'_, L> {
    fn drop(&mut self) {
        self.guard.take();
        let mut turns = self
            .credits
            .turns
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if turns
            .get(&self.org)
            .is_some_and(|lock| Arc::strong_count(lock) == 1)
        {
            turns.remove(&self.org);
        }
    }
}

/// A hold being made, counted as a call of this server's from before its append, so that
/// however the making ends (dropped, or the store's answer lost) one that may have landed is
/// let go of by a charge of nothing naming it.
struct Attempt<'a, L> {
    credits: &'a Credits<L>,
    org: OrgId,
    id: EventId,
    /// Whether an append that holds it was tried.
    tried: AtomicBool,
    /// Whether it landed, for its call to settle.
    kept: bool,
}

impl<'a, L> Attempt<'a, L> {
    fn new(credits: &'a Credits<L>, org: OrgId, id: EventId) -> Self {
        credits.with_book(org, |book| book.running.insert(id));
        Self {
            credits,
            org,
            id,
            tried: AtomicBool::new(false),
            kept: false,
        }
    }

    /// It landed: its call settles it.
    fn kept(mut self) {
        self.kept = true;
    }

    /// Nothing that holds it landed.
    fn never_landed(self) {
        self.tried.store(false, Ordering::SeqCst);
    }
}

impl<L> Drop for Attempt<'_, L> {
    fn drop(&mut self) {
        if self.kept {
            return;
        }
        if self.tried.load(Ordering::SeqCst) {
            let payer = Payer {
                org: self.org,
                person: None,
            };
            self.credits.settle(payer, Some(self.id), Nanos::ZERO);
        } else {
            let id = self.id;
            self.credits
                .with_book(self.org, |book| book.running.remove(&id));
        }
    }
}

/// What the gateway asks of the credits, whatever log keeps them: a trait only so that the
/// gateway, which keeps no log, need not name the log's type.
pub(crate) trait Charging: Send + Sync + fmt::Debug {
    /// Holds `worst`, in dollars, of `org`'s credits for a call, or refuses it: the hold's
    /// id, when anything was held.
    fn hold(
        &self,
        org: OrgId,
        worst: Nanos,
    ) -> BoxFuture<'_, Result<Option<EventId>, ProviderError>>;
    /// Lets go of `hold` and counts `cost`, in dollars, in its place, at once.
    fn settle(&self, payer: Payer, hold: Option<EventId>, cost: Nanos);
    /// Writes what was counted for `org` and the store does not have yet.
    fn write(&self, org: OrgId) -> BoxFuture<'_, ()>;
}

impl<L: EventLog> Charging for Credits<L> {
    fn hold(
        &self,
        org: OrgId,
        worst: Nanos,
    ) -> BoxFuture<'_, Result<Option<EventId>, ProviderError>> {
        Box::pin(Self::hold(self, org, worst))
    }

    fn settle(&self, payer: Payer, hold: Option<EventId>, cost: Nanos) {
        Self::settle(self, payer, hold, cost);
    }

    fn write(&self, org: OrgId) -> BoxFuture<'_, ()> {
        Box::pin(Self::write(self, org))
    }
}

#[cfg(test)]
mod tests {
    use glave_event::PrincipalId;
    use glave_ports::InMemoryLog;
    use glave_ports::testing::{CountingEntropy, ManualClock};

    use super::*;

    fn org(n: usize) -> OrgId {
        let second = i64::try_from(n).unwrap();
        let at = jiff::Timestamp::from_second(second).unwrap();
        OrgId::from_uuid(PrincipalId::new(at, [1; 10]).unwrap().as_uuid())
    }

    fn event_id(n: i64) -> EventId {
        EventId::new(jiff::Timestamp::from_second(n).unwrap(), [2; 10]).unwrap()
    }

    #[test]
    fn past_its_cap_the_least_recently_used_idle_book_is_forgotten_never_one_in_use() {
        let credits = Credits::new(
            InMemoryLog::new(),
            Box::new(ManualClock::at_second(1)),
            Box::new(CountingEntropy::starting_at(1)),
            None,
        );
        credits.with_book(org(0), |book| book.running.insert(event_id(0)));
        for n in 1..BOOKS {
            credits.with_book(org(n), |_| ());
        }
        // The second is used again, so the third is now the least recently used idle one.
        credits.with_book(org(1), |_| ());
        credits.with_book(org(BOOKS), |_| ());
        let kept = &credits.books().kept;
        assert_eq!(kept.len(), BOOKS);
        assert!(kept.contains_key(&org(0)), "it holds money for a call");
        assert!(kept.contains_key(&org(1)), "used again since");
        assert!(
            !kept.contains_key(&org(2)),
            "the least recently used idle one"
        );
    }
}
