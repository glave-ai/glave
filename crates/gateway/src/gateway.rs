//! The gateway as a model provider: each call goes by its purpose's route, to the first target
//! that can serve it, within budget, and comes back saying who served it.

use std::future::Future;

use glave_ports::{Completion, Delta, Failure, ModelProvider, ModelRequest, ProviderError, Served};

use crate::budget::{Budget, Budgets, Charge, Ledger, OverBudget};
use crate::estimate::Needs;
use crate::plan::{candidates, prepare};
use crate::routes::Routes;
use crate::target::Target;

/// The output a budget reserves when neither the request nor the target sets a limit.
const DEFAULT_OUTPUT_TOKENS: u32 = 32_768;

/// Model calls routed by purpose, with fallback, budgets and metering.
#[derive(Debug)]
pub struct Gateway {
    routes: Routes,
    ledger: Ledger,
}

impl Gateway {
    /// A gateway over `routes`, with no budgets.
    pub fn new(routes: Routes) -> Self {
        Self {
            routes,
            ledger: Ledger::default(),
        }
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

    /// What `budget` has spent so far in this process, in its limit's unit.
    pub fn spent(&self, budget: Budget) -> u64 {
        self.ledger.spent(budget)
    }

    async fn serve(
        &self,
        request: &ModelRequest,
        on_delta: &mut (dyn FnMut(Delta) + Send),
    ) -> Result<Completion, ProviderError> {
        let needs = Needs::of(request);
        let targets = candidates(self.routes.targets(request.purpose), request, &needs)?;
        let mut failed: Option<ProviderError> = None;
        let mut over: Option<OverBudget> = None;
        for target in targets {
            let hold = match self
                .ledger
                .reserve(&request.threads, worst(target, request, &needs))
            {
                Ok(hold) => hold,
                Err(refused) => {
                    over.get_or_insert(refused);
                    continue;
                }
            };
            let (sent, stripped) = prepare(request, target, &needs);
            match target.handle.send(&sent, on_delta).await {
                Ok(mut completion) => {
                    if completion.cost.is_none() {
                        completion.cost = target
                            .price
                            .as_ref()
                            .map(|price| price.cost(&completion.usage.counts()));
                    }
                    hold.settle(actual(&completion));
                    completion.served = Some(Served {
                        route: request.purpose.id().to_owned(),
                        target: target.name.clone(),
                        reasoning_stripped: stripped,
                    });
                    return Ok(completion);
                }
                // A failure that may pass moves on to the next target; any other is final.
                Err(error) if error.is_transient() => failed = Some(error),
                Err(error) => return Err(error),
            }
        }
        Err(match (failed, over) {
            (Some(error), _) => error,
            (None, Some(over)) => over.into(),
            (None, None) => ProviderError::Unserved {
                failure: Failure::Rejected,
                reason: format!("the {} route has no targets", request.purpose.id()),
            },
        })
    }
}

impl ModelProvider for Gateway {
    fn complete(
        &self,
        request: &ModelRequest,
        on_delta: &mut (dyn FnMut(Delta) + Send),
    ) -> impl Future<Output = Result<Completion, ProviderError>> + Send {
        self.serve(request, on_delta)
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
    Charge {
        tokens: counts.input.saturating_add(counts.output),
        money: target
            .price
            .as_ref()
            .map(|price| (price.micros(&counts), price.currency)),
    }
}

/// What a finished call cost: its reported tokens, and its cost when known.
fn actual(completion: &Completion) -> Charge {
    let counts = completion.usage.counts();
    Charge {
        tokens: counts.input.saturating_add(counts.output),
        money: completion.cost.as_ref().map(|cost| {
            (
                u64::try_from(cost.amount.micros).unwrap_or(0),
                cost.amount.currency,
            )
        }),
    }
}
