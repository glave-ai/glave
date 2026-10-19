//! Budgets, reserve then settle: before a call is sent its worst case is held against every
//! budget it counts in, and when it ends the hold becomes what it really cost. A call that
//! could exceed a budget is refused before it is sent, so calls side by side never overshoot.

use std::collections::BTreeMap;
use std::fmt;
use std::sync::{Mutex, PoisonError};

use glave_event::{Currency, Money, PrincipalId, ThreadId};
use glave_ports::ProviderError;
use thiserror::Error;

/// Thread accounts kept at once. Past this, the oldest thread with nothing in flight is
/// forgotten; its spending stays in the log.
const MOST_THREADS: usize = 10_000;

/// Callers' accounts kept at once. Past this, an account with nothing in flight from the
/// earliest period is forgotten; its spending stays in the log, from which it is read again.
const MOST_CALLERS: usize = 100_000;

/// How much a budget allows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Limit {
    /// Tokens in and out.
    Tokens(u64),
    /// Money. Only calls with a known cost count: a configured price before sending, the
    /// provider's reported cost or that price after.
    Money(Money),
}

/// The budgets every call counts in. A call counts in its thread's budget and in the budgets
/// of every thread above it, so a subagent spends from its parent's budget too.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Budgets {
    pub organization: Option<Limit>,
    /// The person's: the one person of a gateway in glave, or each signed-in caller of a
    /// gateway serving many, in each period (see [`BudgetKey`]).
    pub person: Option<Limit>,
    /// Each thread's, its subagents' calls included.
    pub thread: Option<Limit>,
}

/// The key of a signed-in caller's budget on a gateway serving many people: the person and
/// one period of their budget.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct BudgetKey {
    pub person: PrincipalId,
    /// The period the person's budget runs for, numbered by whoever calls (a day since 1970,
    /// say): each new period starts from nothing.
    pub period: u32,
}

/// One budget.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Budget {
    Organization,
    /// The person of a gateway in glave.
    Person,
    /// A signed-in caller's person budget, in one period.
    Caller(BudgetKey),
    Thread(ThreadId),
}

impl fmt::Display for Budget {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Organization => f.write_str("organization"),
            Self::Person => f.write_str("person"),
            Self::Caller(caller) => write!(f, "person {}", caller.person),
            Self::Thread(id) => write!(f, "thread {id}"),
        }
    }
}

/// A call refused because it could exceed `budget`. Amounts are in the limit's unit: tokens,
/// or millionths of its currency.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("the {budget} budget would be exceeded: {}", self.detail())]
pub struct OverBudget {
    pub budget: Budget,
    pub limit: Limit,
    /// Spent, plus held for calls in flight.
    pub committed: u64,
    /// The most this call could cost.
    pub needed: u64,
}

impl OverBudget {
    fn detail(&self) -> String {
        let (limit, unit) = match self.limit {
            Limit::Tokens(tokens) => (tokens, "tokens".to_owned()),
            Limit::Money(money) => (allowed(money), format!("millionths of {}", money.currency)),
        };
        format!(
            "{} of {limit} {unit} committed, and this call could need {} more",
            self.committed, self.needed
        )
    }
}

impl From<OverBudget> for ProviderError {
    fn from(over: OverBudget) -> Self {
        Self::OverBudget {
            budget: over.budget.to_string(),
            detail: over.detail(),
        }
    }
}

/// What a call costs, or could: tokens, and money when known.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct Charge {
    pub(crate) tokens: u64,
    pub(crate) money: Option<(u64, Currency)>,
}

impl Charge {
    /// The charge in `limit`'s unit; money in another currency, or unknown, counts as none.
    fn against(self, limit: Limit) -> u64 {
        match (limit, self.money) {
            (Limit::Tokens(_), _) => self.tokens,
            (Limit::Money(cap), Some((micros, currency))) if currency == cap.currency => micros,
            (Limit::Money(_), _) => 0,
        }
    }
}

fn allowed(money: Money) -> u64 {
    u64::try_from(money.micros).unwrap_or(0)
}

fn cap(limit: Limit) -> u64 {
    match limit {
        Limit::Tokens(tokens) => tokens,
        Limit::Money(money) => allowed(money),
    }
}

#[derive(Debug, Clone, Copy, Default)]
struct Account {
    spent: u64,
    held: u64,
}

#[derive(Debug, Default)]
struct Accounts {
    organization: Account,
    person: Account,
    callers: BTreeMap<BudgetKey, Account>,
    threads: BTreeMap<ThreadId, Account>,
}

/// Every budget's account.
#[derive(Debug, Default)]
pub(crate) struct Ledger {
    limits: Budgets,
    accounts: Mutex<Accounts>,
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

    fn none(&self) -> bool {
        self.limits == Budgets::default()
    }

