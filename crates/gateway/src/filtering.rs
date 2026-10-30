//! A person's filters among a router's providers (`specs/gateway.md`, "A person's filters"):
//! before each call that says whose work it is, where a router (OpenRouter) is among its
//! targets, the gateway asks the host that keeps people's choices for theirs, and keeps to them
//! at each target. Only a router can: a call they narrow goes to no other target where a router
//! serves it, and is left alone where none does. At a router, the providers that fit are worked
//! out from what its catalog says of them, the target's [`Router`], and sent as what the router
//! takes (`only`), with the filters it checks itself; a call no provider fits, or one whose
//! filters need what the catalog does not say, is never sent.

use std::collections::BTreeSet;
use std::sync::Arc;

use glave_event::{CatalogEndpoint, CatalogProvider, Filters, Hides, Precision, Serve, TokenPrice};
use glave_ports::{BoxFuture, Payer, ProviderError, Route};

use crate::target::Target;

/// Where a gateway finds the filters of a person whose work a call is.
pub trait Filtering: Send + Sync + std::fmt::Debug {
    /// `payer`'s person's filters; none when they chose none. An error stops the call, which is
    /// never sent past filters that could not be read.
    fn filters(&self, payer: Payer) -> BoxFuture<'_, Result<Option<Filters>, ProviderError>>;
}

/// What a router's catalog says of who serves a target's model: its providers, each with where
/// it is based, shared by every model of the catalog, and the model's endpoints. Either is
/// empty while the catalog says nothing of it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Router {
    pub providers: Arc<[CatalogProvider]>,
    pub endpoints: Vec<CatalogEndpoint>,
}

/// What a router's catalog says of one provider serving a model, as filters read it.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Facts<'a> {
    /// The provider, by slug.
    pub(crate) slug: &'a str,
    /// The country it is based in, when said.
    pub(crate) home: Option<&'a str>,
    /// The countries of its datacenters, when said.
    pub(crate) datacenters: Option<&'a [String]>,
    pub(crate) precision: Precision,
    pub(crate) price: Option<TokenPrice>,
    pub(crate) keeps_nothing: bool,
}

/// Which of `filters` hide the provider `facts` describe, as far as they say: what is not said
/// never fits a filter that needs it. None when it fits them all.
pub(crate) fn hidden_by(facts: &Facts<'_>, filters: &Filters) -> Vec<Hides> {
    let precise = filters
        .precision
        .is_none_or(|least| least == Precision::Unstated || facts.precision >= least);
    let priced = filters.max_price.is_none_or(|most| {
        facts.price.is_some_and(|price| {
            most.input.is_none_or(|input| price.input <= input)
                && most.output.is_none_or(|output| price.output <= output)
        })
    });
    let based = based(facts.home, facts.datacenters, &filters.based_in);
    [
        (!based, Hides::BasedIn),
        (!precise, Hides::Precision),
        (
            filters.keeps_nothing && !facts.keeps_nothing,
            Hides::KeepsSome,
        ),
        (!priced, Hides::Price),
        (
            filters.avoid.iter().any(|slug| slug == facts.slug),
            Hides::Avoided,
        ),
    ]
    .into_iter()
    .filter_map(|(hidden, by)| hidden.then_some(by))
    .collect()
}

/// Whether a provider at `home`, with `datacenters`, is based in one of `places`, anywhere
/// when none: its home, and each of its datacenters where they are said.
fn based(home: Option<&str>, datacenters: Option<&[String]>, places: &[String]) -> bool {
    let within = |country: &str| places.iter().any(|place| place == country);
    places.is_empty()
        || (home.is_some_and(within)
            && datacenters.is_none_or(|datacenters| datacenters.iter().all(|c| within(c))))
}

/// Why filters keep a call from a target.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Kept {
    /// They narrow who may serve it, and only a router keeps to that.
    NotARouter,
    /// They choose by where providers are based, which its catalog does not say.
    Unknown,
    /// None of its providers fits them.
    NoneFits,
}

/// What `filters` ask of `target` for the model a call asked for, `model`, where `beside_one`
/// says a router is among the call's targets. None when they ask nothing of it.
pub(crate) fn route(
    filters: &Filters,
    model: &str,
    target: &Target,
    beside_one: bool,
) -> Result<Option<Route>, Kept> {
    let names = [model, target.model.as_str()];
    route_at(filters, names, target.router.as_deref(), beside_one)
}

