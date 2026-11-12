//! What one organization's credits ledger says, as far as it was read: its holds among it, each
//! call's worst case, whichever server holds it; the charges this server made that the store
//! does not have yet, counted already; and which holds are this server's calls still running. Amounts are billionths of a US dollar, kept exact: rounding to cents is for showing
//! them, never here. Anything in another currency is no part of it.

use std::collections::{BTreeMap, BTreeSet};

use glave_event::{
    At, Body, CreditsCharged, Currency, EventId, Money, Nanos, PrincipalId, RawEvent,
};
use glave_ports::Unpaid;
use glave_ports::stream::in_order;
use jiff::SignedDuration;
use jiff::Timestamp;
use jiff::tz::TimeZone;

/// An organization's credits as its ledger says now, with this server's charges not yet in it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Account {
    /// Everything added, less everything charged: below nothing only where calls cost more
    /// than they held.
    pub balance: Money,
    /// The most it may spend in a month, if anything limits it: its Owner's, else the
    /// server's own.
    pub limit: Option<Money>,
    /// What it spent this month.
    pub spent: Money,
    /// Each person's part of this month's spending, by whose work it was.
    pub people: BTreeMap<PrincipalId, Money>,
    /// When this month began, in UTC.
    pub since: Timestamp,
}

/// A calendar month in UTC, counted from the year 0.
type Month = i32;

/// The month `at` falls in.
fn month_of(at: Timestamp) -> Month {
    let date = at.to_zoned(TimeZone::UTC).date();
    i32::from(date.year()) * 12 + i32::from(date.month()) - 1
}

/// When the month `at` falls in began, in UTC.
fn month_start(at: Timestamp) -> Timestamp {
    let first = at.to_zoned(TimeZone::UTC).date().first_of_month();
    first
        .to_zoned(TimeZone::UTC)
        .map_or(at, |start| start.timestamp())
}

/// `nanos` billionths of a dollar.
pub(super) fn dollars(nanos: Nanos) -> Money {
    Money {
        nanos,
        currency: Currency::USD,
    }
}

/// `money` in billionths of a dollar, when it is in dollars.
pub(super) fn in_dollars(money: Money) -> Option<Nanos> {
    (money.currency == Currency::USD).then_some(money.nanos)
}

/// One month's spending: everyone's, and each person's part.
#[derive(Debug, Default, Clone)]
struct Spending {
    month: Month,
    all: Nanos,
    people: BTreeMap<PrincipalId, Nanos>,
}

impl Spending {
    /// Counts `amount` spent `at` for `person`'s work, a new month starting from nothing.
    fn add(&mut self, at: At, amount: Nanos, person: Option<PrincipalId>) {
        let month = month_of(at.timestamp());
        if month < self.month {
            return;
        }
        if month > self.month {
            *self = Self {
                month,
                ..Self::default()
            };
        }
        self.all = self.all.saturating_add(amount);
        if let Some(person) = person {
            let theirs = self.people.entry(person).or_default();
            *theirs = theirs.saturating_add(amount);
        }
    }
}

/// How long a hold counts for every server but the one whose call it is: past this, a hold
/// whose charge never came (its server stopped mid-call) lets go of what it held. The server
/// whose call it is counts it until the call ends, however long that is.
pub(super) const LAPSE: SignedDuration = SignedDuration::from_hours(1);

/// A call's worst case held in the ledger, and when.
#[derive(Debug, Clone, Copy)]
struct Held {
    amount: Nanos,
    at: Timestamp,
}

/// A charge this server made, counted, and not yet in the store: its event's id once a write
/// was first tried, so the event read back takes its place, however that write ended.
#[derive(Debug, Clone)]
pub(super) struct Unwritten {
    pub(super) charged: CreditsCharged,
    pub(super) at: At,
    pub(super) id: Option<EventId>,
}

/// One organization's book.
#[derive(Debug, Default)]
pub(super) struct Book {
    /// The `seq` the next event will take.
    next: u64,
    /// What the ledger says is left.
    balance: Nanos,
    /// The latest limit its Owner set, if any.
    limit: Option<Money>,
    /// The payments added, by reference, with how much each added.
    references: BTreeMap<String, Money>,
    /// What the ledger says was spent in its latest month.
    spending: Spending,
    /// The holds the ledger says no charge let go of yet, by their events' ids.
    holds: BTreeMap<EventId, Held>,
    /// This server's charges the store does not have yet, oldest first.
    pub(super) unwritten: Vec<Unwritten>,
    /// The holds of this server's calls still running, which never lapse here.
    pub(super) running: BTreeSet<EventId>,
    /// When it was last used, by the books' own count: the least recent goes first.
    pub(super) used: u64,
}

impl Book {
    pub(super) fn next_seq(&self) -> u64 {
        self.next.max(1)
    }

