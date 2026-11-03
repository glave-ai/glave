//! Budgets, reserve then settle: before a signed-in caller's call is sent its worst case is
//! held against their budget, and when it ends the hold becomes what it really cost. A call
//! that could exceed it is refused before it is sent, so calls side by side never overshoot.
//! One kind is kept, the one a deployment sets: each caller's money in each period, a UTC day
//! (`[budget] person_per_day_usd`), by a gateway serving many.

use std::collections::BTreeMap;
use std::sync::{Mutex, PoisonError};

use glave_event::{Currency, Money, Nanos, PrincipalId};
use glave_ports::ProviderError;
use thiserror::Error;

/// Callers' accounts kept at once. Past this, an account with nothing in flight from the
/// earliest period is forgotten; its spending stays in the log, from which it is read again.
const MOST_CALLERS: usize = 100_000;

/// The budgets calls count in.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Budgets {
    /// Each signed-in caller's, in each period (see [`BudgetKey`]). Only calls with a known
    /// cost count: a configured price before sending, the provider's reported cost or that
    /// price after.
    pub person: Option<Money>,
}

/// The key of a signed-in caller's budget: the person and one period of their budget.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct BudgetKey {
    pub person: PrincipalId,
    /// The period the person's budget runs for, numbered by whoever calls (a day since 1970,
    /// say): each new period starts from nothing.
    pub period: u32,
}

/// A call refused because it could exceed `caller`'s budget. Amounts are in billionths of
/// the limit's currency.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("the person {} budget would be exceeded: {}", self.caller.person, self.detail())]
pub struct OverBudget {
    pub caller: BudgetKey,
    pub limit: Money,
    /// Spent, plus held for calls in flight.
    pub committed: Nanos,
    /// The most this call could cost.
    pub needed: Nanos,
}

impl OverBudget {
    fn detail(&self) -> String {
        format!(
            "{} of {} billionths of {} committed, and this call could need {} more",
            self.committed.get(),
            allowed(self.limit).get(),
            self.limit.currency,
            self.needed.get()
        )
    }
}

impl From<OverBudget> for ProviderError {
    fn from(over: OverBudget) -> Self {
        Self::OverBudget {
            budget: format!("person {}", over.caller.person),
            detail: over.detail(),
        }
    }
}

/// What a call costs, or could: in billionths of a currency, when known.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct Charge(pub(crate) Option<(Nanos, Currency)>);

impl Charge {
    /// The charge in `limit`'s currency; in another, or unknown, it counts as none.
    fn against(self, limit: Money) -> Nanos {
        match self.0 {
            Some((nanos, currency)) if currency == limit.currency => nanos.max(Nanos::ZERO),
            _ => Nanos::ZERO,
        }
    }
}

fn allowed(money: Money) -> Nanos {
    money.nanos.max(Nanos::ZERO)
}

#[derive(Debug, Clone, Copy, Default)]
struct Account {
    spent: Nanos,
    held: Nanos,
}

/// Every caller's account.
#[derive(Debug, Default)]
pub(crate) struct Ledger {
    limits: Budgets,
    accounts: Mutex<BTreeMap<BudgetKey, Account>>,
}

impl Ledger {
    pub(crate) fn new(limits: Budgets) -> Self {
        Self {
            limits,
            accounts: Mutex::default(),
        }
    }

    pub(crate) fn limits(&self) -> Budgets {
        self.limits
    }

    /// Holds `worst` in `caller`'s budget, or refuses it, holding nothing. A call for no one
    /// signed in counts in none.
    pub(crate) fn reserve(
        &self,
        caller: Option<BudgetKey>,
        worst: Charge,
    ) -> Result<Hold<'_>, OverBudget> {
        let (Some(caller), Some(limit)) = (caller, self.limits.person) else {
            return Ok(Hold::empty(self));
        };
        let needed = worst.against(limit);
        let mut accounts = self.accounts.lock().unwrap_or_else(PoisonError::into_inner);
        let account = accounts.get(&caller).copied().unwrap_or_default();
        let committed = account.spent.saturating_add(account.held);
        // Past the most there can be is over any limit, the highest too.
        let within = committed
            .checked_add(needed)
            .is_some_and(|total| total <= allowed(limit));
        if !within {
            return Err(OverBudget {
                caller,
                limit,
                committed,
                needed,
            });
        }
        let account = entry(&mut accounts, caller);
        account.held = account.held.saturating_add(needed);
        Ok(Hold {
            ledger: self,
            held: Some((caller, limit, needed)),
        })
    }

    /// What `caller` has spent, in billionths of the limit's currency: settled calls only.
    pub(crate) fn spent(&self, caller: BudgetKey) -> Nanos {
        let accounts = self.accounts.lock().unwrap_or_else(PoisonError::into_inner);
        accounts
            .get(&caller)
            .map_or(Nanos::ZERO, |account| account.spent)
    }

    /// Counts `amount`, in billionths of the limit's currency, as already spent by `caller`:
    /// spending read back from the log after a restart.
    pub(crate) fn add_spent(&self, caller: BudgetKey, amount: Nanos) {
        let mut accounts = self.accounts.lock().unwrap_or_else(PoisonError::into_inner);
        let account = entry(&mut accounts, caller);
        account.spent = account.spent.saturating_add(amount);
    }

    fn release(&self, caller: BudgetKey, held: Nanos, spent: Nanos) {
        let mut accounts = self.accounts.lock().unwrap_or_else(PoisonError::into_inner);
        let account = entry(&mut accounts, caller);
        account.held = account.held.saturating_sub(held);
        account.spent = account.spent.saturating_add(spent);
    }
}