/// What `filters` ask of a target, the router `router` or none, serving the model a call asked
/// for by `names` (the call's name for it, then the target's own), where `beside_one` says a
/// router is among the call's targets. None when they ask nothing.
fn route_at(
    filters: &Filters,
    names: [&str; 2],
    router: Option<&Router>,
    beside_one: bool,
) -> Result<Option<Route>, Kept> {
    // The model a thread asked for, else the router's own name for it, which stays when
    // another service's account comes to serve it first.
    let serve = names.iter().find_map(|name| filters.serves.get(*name));
    let sort = match serve {
        Some(Serve::Sort(sort)) => Some(*sort),
        Some(Serve::Provider(_)) => None,
        None => filters.sort,
    };
    let chosen = match serve {
        Some(Serve::Provider(slug)) => Some(slug.as_str()),
        _ => None,
    };
    let precision = filters.precision.unwrap_or(Precision::Unstated);
    let narrows = !filters.based_in.is_empty()
        || precision != Precision::Unstated
        || filters.keeps_nothing
        || filters.max_price.is_some()
        || !filters.avoid.is_empty()
        || chosen.is_some();
    let Some(router) = router else {
        return if narrows && beside_one {
            Err(Kept::NotARouter)
        } else {
            Ok(None)
        };
    };
    // Where it is based and the one chosen are what the router has no filter of its own for:
    // they go as the providers it may use. The rest it checks itself, against what it knows
    // now; where its catalog lists the model's endpoints, a call none fits is never sent.
    let own = !filters.based_in.is_empty() || chosen.is_some();
    let only = match (narrows, router.endpoints.is_empty()) {
        (false, _) => None,
        (true, true) => own
            .then(|| by_provider(filters, chosen, router))
            .transpose()?,
        (true, false) => {
            let fitting = by_endpoint(filters, chosen, router)?;
            own.then_some(fitting)
        }
    };
    let route = Route {
        only,
        ignore: filters.avoid.clone(),
        quantizations: precision
            .at_least()
            .into_iter()
            .map(str::to_owned)
            .collect(),
        sort,
        zdr: filters.keeps_nothing,
        max_price: filters.max_price,
    };
    Ok((route != Route::default()).then_some(route))
}

/// The providers of `router` that fit where `filters` say they are based, and `chosen` when
/// one is, by slug, where its catalog does not list the model's endpoints.
fn by_provider(
    filters: &Filters,
    chosen: Option<&str>,
    router: &Router,
) -> Result<Vec<String>, Kept> {
    if filters.based_in.is_empty() {
        // A provider chosen that its catalog does not list yet is the router's to find.
        let allowed = chosen.filter(|chosen| !filters.avoid.iter().any(|slug| slug == chosen));
        return owned(allowed.into_iter().collect());
    }
    if router.providers.is_empty() {
        return Err(Kept::Unknown);
    }
    let fitting: BTreeSet<&str> = router
        .providers
        .iter()
        .filter(|provider| chosen.is_none_or(|chosen| chosen == provider.slug))
        .filter(|provider| !filters.avoid.contains(&provider.slug))
        .filter(|provider| {
            let datacenters = provider.datacenters.as_deref();
            based(
                provider.headquarters.as_deref(),
                datacenters,
                &filters.based_in,
            )
        })
        .map(|provider| provider.slug.as_str())
        .collect();
    owned(fitting)
}

/// The providers of `router`'s endpoints of the model that fit `filters`, and `chosen` when
/// one is, by slug.
fn by_endpoint(
    filters: &Filters,
    chosen: Option<&str>,
    router: &Router,
) -> Result<Vec<String>, Kept> {
    let fitting: BTreeSet<&str> = router
        .endpoints
        .iter()
        .filter(|endpoint| chosen.is_none_or(|chosen| chosen == endpoint.provider))
        .filter(|endpoint| hidden_by(&facts(endpoint, router), filters).is_empty())
        .map(|endpoint| endpoint.provider.as_str())
        .collect();
    owned(fitting)
}

/// `fitting`, refused when it is empty.
fn owned(fitting: BTreeSet<&str>) -> Result<Vec<String>, Kept> {
    if fitting.is_empty() {
        return Err(Kept::NoneFits);
    }
    Ok(fitting.into_iter().map(str::to_owned).collect())
}

/// What `router`'s catalog says of `endpoint`'s provider.
pub(crate) fn facts<'a>(endpoint: &'a CatalogEndpoint, router: &'a Router) -> Facts<'a> {
    let provider = router
        .providers
        .iter()
        .find(|provider| provider.slug == endpoint.provider);
    Facts {
        slug: &endpoint.provider,
        home: provider.and_then(|provider| provider.headquarters.as_deref()),
        datacenters: provider.and_then(|provider| provider.datacenters.as_deref()),
        precision: Precision::of(endpoint.quantization.as_deref()),
        price: endpoint.price,
        keeps_nothing: endpoint.zdr,
    }
}

#[cfg(test)]
mod tests;