    /// Takes in the event at the next `seq`; any other is already in, or not yet. A charge of
    /// this server's own read back takes the place of its count.
    pub(super) fn apply(&mut self, raw: RawEvent) {
        let id = raw.id;
        let Some((at, body)) = in_order(&mut self.next, raw) else {
            return;
        };
        self.unwritten.retain(|unwritten| unwritten.id != Some(id));
        match body {
            Body::CreditsAdded(added) => {
                if let Some(amount) = in_dollars(added.amount) {
                    self.balance = self.balance.saturating_add(amount);
                }
                self.references.insert(added.reference, added.amount);
            }
            Body::CreditsLimitSet(set) => self.limit = Some(set.limit),
            Body::CreditsHeld(held) => {
                if let Some(amount) = in_dollars(held.amount) {
                    let at = at.timestamp();
                    self.holds.insert(id, Held { amount, at });
                }
            }
            Body::CreditsCharged(charged) => {
                if let Some(hold) = charged.hold {
                    self.holds.remove(&hold);
                }
                if let Some(amount) = in_dollars(charged.amount) {
                    self.balance = self.balance.saturating_sub(amount);
                    self.spending.add(at, amount, charged.person);
                }
            }
            _ => {}
        }
    }

    /// How much the payment `reference` names added, when it was added already.
    pub(super) fn added(&self, reference: &str) -> Option<Money> {
        self.references.get(reference).copied()
    }

    /// The limit that holds: its Owner's, else `default`, the server's own.
    fn limit(&self, default: Option<Money>) -> Option<Money> {
        self.limit.or(default)
    }

    /// What is left, and the spending, with this server's charges not yet written counted in.
    fn counted(&self) -> (Nanos, Spending) {
        let mut balance = self.balance;
        let mut spending = self.spending.clone();
        for unwritten in &self.unwritten {
            if let Some(amount) = in_dollars(unwritten.charged.amount) {
                balance = balance.saturating_sub(amount);
                spending.add(unwritten.at, amount, unwritten.charged.person);
            }
        }
        (balance, spending)
    }

    /// What holds count at `now`: those of this server's calls running, and the others' not
    /// yet lapsed, less those a charge here not yet written lets go of.
    fn held(&self, now: Timestamp) -> Nanos {
        let released: BTreeSet<EventId> = self
            .unwritten
            .iter()
            .filter_map(|unwritten| unwritten.charged.hold)
            .collect();
        self.holds
            .iter()
            .filter(|(id, _)| !released.contains(id))
            .filter(|(id, held)| self.running.contains(id) || now.duration_since(held.at) < LAPSE)
            .fold(Nanos::ZERO, |sum, (_, held)| {
                sum.saturating_add(held.amount)
            })
    }

    /// Whether `worst` may be held for a call at `now`, or why not: nothing is left of the
    /// balance or of the month's limit, or `worst` is more than what holds leave free of
    /// either.
    pub(super) fn may_hold(
        &self,
        worst: Nanos,
        now: Timestamp,
        default: Option<Money>,
    ) -> Result<(), Unpaid> {
        let (balance, spending) = self.counted();
        if balance <= Nanos::ZERO {
            return Err(Unpaid::Spent);
        }
        let held = self.held(now);
        let free = balance.saturating_sub(held);
        if worst > free {
            let (free, needs) = (dollars(free.max(Nanos::ZERO)), dollars(worst));
            return Err(Unpaid::Short { free, needs });
        }
        if let Some(limit) = self.limit(default) {
            let spent = if spending.month == month_of(now) {
                spending.all
            } else {
                Nanos::ZERO
            };
            let room = in_dollars(limit)
                .unwrap_or(Nanos::ZERO)
                .saturating_sub(spent);
            if room <= Nanos::ZERO {
                return Err(Unpaid::MonthLimit { limit });
            }
            let free = room.saturating_sub(held);
            if worst > free {
                let (free, needs) = (dollars(free.max(Nanos::ZERO)), dollars(worst));
                return Err(Unpaid::MonthShort { limit, free, needs });
            }
        }
        Ok(())
    }

    /// Lets go of the call's `hold` here, and counts `charged` in its place, at once, to be
    /// written after; written, it lets go of the hold for every server.
    pub(super) fn settle(&mut self, hold: Option<EventId>, charged: Option<Unwritten>) {
        if let Some(hold) = hold {
            self.running.remove(&hold);
        }
        self.unwritten.extend(charged);
    }

    /// The account as it stands at `now`, the server's own limit `default`.
    pub(super) fn account(&self, now: Timestamp, default: Option<Money>) -> Account {
        let (balance, spending) = self.counted();
        let this_month = spending.month == month_of(now);
        let (spent, people) = if this_month {
            let people = spending
                .people
                .iter()
                .map(|(person, spent)| (*person, dollars(*spent)))
                .collect();
            (spending.all, people)
        } else {
            (Nanos::ZERO, BTreeMap::new())
        };
        Account {
            balance: dollars(balance),
            limit: self.limit(default),
            spent: dollars(spent),
            people,
            since: month_start(now),
        }
    }

    /// Whether it may be forgotten: no call of this server's holds anything of it, and it has
    /// nothing to write.
    pub(super) fn idle(&self) -> bool {
        self.running.is_empty() && self.unwritten.is_empty()
    }
}

#[cfg(test)]
mod tests;
