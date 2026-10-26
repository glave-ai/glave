//! Budgets, reserve then settle: before a signed-in caller's call is sent its worst case is
//! held against their budget, and when it ends the hold becomes what it really cost. A call
//! that could exceed it is refused before it is sent, so calls side by side never overshoot.
//! One kind is kept, the one a deployment sets: each caller's money in each period, a UTC day
//! (`[budget] person_per_day_usd`), by a gateway serving many.

use std::collections::BTreeMap;
use std::sync::{Mutex, PoisonError};

use glave_event::{Currency, Money, PrincipalId};
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

/// A call refused because it could exceed `caller`'s budget. Amounts are in millionths of
/// the limit's currency.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("the person {} budget would be exceeded: {}", self.caller.person, self.detail())]
pub struct OverBudget {
    pub caller: BudgetKey,
    pub limit: Money,
    /// Spent, plus held for calls in flight.
    pub committed: u64,
    /// The most this call could cost.
    pub needed: u64,
}

impl OverBudget {
    fn detail(&self) -> String {
        format!(
            "{} of {} millionths of {} committed, and this call could need {} more",
            self.committed,
            allowed(self.limit),
            self.limit.currency,
            self.needed
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

/// What a call costs, or could: in millionths of a currency, when known.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct Charge(pub(crate) Option<(u64, Currency)>);

impl Charge {
    /// The charge in `limit`'s currency; in another, or unknown, it counts as none.
    fn against(self, limit: Money) -> u64 {
        match self.0 {
            Some((micros, currency)) if currency == limit.currency => micros,
            _ => 0,
        }
    }
}

fn allowed(money: Money) -> u64 {
    u64::try_from(money.micros).unwrap_or(0)
}

#[derive(Debug, Clone, Copy, Default)]
struct Account {
    spent: u64,
    held: u64,
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
        if committed.saturating_add(needed) > allowed(limit) {
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

    /// What `caller` has spent, in millionths of the limit's currency: settled calls only.
    pub(crate) fn spent(&self, caller: BudgetKey) -> u64 {
        let accounts = self.accounts.lock().unwrap_or_else(PoisonError::into_inner);
        accounts.get(&caller).map_or(0, |account| account.spent)
    }

    /// Counts `amount`, in millionths of the limit's currency, as already spent by `caller`:
    /// spending read back from the log after a restart.
    pub(crate) fn add_spent(&self, caller: BudgetKey, amount: u64) {
        let mut accounts = self.accounts.lock().unwrap_or_else(PoisonError::into_inner);
        let account = entry(&mut accounts, caller);
        account.spent = account.spent.saturating_add(amount);
    }

    fn release(&self, caller: BudgetKey, held: u64, spent: u64) {
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
            .filter(|(_, account)| account.held == 0)
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
    held: Option<(BudgetKey, Money, u64)>,
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
            self.ledger.release(caller, held, 0);
        }
    }
}
