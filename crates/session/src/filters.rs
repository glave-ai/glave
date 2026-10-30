//! A person's filters among a router's providers (`specs/gateway.md`, "A person's filters"):
//! kept in their settings ledger as `filters.set`, read and chosen by `filters/get` and
//! `filters/set`, and kept to by the gateway at every model call of their work, which asks for
//! them through [`PersonFilters`]: a host hands one to its gateway and to the server it makes,
//! which answers through it from then on.

use std::sync::{Arc, OnceLock, Weak};

use glave_event::{Body, Filters, FiltersSet, Payload, Serve, decode};
use glave_gateway::Filtering;
use glave_ports::stream::Reserved;
use glave_ports::{
    BlobStore, BoxFuture, Caller, EventLog, Failure, ModelProvider, Payer, ProviderError,
};

use crate::protocol::{ApiError, Reply, code};
use crate::server::{Inner, Server};
use crate::words::Sentence;

/// What "Europe" means among the places providers are based in: the European Economic Area,
/// Switzerland and the United Kingdom, by their ISO 3166-1 alpha-2 codes.
const EUROPE: [&str; 32] = [
    "AT", "BE", "BG", "CH", "CY", "CZ", "DE", "DK", "EE", "ES", "FI", "FR", "GB", "GR", "HR", "HU",
    "IE", "IS", "IT", "LI", "LT", "LU", "LV", "MT", "NL", "NO", "PL", "PT", "RO", "SE", "SI", "SK",
];

/// The most countries, providers and models a person's filters name, each list apart.
const MOST_NAMED: usize = 256;

/// The longest name of a provider or a model in a person's filters, in characters.
const LONGEST_NAME: usize = 256;

/// Reads the filters of the person whose work a call is.
type Read =
    dyn Fn(Payer) -> BoxFuture<'static, Result<Option<Filters>, ProviderError>> + Send + Sync;

/// Each person's filters, as the gateway asks for them before each call of their work: read
/// from the server that keeps them once it answers through this. Until then, and once it is
/// gone, every call that asks is refused, so none goes out past filters no one read.
#[derive(Default)]
pub struct PersonFilters {
    server: OnceLock<Box<Read>>,
}

impl std::fmt::Debug for PersonFilters {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PersonFilters")
            .field("answered", &self.server.get().is_some())
            .finish()
    }
}

impl Filtering for PersonFilters {
    fn filters(&self, payer: Payer) -> BoxFuture<'_, Result<Option<Filters>, ProviderError>> {
        match self.server.get() {
            Some(read) => read(payer),
            None => Box::pin(async { Err(unread("glave isn't ready to read them yet")) }),
        }
    }
}

/// The refusal of a call whose filters were not read, saying `why`.
fn unread(why: &str) -> ProviderError {
    ProviderError::Unserved {
        failure: Failure::Unavailable,
        reason: format!("your filters among providers could not be read ({why}): try again"),
    }
}

