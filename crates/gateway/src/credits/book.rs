//! What one organization's credits ledger says, as far as it was read; the charges this server
//! made that the store does not have yet, counted already; and what calls running hold against
//! it now. Amounts are millionths of a US dollar; anything in another currency is no part of
//! it.

use std::collections::BTreeMap;

use glave_event::{At, Body, CreditsCharged, Currency, Money, PrincipalId, RawEvent};
use glave_ports::Unpaid;
use glave_ports::stream::in_order;
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
pub(super) type Month = i32;

/// The month `at` falls in.
pub(super) fn month_of(at: Timestamp) -> Month {
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

/// `micros` millionths of a dollar.
pub(super) fn dollars(micros: i64) -> Money {
    Money {
        micros,
        currency: Currency::USD,
    }
}

/// `money` in millionths of a dollar, when it is in dollars.
pub(super) fn micros(money: Money) -> Option<i64> {
    (money.currency == Currency::USD).then_some(money.micros)
}

/// One month's spending: everyone's, and each person's part.
#[derive(Debug, Default, Clone)]
struct Spending {
    month: Month,
    all: i64,
    people: BTreeMap<PrincipalId, i64>,
}

impl Spending {
    /// Counts `amount` spent `at` for `person`'s work, a new month starting from nothing.
    fn add(&mut self, at: At, amount: i64, person: Option<PrincipalId>) {
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

/// A charge this server made, counted, and not yet in the store.
#[derive(Debug, Clone)]
pub(super) struct Unwritten {
    pub(super) charged: CreditsCharged,
    pub(super) at: At,
}

/// One organization's book.
#[derive(Debug, Default)]
pub(super) struct Book {
    /// The `seq` the next event will take.
    next: u64,
    /// What the ledger says is left.
    balance: i64,
    /// The latest limit its Owner set, if any.
    limit: Option<Money>,
    /// The payments added, by reference, with how much each added.
    references: BTreeMap<String, Money>,
    /// What the ledger says was spent in its latest month.
    spending: Spending,
    /// This server's charges the store does not have yet, oldest first.
    pub(super) unwritten: Vec<Unwritten>,
    /// How many of the first of `unwritten` are being written now.
    pub(super) writing: usize,
    /// Held for calls running.
    pub(super) held: i64,
    /// When it was last used, by the books' own count: the least recent goes first.
    pub(super) used: u64,
}

impl Book {
    pub(super) fn next_seq(&self) -> u64 {
        self.next.max(1)
    }

    /// Takes in the event at the next `seq`; any other is already in, or not yet.
    pub(super) fn apply(&mut self, raw: RawEvent) {
        let Some((at, body)) = in_order(&mut self.next, raw) else {
            return;
        };
        match body {
            Body::CreditsAdded(added) => {
                if let Some(amount) = micros(added.amount) {
                    self.balance = self.balance.saturating_add(amount);
                }
                self.references.insert(added.reference, added.amount);
            }
            Body::CreditsLimitSet(set) => self.limit = Some(set.limit),
            Body::CreditsCharged(charged) => {
                if let Some(amount) = micros(charged.amount) {
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
    fn counted(&self) -> (i64, Spending) {
        let mut balance = self.balance;
        let mut spending = self.spending.clone();
        for unwritten in &self.unwritten {
            if let Some(amount) = micros(unwritten.charged.amount) {
                balance = balance.saturating_sub(amount);
                spending.add(unwritten.at, amount, unwritten.charged.person);
            }
        }
        (balance, spending)
    }

    /// Holds `worst` for a call in `month`, or refuses it: when nothing is left of the balance
    /// or the month's limit once what calls running hold is taken off, or `worst` is more than
    /// what is left.
    pub(super) fn hold(
        &mut self,
        worst: i64,
        month: Month,
        default: Option<Money>,
    ) -> Result<(), Unpaid> {
        let (balance, spending) = self.counted();
        let left = balance.saturating_sub(self.held);
        if left <= 0 {
            return Err(Unpaid::Spent);
        }
        if worst > left {
            let (left, needs) = (dollars(left), dollars(worst));
            return Err(Unpaid::Short { left, needs });
        }
        if let Some(limit) = self.limit(default) {
            let spent = if spending.month == month {
                spending.all
            } else {
                0
            };
            let room = micros(limit)
                .unwrap_or(0)
                .saturating_sub(spent)
                .saturating_sub(self.held);
            if room <= 0 {
                return Err(Unpaid::MonthLimit { limit });
            }
            if worst > room {
                let (left, needs) = (dollars(room), dollars(worst));
                return Err(Unpaid::MonthShort { limit, left, needs });
            }
        }
        self.held = self.held.saturating_add(worst);
        Ok(())
    }

    /// Lets go of `held`, and counts `charged` in its place, at once, to be written after.
    pub(super) fn settle(&mut self, held: i64, charged: Option<Unwritten>) {
        self.held = self.held.saturating_sub(held);
        self.unwritten.extend(charged);
    }

    /// Takes in `written`, the events the first `count` of `unwritten` became once they were
    /// appended, in their place.
    pub(super) fn written(&mut self, count: usize, written: Vec<RawEvent>) {
        let count = count.min(self.unwritten.len());
        self.unwritten.drain(..count);
        for raw in written {
            self.apply(raw);
        }
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
            (0, BTreeMap::new())
        };
        Account {
            balance: dollars(balance),
            limit: self.limit(default),
            spent: dollars(spent),
            people,
            since: month_start(now),
        }
    }

    /// Whether it may be forgotten: no call holds anything of it, and it has nothing to write.
    pub(super) fn idle(&self) -> bool {
        self.held == 0 && self.unwritten.is_empty()
    }
}

#[cfg(test)]
mod tests;