/// `caller`'s account, made when it has none; past [`MOST_CALLERS`], one with nothing in
/// flight from the earliest period is forgotten first.
fn entry(accounts: &mut BTreeMap<BudgetKey, Account>, caller: BudgetKey) -> &mut Account {
    if accounts.len() >= MOST_CALLERS && !accounts.contains_key(&caller) {
        let idle = accounts
            .iter()
            .filter(|(_, account)| account.held == Nanos::ZERO)
            .min_by_key(|(caller, _)| caller.period)
            .map(|(caller, _)| *caller);
        if let Some(idle) = idle {
            accounts.remove(&idle);
        }
    }
    accounts.entry(caller).or_default()
}

/// A call's worst case, held until it settles. Dropped unsettled (the call failed or was
/// cancelled), it is released and nothing is spent.
#[derive(Debug)]
pub(crate) struct Hold<'a> {
    ledger: &'a Ledger,
    /// Whose budget, its limit, and what is held in it; none when the call counts in none.
    held: Option<(BudgetKey, Money, Nanos)>,
}

impl<'a> Hold<'a> {
    fn empty(ledger: &'a Ledger) -> Self {
        Self { ledger, held: None }
    }

    /// Turns the hold into what the call really cost.
    pub(crate) fn settle(mut self, actual: Charge) {
        if let Some((caller, limit, held)) = self.held.take() {
            self.ledger.release(caller, held, actual.against(limit));
        }
    }
}

impl Drop for Hold<'_> {
    fn drop(&mut self) {
        if let Some((caller, _, held)) = self.held.take() {
            self.ledger.release(caller, held, Nanos::ZERO);
        }
    }
}

#[cfg(test)]
mod tests {
    use glave_event::PrincipalId;

    use super::*;

    fn caller() -> BudgetKey {
        let at = jiff::Timestamp::from_second(1).unwrap();
        BudgetKey {
            person: PrincipalId::new(at, [1; 10]).unwrap(),
            period: 1,
        }
    }

    fn charge(nanos: i64) -> Charge {
        Charge(Some((Nanos::new(nanos), Currency::USD)))
    }

    fn ledger(limit: Nanos) -> Ledger {
        Ledger::new(Budgets {
            person: Some(Money {
                nanos: limit,
                currency: Currency::USD,
            }),
        })
    }

    #[test]
    fn a_hold_is_the_worst_case_and_settles_to_the_actual() {
        let ledger = ledger(Nanos::new(10));
        let hold = ledger.reserve(Some(caller()), charge(10)).unwrap();
        assert!(
            ledger.reserve(Some(caller()), charge(1)).is_err(),
            "all held"
        );
        hold.settle(charge(3));
        assert_eq!(ledger.spent(caller()), Nanos::new(3));
        assert!(
            ledger.reserve(Some(caller()), charge(8)).is_err(),
            "7 are left"
        );
        drop(ledger.reserve(Some(caller()), charge(7)).unwrap());
        assert_eq!(
            ledger.spent(caller()),
            Nanos::new(3),
            "unsettled, nothing is spent"
        );
    }

    #[test]
    fn at_the_limit_of_what_can_be_counted_a_hold_is_refused_not_wrapped() {
        let ledger = ledger(Nanos::MAX);
        let all = ledger.reserve(Some(caller()), Charge(Some((Nanos::MAX, Currency::USD))));
        let all = all.unwrap();
        let more = ledger.reserve(Some(caller()), charge(1)).unwrap_err();
        assert_eq!(more.committed, Nanos::MAX);
        assert_eq!(more.needed, Nanos::new(1));
        all.settle(Charge(Some((Nanos::MAX, Currency::USD))));
        ledger.add_spent(caller(), Nanos::new(1));
        assert_eq!(
            ledger.spent(caller()),
            Nanos::MAX,
            "stops at the most there can be"
        );
        assert!(ledger.reserve(Some(caller()), charge(1)).is_err());
    }

    #[test]
    fn a_charge_below_nothing_counts_as_none() {
        let ledger = ledger(Nanos::new(1));
        let hold = ledger.reserve(Some(caller()), charge(-5)).unwrap();
        hold.settle(charge(-5));
        assert_eq!(ledger.spent(caller()), Nanos::ZERO);
    }
}