impl<P, L, B> Server<P, L, B>
where
    P: ModelProvider + 'static,
    L: EventLog + 'static,
    B: BlobStore + 'static,
{
    /// Answers `filters`, which a host handed its gateway, through this server, for as long
    /// as it is there. A host hands each server its own: one answered already stays with the
    /// first.
    pub fn answer_filters(&self, filters: &PersonFilters) {
        let inner: Weak<Inner<P, L, B>> = Arc::downgrade(&self.inner);
        let read: Box<Read> = Box::new(move |payer| {
            let inner = Weak::clone(&inner);
            Box::pin(async move {
                let inner = inner.upgrade().ok_or_else(|| unread("glave is stopping"))?;
                Server { inner }.filters_of(payer).await
            })
        });
        if filters.server.set(read).is_err() {
            tracing::warn!("a person's filters are answered by another server already");
        }
    }

    /// Answers `filters/get`: the caller's filters, none until they choose.
    pub(crate) async fn filters(&self, caller: &Caller) -> Result<Reply, ApiError> {
        Ok(filters_reply(self.kept_filters(caller).await?))
    }

    /// Answers `filters/set`: keeps `filters` as the caller's, for every call of their work
    /// from now on.
    pub(crate) async fn set_filters(
        &self,
        caller: &Caller,
        filters: Filters,
    ) -> Result<Reply, ApiError> {
        checked(&filters)?;
        let set = FiltersSet { filters };
        self.keep_setting(caller, &set).await?;
        Ok(filters_reply(set.filters))
    }

    /// The caller's filters: none until they choose. Refused when the latest they chose cannot
    /// be read, never taken for none.
    pub(crate) async fn kept_filters(&self, caller: &Caller) -> Result<Filters, ApiError> {
        let (_, scope) = self.personal_project(caller).await?;
        let ledger = Reserved::Settings.id();
        let log = self.inner.harness.log();
        let Some(raw) = log.latest_of(&scope, ledger, &[FiltersSet::TYPE]).await? else {
            return Ok(Filters::default());
        };
        match decode(raw).map(|event| event.body) {
            Ok(Body::FiltersSet(set)) => Ok(set.filters),
            _ => Err(ApiError::new(code::INTERNAL, Sentence::FiltersUnreadable)),
        }
    }

    /// The filters of `payer`'s person, or of the person this server works for when it names
    /// none: none when they chose none; refused when they cannot be read.
    async fn filters_of(&self, payer: Payer) -> Result<Option<Filters>, ProviderError> {
        let caller = Caller {
            person: payer.person,
            org: Some(payer.org),
            ..Caller::default()
        };
        let filters = self.kept_filters(&caller).await.map_err(|error| {
            tracing::warn!(%error, "a person's filters among providers could not be read");
            unread("glave could not read them")
        })?;
        Ok((filters != Filters::default()).then_some(filters))
    }
}

/// The reply that says `filters`, with what "Europe" means among them.
fn filters_reply(filters: Filters) -> Reply {
    Reply::Filters {
        filters,
        europe: EUROPE.iter().map(|code| (*code).to_owned()).collect(),
    }
}

/// Refuses filters a person can't have meant: a country that is not two capital letters, a
/// provider that is not a slug, a blank or overlong model, a price limit that names no price,
/// or more of any than [`MOST_NAMED`].
fn checked(filters: &Filters) -> Result<(), ApiError> {
    let invalid = |sentence| Err(ApiError::new(code::INVALID_PARAMS, sentence));
    let counts = [
        filters.based_in.len(),
        filters.avoid.len(),
        filters.serves.len(),
    ];
    if counts.iter().any(|count| *count > MOST_NAMED) {
        return invalid(Sentence::FiltersTooMany { most: MOST_NAMED });
    }
    if let Some(place) = filters
        .based_in
        .iter()
        .find(|place| place.len() != 2 || !place.bytes().all(|b| b.is_ascii_uppercase()))
    {
        return invalid(Sentence::NotACountry {
            place: place.escape_debug().to_string(),
        });
    }
    if filters
        .max_price
        .is_some_and(|most| most.input.is_none() && most.output.is_none())
    {
        return invalid(Sentence::NoMostPrice);
    }
    let chosen = filters.serves.values().filter_map(|serve| match serve {
        Serve::Provider(slug) => Some(slug),
        Serve::Sort(_) => None,
    });
    if let Some(slug) = filters
        .avoid
        .iter()
        .chain(chosen)
        .find(|slug| !is_slug(slug))
    {
        return invalid(Sentence::NotAProvider {
            provider: slug.escape_debug().to_string(),
        });
    }
    let long = |model: &&String| model.trim().is_empty() || model.chars().count() > LONGEST_NAME;
    if filters.serves.keys().any(|model| long(&model)) {
        return invalid(Sentence::FilterName { most: LONGEST_NAME });
    }
    Ok(())
}

/// Whether `slug` names a provider as a router does: lower case letters, digits and dashes.
fn is_slug(slug: &str) -> bool {
    !slug.is_empty()
        && slug.len() <= LONGEST_NAME
        && slug
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}
