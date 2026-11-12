//! The gateway as a model provider: each call goes by its purpose's route, to the first target
//! that can serve it and its credits can pay for, and comes back saying who served it.

use std::sync::Arc;

use glave_event::{CostKind, Currency, Money};
use glave_ports::{
    Completion, Delta, EventLog, Failure, Made, MediaAsk, ModelProvider, ModelRequest,
    ProviderError, Purpose, Route, Served, Unpaid,
};
use tracing::Instrument as _;

use crate::credits::{Charging, Credits, Paying};
use crate::estimate::Needs;
use crate::filtering::{Filtering, Keeping};
use crate::media::{self, IMAGE_IN_TOKENS};
use crate::plan::{candidates, prepare};
use crate::price::Price;
use crate::routes::{RouteError, Routes};
use crate::target::Target;
use crate::telemetry;
use glave_event::Filters;

/// The output a call's worst case counts when neither the request nor the target sets a limit.
const DEFAULT_OUTPUT_TOKENS: u32 = 32_768;

/// What a call asks of the target that serves it: an answer streamed as it comes, or what a
/// media tool's call makes.
enum Call<'a> {
    Answer(&'a mut (dyn FnMut(Delta) + Send)),
    Make(&'a MediaAsk),
}

/// Model calls routed by purpose, with fallback, credits and metering.
#[derive(Debug)]
pub struct Gateway {
    routes: Routes,
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
    /// A gateway over `routes`, charging no credits.
    pub fn new(routes: Routes) -> Self {
        Self {
            routes,
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

    pub fn routes(&self) -> &Routes {
        &self.routes
    }

    /// Gives `purpose` its own targets, as [`Routes::with_route`] does: a media tool's, which
    /// `main`'s never serve.
    pub fn with_route(
        mut self,
        purpose: Purpose,
        targets: Vec<Target>,
    ) -> Result<Self, RouteError> {
        self.routes = self.routes.with_route(purpose, targets)?;
        Ok(self)
    }

    /// Serves `request` on `targets`, tried in order, as `call` asks.
    async fn serve(
        &self,
        targets: &[Target],
        request: &ModelRequest,
        mut call: Call<'_>,
    ) -> Result<Made, ProviderError> {
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
        let mut unpaid: Option<ProviderError> = None;
        for (target, route) in targets {
            let paying = match self.pay(request, target, &needs).await {
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
            let sent = Self::send_to(target, sent, &needs, paying.as_ref(), &mut call).await;
            match sent {
                Ok(made) => {
                    if let Some(paying) = paying {
                        let completion = &made.completion;
                        let cost = completion.cost.as_ref().map(|cost| cost.amount);
                        paying.settle(cost, &completion.usage.counts()).await;
                    }
                    return Ok(made);
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
        Err(match (failed, unpaid) {
            (Some(error), _) | (None, Some(error)) => error,
            (None, None) => ProviderError::Unserved {
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
        call: &mut Call<'_>,
    ) -> Result<Made, ProviderError> {
        let (sent, stripped) = prepare(request, route, target, needs);
        let effort = sent.effort.clone();
        let span = telemetry::model_call(request.purpose.id(), &target.name);
        let sent = match call {
            Call::Answer(on_delta) => {
                let mut counted = |delta: Delta| {
                    if let Some(paying) = paying {
                        paying.saw(&delta);
                    }
                    on_delta(delta);
                };
                let answered = target.handle.send(&sent, &mut counted);
                let answered = answered.instrument(span.clone()).await;
                answered.map(|completion| Made {
                    files: Vec::new(),
                    completion,
                })
            }
            Call::Make(ask) => {
                let ask = media::sent_to(ask, target, request.purpose);
                let made = target.handle.make(&sent, &ask);
                made.instrument(span.clone()).await
            }
        };
        let sent = sent.map(
            |Made {
                 files,
                 mut completion,
             }| {
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
                Made { files, completion }
            },
        );
        let ended = sent.as_ref().map(|made| &made.completion);
        telemetry::model_ended(&span, request.purpose.id(), &target.name, ended);
        sent
    }

    /// Holds a call's worst case at `target` against the credits of the organization whose
    /// work it is, where this gateway keeps credits and the call says whose it is. A target
    /// with no price in dollars can't be held, so it is refused.
    async fn pay<'a>(
        &'a self,
        request: &ModelRequest,
        target: &'a Target,
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
        let worst = worst(target, request, needs, price);
        let input = needs.fit_tokens();
        let kind = if request.purpose.is_media() {
            CostKind::Media
        } else {
            CostKind::Model
        };
        Paying::hold(credits, payer, worst, (price, input), kind)
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
        let made = self.serve(&targets, request, Call::Answer(on_delta)).await;
        made.map(|made| made.completion)
    }

    /// A media tool's call, by its purpose's own route, to its tier's targets or the first
    /// that can honour it, held and charged as `media`.
    async fn make(&self, request: &ModelRequest, ask: &MediaAsk) -> Result<Made, ProviderError> {
        let route = self.routes.targets(request.purpose);
        let targets = media::chosen(route, request.purpose, ask)?;
        self.serve(&targets, request, Call::Make(ask)).await
    }

    fn serves(&self, purpose: Purpose) -> bool {
        self.routes.serves(purpose)
    }
}

/// The most a call to `target` could cost at its `price`, in dollars: the input, estimated
/// cautiously, each image a media call sends in among it, and the longest answer allowed, a
/// media tier's own most.
fn worst(target: &Target, request: &ModelRequest, needs: &Needs<'_>, price: &Price) -> Money {
    let output = (target.tier.as_ref().map(|tier| tier.output_tokens))
        .or(request.max_output_tokens)
        .or(target.can.max_output_tokens)
        .unwrap_or(DEFAULT_OUTPUT_TOKENS);
    let images = if request.purpose.is_media() {
        u64::try_from(request.files.len()).map_or(u64::MAX, |n| n.saturating_mul(IMAGE_IN_TOKENS))
    } else {
        0
    };
    let counts = glave_event::TokenCounts {
        input: needs.reserve_tokens().saturating_add(images),
        output: u64::from(output),
        ..glave_event::TokenCounts::default()
    };
    Money {
        nanos: price.nanos(&counts),
        currency: Currency::USD,
    }
}