    /// The budgets a call by `caller` for `threads` counts in, with their limits.
    fn applying(&self, caller: Option<BudgetKey>, threads: &[ThreadId]) -> Vec<(Budget, Limit)> {
        let mut budgets = Vec::new();
        if let Some(limit) = self.limits.organization {
            budgets.push((Budget::Organization, limit));
        }
        if let Some(limit) = self.limits.person {
            let person = caller.map_or(Budget::Person, Budget::Caller);
            budgets.push((person, limit));
        }
        if let Some(limit) = self.limits.thread {
            for (n, id) in threads.iter().enumerate() {
                if !threads.iter().take(n).any(|seen| seen == id) {
                    budgets.push((Budget::Thread(*id), limit));
                }
            }
        }
        budgets
    }

    /// Holds `worst` in every budget the call counts in, or refuses it, holding nothing.
    pub(crate) fn reserve(
        &self,
        caller: Option<BudgetKey>,
        threads: &[ThreadId],
        worst: Charge,
    ) -> Result<Hold<'_>, OverBudget> {
        if self.none() {
            return Ok(Hold::empty(self));
        }
        let budgets = self.applying(caller, threads);
        let mut accounts = self.accounts.lock().unwrap_or_else(PoisonError::into_inner);
        for (budget, limit) in &budgets {
            let account = accounts.get(*budget);
            let committed = account.spent.saturating_add(account.held);
            let needed = worst.against(*limit);
            if committed.saturating_add(needed) > cap(*limit) {
                return Err(OverBudget {
                    budget: *budget,
                    limit: *limit,
                    committed,
                    needed,
                });
            }
        }
        for (budget, limit) in &budgets {
            let account = accounts.entry(*budget);
            account.held = account.held.saturating_add(worst.against(*limit));
        }
        Ok(Hold {
            ledger: self,
            budgets,
            worst,
        })
    }

    /// What `budget` has spent, in its limit's unit: settled calls only.
    pub(crate) fn spent(&self, budget: Budget) -> u64 {
        let accounts = self.accounts.lock().unwrap_or_else(PoisonError::into_inner);
        accounts.get(budget).spent
    }

    /// Counts `amount`, in the budget's limit's unit, as already spent in `budget`: spending
    /// read back from the log after a restart.
    pub(crate) fn add_spent(&self, budget: Budget, amount: u64) {
        let mut accounts = self.accounts.lock().unwrap_or_else(PoisonError::into_inner);
        let account = accounts.entry(budget);
        account.spent = account.spent.saturating_add(amount);
    }

    fn release(&self, budgets: &[(Budget, Limit)], worst: Charge, actual: Charge) {
        if budgets.is_empty() {
            return;
        }
        let mut accounts = self.accounts.lock().unwrap_or_else(PoisonError::into_inner);
        for (budget, limit) in budgets {
            let account = accounts.entry(*budget);
            account.held = account.held.saturating_sub(worst.against(*limit));
            account.spent = account.spent.saturating_add(actual.against(*limit));
        }
    }
}

impl Accounts {
    fn get(&self, budget: Budget) -> Account {
        match budget {
            Budget::Organization => self.organization,
            Budget::Person => self.person,
            Budget::Caller(caller) => self.callers.get(&caller).copied().unwrap_or_default(),
            Budget::Thread(id) => self.threads.get(&id).copied().unwrap_or_default(),
        }
    }

    fn entry(&mut self, budget: Budget) -> &mut Account {
        match budget {
            Budget::Organization => &mut self.organization,
            Budget::Person => &mut self.person,
            Budget::Caller(caller) => {
                if self.callers.len() >= MOST_CALLERS && !self.callers.contains_key(&caller) {
                    let idle = self
                        .callers
                        .iter()
                        .filter(|(_, account)| account.held == 0)
                        .min_by_key(|(caller, _)| caller.period)
                        .map(|(caller, _)| *caller);
                    if let Some(idle) = idle {
                        self.callers.remove(&idle);
                    }
                }
                self.callers.entry(caller).or_default()
            }
            Budget::Thread(id) => {
                if self.threads.len() >= MOST_THREADS && !self.threads.contains_key(&id) {
                    // Thread ids are time-ordered, so the first idle one is the oldest.
                    let idle = self
                        .threads
                        .iter()
                        .find(|(_, account)| account.held == 0)
                        .map(|(id, _)| *id);
                    if let Some(idle) = idle {
                        self.threads.remove(&idle);
                    }
                }
                self.threads.entry(id).or_default()
            }
        }
    }
}

/// A call's worst case, held until it settles. Dropped unsettled (the call failed or was
/// cancelled), it is released and nothing is spent.
#[derive(Debug)]
pub(crate) struct Hold<'a> {
    ledger: &'a Ledger,
    budgets: Vec<(Budget, Limit)>,
    worst: Charge,
}

impl<'a> Hold<'a> {
    fn empty(ledger: &'a Ledger) -> Self {
        Self {
            ledger,
            budgets: Vec::new(),
            worst: Charge::default(),
        }
    }

    /// Turns the hold into what the call really cost.
    pub(crate) fn settle(mut self, actual: Charge) {
        let budgets = std::mem::take(&mut self.budgets);
        self.ledger.release(&budgets, self.worst, actual);
    }
}

impl Drop for Hold<'_> {
    fn drop(&mut self) {
        let budgets = std::mem::take(&mut self.budgets);
        self.ledger.release(&budgets, self.worst, Charge::default());
    }
}
