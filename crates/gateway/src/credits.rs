//! Credits (`specs/gateway.md`, "Credits"): each organization's money, paid ahead, that its
//! model calls spend at the provider's own cost. Kept in its credits ledger, one stream of the
//! organization's own in the store: money added (`credits.added`, once per payment), the
//! month's limit its Owner set (`credits.limit.set`), and each charge (`credits.charged`).
//! Before a call is sent its worst case is held against what is left of the balance and of the
//! month's limit; when it ends, what it cost is counted in this server's book at once, in the
//! hold's place, and written to the store after, so a charge whose writing fails or is
//! cancelled is still counted, and written with the next. Any number of servers may share one
//! store: each reads what the others charged before it holds, and appends after anything they
//! wrote meanwhile.

mod book;
mod paying;

use std::collections::BTreeMap;
use std::fmt;
use std::sync::{Mutex, MutexGuard, PoisonError};

use glave_event::{
    At, CostKind, CreditsAdded, CreditsCharged, CreditsLimitSet, EventId, Header, Money, Payload,
    Principal, RawEvent, Seq, encode,
};
use glave_ports::stream::{Reserved, StreamError, append_retrying, catch_up};
use glave_ports::{
    BoxFuture, Clock, Entropy, EventLog, Failure, OrgId, Payer, ProviderError, Scope,
};
use thiserror::Error;

pub use book::Account;
use book::{Book, Unwritten, dollars, month_of};
pub(crate) use paying::Paying;

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
        self.record(org, |_| {
            Ok(Some(CreditsLimitSet {
                limit,
                by: by.clone(),
            }))
        })
        .await?;
        self.account(org).await
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
        let appended = append_retrying(&self.log, &scope(org), ledger, || async move {
            self.catch_up(org).await?;
            let (next, payload) = self.with_book(org, |book| (book.next_seq(), make(book)));
            let Some(payload) = payload? else {
                return Ok(None);
            };
            let now = At::from(self.clock.now());
            Ok::<_, CreditsError>(Some((vec![self.event(next, now, &payload)?], ())))
        })
        .await?;
        self.with_book(org, |book| {
            for raw in appended.into_iter().flat_map(|(events, ())| events) {
                book.apply(raw);
            }
        });
        Ok(())
    }

    /// Writes the charges this server counted for `org` that the store does not have yet, as
    /// one append each time, until none is left or another write here is under way. One that
    /// fails is kept, counted, for the next.
    async fn write(&self, org: OrgId) {
        loop {
            let taken = self.with_book(org, |book| {
                if book.writing > 0 || book.unwritten.is_empty() {
                    return None;
                }
                book.writing = book.unwritten.len();
                Some(book.unwritten.clone())
            });
            let Some(charges) = taken else {
                return;
            };
            let _writing = Writing { credits: self, org };
            let ledger = Reserved::Credits.id();
            let charges = &charges;
            let scope = scope(org);
            let appended = append_retrying(&self.log, &scope, ledger, || async move {
                self.catch_up(org).await?;
                let next = self.with_book(org, |book| book.next_seq());
                let events = (next..)
                    .zip(charges)
                    .map(|(seq, unwritten)| self.event(seq, unwritten.at, &unwritten.charged))
                    .collect::<Result<Vec<RawEvent>, CreditsError>>()?;
                Ok::<_, CreditsError>(Some((events, ())))
            });
            match appended.await {
                Ok(appended) => {
                    let events = appended.map(|(events, ())| events).unwrap_or_default();
                    self.with_book(org, |book| book.written(charges.len(), events));
                }
                Err(error) => {
                    tracing::warn!(%org, "could not write the organization's charges yet: {error}");
                    return;
                }
            }
        }
    }

    /// `payload` as the credits ledger's event at `seq`, written `at`.
    fn event<T: Payload>(&self, seq: u64, at: At, payload: &T) -> Result<RawEvent, CreditsError> {
        let failed = |error: &dyn fmt::Display| CreditsError::Event(error.to_string());
        let random = self.entropy.bytes().map_err(|e| failed(&e))?;
        let header = Header {
            id: EventId::new(at.timestamp(), random).map_err(|e| failed(&e))?,
            thread: Reserved::Credits.id(),
            seq: Seq::new(seq).map_err(|e| failed(&e))?,
            at,
        };
        encode(header, payload).map_err(|e| failed(&e))
    }

    /// Holds `worst` of `org`'s credits once what this server owes the store is written and
    /// the others' charges are read, or refuses it.
    async fn hold(&self, org: OrgId, worst: i64) -> Result<(), ProviderError> {
        self.write(org).await;
        if let Err(error) = self.catch_up(org).await {
            // Credits that cannot be read pay for nothing; the store may answer next time.
            tracing::warn!(%org, "could not read the organization's credits: {error}");
            return Err(ProviderError::Unserved {
                failure: Failure::Unavailable,
                reason: "glave could not read your credits just now: try again".to_owned(),
            });
        }
        let month = month_of(self.clock.now());
        self.with_book(org, |book| book.hold(worst, month, self.limit))
            .map_err(ProviderError::Unpaid)
    }

    /// Lets go of `held` of `payer`'s organization's credits and counts `cost` in its place.
    fn settle(&self, payer: Payer, held: i64, cost: i64) {
        let charged = (cost > 0).then(|| Unwritten {
            charged: CreditsCharged {
                kind: CostKind::Model,
                amount: dollars(cost),
                person: payer.person,
            },
            at: At::from(self.clock.now()),
        });
        self.with_book(payer.org, |book| book.settle(held, charged));
    }
}

/// A write of an organization's charges under way; it ends however its writing does.
struct Writing<'a, L> {
    credits: &'a Credits<L>,
    org: OrgId,
}

impl<L> Drop for Writing<'_, L> {
    fn drop(&mut self) {
        self.credits.with_book(self.org, |book| book.writing = 0);
    }
}

/// What the gateway asks of the credits, whatever log keeps them: a trait only so that the
/// gateway, which keeps no log, need not name the log's type.
pub(crate) trait Charging: Send + Sync + fmt::Debug {
    /// Holds `worst` millionths of a dollar of `org`'s credits for a call, or refuses it.
    fn hold(&self, org: OrgId, worst: i64) -> BoxFuture<'_, Result<(), ProviderError>>;
    /// Lets go of `held` and counts `cost` millionths of a dollar in its place, at once.
    fn settle(&self, payer: Payer, held: i64, cost: i64);
    /// Writes what was counted for `org` and the store does not have yet.
    fn write(&self, org: OrgId) -> BoxFuture<'_, ()>;
}

impl<L: EventLog> Charging for Credits<L> {
    fn hold(&self, org: OrgId, worst: i64) -> BoxFuture<'_, Result<(), ProviderError>> {
        Box::pin(Self::hold(self, org, worst))
    }

    fn settle(&self, payer: Payer, held: i64, cost: i64) {
        Self::settle(self, payer, held, cost);
    }

    fn write(&self, org: OrgId) -> BoxFuture<'_, ()> {
        Box::pin(Self::write(self, org))
    }
}
