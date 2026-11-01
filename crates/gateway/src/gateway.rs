//! The gateway as a model provider: each call goes by its purpose's route, to the first target
//! that can serve it, within budget, and comes back saying who served it.

use std::sync::Arc;

use glave_event::{Currency, Money};
use glave_ports::{
    Completion, Delta, EventLog, Failure, ModelProvider, ModelRequest, ProviderError, Route,
    Served, Unpaid,
};
use tracing::Instrument as _;

use crate::budget::{BudgetKey, Budgets, Charge, Ledger, OverBudget};
use crate::credits::{Charging, Credits, Paying};
use crate::estimate::Needs;
use crate::filtering::{Filtering, Keeping};
use crate::plan::{candidates, prepare};
use crate::routes::Routes;
use crate::target::Target;
use crate::telemetry;
use glave_event::Filters;

/// The output a budget reserves when neither the request nor the target sets a limit.
const DEFAULT_OUTPUT_TOKENS: u32 = 32_768;

/// Model calls routed by purpose, with fallback, budgets, credits and metering.
#[derive(Debug)]
pub struct Gateway {
    routes: Routes,
    ledger: Ledger,
    /// The organizations' credits each call with a payer is charged to; none where the
    /// server keeps none.
    credits: Option<Arc<dyn Charging>>,
    /// Where each call with a payer finds its person's filters among a router's providers;
    /// none where the host keeps none.
    filters: Option<Arc<dyn Filtering>>,
    /// What the server allows for every call, whoever's it is, which nothing a person chooses
    /// loosens: where providers may be based, and that they keep nothing.
    floor: Option<Filters>,
}

impl Gateway {
    /// A gateway over `routes`, with no budgets.
    pub fn new(routes: Routes) -> Self {
        Self {
            routes,
            ledger: Ledger::default(),
            credits: None,
            filters: None,
            floor: None,
        }
    }

    /// Charges each call that says whose work it is to its organization's `credits`.
    #[must_use]
    pub fn with_credits<L: EventLog + 'static>(mut self, credits: Arc<Credits<L>>) -> Self {
        self.credits = Some(credits);
        self
    }

    /// Sends every call only to providers `floor` allows, a person's filters within it.
    #[must_use]
    pub fn with_floor(mut self, floor: Option<Filters>) -> Self {
        self.floor = floor;
        self
    }

    /// Sends each call that says whose work it is only as its person's `filters` ask.
    #[must_use]
    pub fn with_filters(mut self, filters: Arc<dyn Filtering>) -> Self {
        self.filters = Some(filters);
        self
    }

    /// Keeps calls within `budgets`.
    #[must_use]
    pub fn with_budgets(mut self, budgets: Budgets) -> Self {
        self.ledger = Ledger::new(budgets);
        self
    }

    pub fn routes(&self) -> &Routes {
        &self.routes
    }

    /// What `caller` has spent so far in this process, in millionths of their budget's
    /// currency.
    pub fn spent(&self, caller: BudgetKey) -> u64 {
        self.ledger.spent(caller)
    }

    /// Counts `amount`, in millionths of the budget's currency, as spent by `caller` already:
    /// what a gateway serving many reads back from its log when a caller first returns.
    pub fn add_spent(&self, caller: BudgetKey, amount: u64) {
        self.ledger.add_spent(caller, amount);
    }

    /// The budgets calls count in.
    pub fn budgets(&self) -> Budgets {
        self.ledger.limits()
    }

    /// Serves `request` for `caller`, a signed-in person whose own budget it counts in, on
    /// `targets` when given (tried in order, as a route's) or else on its purpose's route.
    pub async fn complete_for(
        &self,
        caller: BudgetKey,
        targets: Option<&[Target]>,
        request: &ModelRequest,
        on_delta: &mut (dyn FnMut(Delta) + Send),
    ) -> Result<Completion, ProviderError> {
        let targets = targets.unwrap_or_else(|| self.routes.targets(request.purpose));
        self.serve(Some(caller), targets, request, on_delta).await
    }

    async fn serve(
        &self,
        caller: Option<BudgetKey>,
        targets: &[Target],
        request: &ModelRequest,
        on_delta: &mut (dyn FnMut(Delta) + Send),
    ) -> Result<Completion, ProviderError> {
        let needs = Needs::of(request);
        // Filters choose among a router's providers: where none is among the targets, they
        // change nothing, and are not read.
        let routed = targets.iter().any(|target| target.router.is_some());
        let filters = match (&self.filters, request.payer) {
            (Some(filters), Some(payer)) if routed => filters.filters(payer).await?,
            _ => None,
        };
        let keeping = Keeping::within(self.floor.as_ref(), filters);
        let targets = candidates(targets, request, &keeping, &needs)?;
        let mut failed: Option<ProviderError> = None;
        let mut over: Option<OverBudget> = None;
        let mut unpaid: Option<ProviderError> = None;
        for (target, route) in targets {
            let worst = worst(target, request, &needs);
            let hold = match self.ledger.reserve(caller, worst) {
                Ok(hold) => hold,
                Err(refused) => {
                    over.get_or_insert(refused);
                    continue;
                }
            };
            let paying = match self.pay(request, target, worst, &needs).await {
                Ok(paying) => paying,
                Err(error) if error.is_transient() => {
                    failed = Some(error);
                    continue;
                }
                Err(refused) => {
                    unpaid.get_or_insert(refused);
                    continue;
                }
            };
            let sent = (request, route.as_ref());
            let sent = Self::send_to(target, sent, &needs, paying.as_ref(), on_delta).await;
            match sent {
                Ok(completion) => {
                    hold.settle(actual(&completion));
                    if let Some(paying) = paying {
                        let cost = completion.cost.as_ref().map(|cost| cost.amount);
                        paying.settle(cost, &completion.usage.counts()).await;
                    }
                    return Ok(completion);
                }
                Err(error) => {
                    if let Some(paying) = paying {
                        paying.broke().await;
                    }
                    // A failure that may pass moves on to the next target; any other is final.
                    if !error.is_transient() {
                        return Err(error);
                    }
                    failed = Some(error);
                }
            }
        }
        Err(match (failed, unpaid, over) {
            (Some(error), _, _) | (None, Some(error), _) => error,
            (None, None, Some(over)) => over.into(),
            (None, None, None) => ProviderError::Unserved {
                failure: Failure::Rejected,
                reason: format!("the {} route has no targets", request.purpose.id()),
            },
        })
    }
}

impl Gateway {
    /// Sends `request` to `target`, as its route's purpose and the target take it and with what
    /// `route` asks of a router, noting what comes for `paying`, and says who served it; its
    /// cost is its target's price when its provider reports none.
    async fn send_to(
        target: &Target,
        (request, route): (&ModelRequest, Option<&Route>),
        needs: &Needs<'_>,
        paying: Option<&Paying<'_>>,
        on_delta: &mut (dyn FnMut(Delta) + Send),
    ) -> Result<Completion, ProviderError> {
        let (sent, stripped) = prepare(request, route, target, needs);
        let effort = sent.effort.clone();
        let span = telemetry::model_call(request.purpose.id(), &target.name);
        let mut counted = |delta: Delta| {
            if let Some(paying) = paying {
                paying.saw(&delta);
            }
            on_delta(delta);
        };
        let sent = target
            .handle
            .send(&sent, &mut counted)
            .instrument(span.clone());
        let sent = sent.await.map(|mut completion| {
            if completion.cost.is_none() {
                completion.cost = target
                    .price
                    .as_ref()
                    .map(|price| price.cost(&completion.usage.counts()));
            }
            completion.served = Some(Served {
                route: request.purpose.id().to_owned(),
                target: target.name.clone(),
                service: target.service.clone(),
                effort,
                reasoning_stripped: stripped,
            });
            completion
        });
        telemetry::model_ended(&span, request.purpose.id(), &target.name, &sent);
        sent
    }

    /// Holds a call's `worst` case at `target` against the credits of the organization whose
    /// work it is, where this gateway keeps credits and the call says whose it is. A target
    /// with no price in dollars can't be held, so it is refused.
    async fn pay<'a>(
        &'a self,
        request: &ModelRequest,
        target: &'a Target,
        worst: Charge,
        needs: &Needs<'_>,
    ) -> Result<Option<Paying<'a>>, ProviderError> {
        let (Some(credits), Some(payer)) = (self.credits.as_deref(), request.payer) else {
            return Ok(None);
        };
        let Some(price) = target
            .price
            .as_ref()
            .filter(|p| p.currency == Currency::USD)
        else {
            let model = target.model.clone();
            return Err(ProviderError::Unpaid(Unpaid::Unpriced { model }));
        };
        let worst = Money {
            micros: worst
                .0
                .map_or(0, |(micros, _)| i64::try_from(micros).unwrap_or(i64::MAX)),
            currency: Currency::USD,
        };
        let input = needs.fit_tokens();
        Paying::hold(credits, payer, worst, price, input)
            .await
            .map(Some)
    }
}

impl ModelProvider for Gateway {
    async fn complete(
        &self,
        request: &ModelRequest,
        on_delta: &mut (dyn FnMut(Delta) + Send),
    ) -> Result<Completion, ProviderError> {
        let targets = self.routes.for_model(request.purpose, &request.model);
        self.serve(None, &targets, request, on_delta).await
    }
}

/// The most a call to `target` could cost: the input, estimated cautiously, and the longest
/// answer allowed, at the target's price when it has one.
fn worst(target: &Target, request: &ModelRequest, needs: &Needs<'_>) -> Charge {
    let output = request
        .max_output_tokens
        .or(target.can.max_output_tokens)
        .unwrap_or(DEFAULT_OUTPUT_TOKENS);
    let counts = glave_event::TokenCounts {
        input: needs.reserve_tokens(),
        output: u64::from(output),
        ..glave_event::TokenCounts::default()
    };
    Charge(
        target
            .price
            .as_ref()
            .map(|price| (price.micros(&counts), price.currency)),
    )
}

/// What a finished call cost, when known.
fn actual(completion: &Completion) -> Charge {
    Charge(completion.cost.as_ref().map(|cost| {
        (
            u64::try_from(cost.amount.micros).unwrap_or(0),
            cost.amount.currency,
        )
    }))
}
