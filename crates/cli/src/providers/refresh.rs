//! Refreshing the catalogs (`specs/gateway.md`, "Catalogs"): each account's service lists its
//! models again, daily and when the person asks, so a new model, a price or a reasoning level
//! shows up without a new glave. A catalog a day old is listed again when the server is next
//! asked for its models or starts a turn; one whose service cannot be reached stays as it was,
//! and is tried again an hour later. The store is written only when a catalog changed.

use std::sync::{Mutex, PoisonError};
use std::time::{Duration, Instant};

use glave_event::{At, Catalog, CatalogModel, ProviderConnected, ProviderKind};
use glave_ports::{Clock as _, Failure, SystemClock};
use glave_session::Refreshed;
use glave_session::protocol::ApiError;

use super::{
    ModelAccounts, catalog_of, check, endpoint_of, endpoints, invalid, provider_failed, refused,
    service,
};
use crate::model::Model;

/// How old a catalog grows before it is listed again.
const DAY: Duration = Duration::from_hours(24);

/// How long after a refresh that failed, or one under way, the next is tried.
const RETRY: Duration = Duration::from_hours(1);

/// Whether a refresh is due by `due`, claiming it: the next is due an hour on, unless the one
/// claimed moves it.
pub(super) fn claim(due: &Mutex<Option<Instant>>) -> bool {
    let mut due = due.lock().unwrap_or_else(PoisonError::into_inner);
    let now = Instant::now();
    if due.is_some_and(|due| due > now) {
        return false;
    }
    *due = Some(now + RETRY);
    true
}

/// `catalogs` with `fresh` in place of its service's.
pub(super) fn with(mut catalogs: Vec<Catalog>, fresh: Catalog) -> Vec<Catalog> {
    catalogs.retain(|kept| kept.kind != fresh.kind || kept.url != fresh.url);
    catalogs.push(fresh);
    catalogs
}

/// Whether `catalog` was listed under a day before `now`, a router's with its providers: one
/// listed when its account was added, without them, is due at once.
fn fresh(catalog: &Catalog, now: At) -> bool {
    let age = now.timestamp().duration_since(catalog.listed.timestamp());
    let whole = catalog.kind != ProviderKind::Openrouter || !catalog.providers.is_empty();
    whole && age.is_positive() && age.unsigned_abs() < DAY
}

/// Whether `before` and `after` hold the same models, and a router's the same providers, for
/// the same services, whenever each was listed. A router's speeds, which change with each
/// listing, count only when anything else changed.
fn same(before: &[Catalog], after: &[Catalog]) -> bool {
    before.len() == after.len()
        && after.iter().all(|catalog| {
            before.iter().any(|kept| {
                kept.kind == catalog.kind
                    && kept.url == catalog.url
                    && kept.providers == catalog.providers
                    && unhurried(&kept.models) == unhurried(&catalog.models)
            })
        })
}

/// `models` with no speed said for any endpoint.
fn unhurried(models: &[CatalogModel]) -> Vec<CatalogModel> {
    let mut models = models.to_vec();
    for endpoint in models.iter_mut().flat_map(|model| &mut model.endpoints) {
        endpoint.speed = None;
    }
    models
}

impl ModelAccounts {
    /// Lists the models of every account's service again, the server's own among them: those
    /// whose catalog is a day old or more, or with `all`, every one. Keeps the catalogs when any changed; a service that
    /// cannot be listed keeps its own, and says why.
    pub(super) async fn refresh(&self, all: bool) -> Result<Refreshed, ApiError> {
        let own = self.lineup().own_account();
        let connected = self.ledger.accounts().await?.providers;
        let accounts: Vec<ProviderConnected> = own.into_iter().chain(connected).collect();
        let kept = self.kept_catalogs().await?;
        let now = At::from(SystemClock.now());
        let mut catalogs: Vec<Catalog> = Vec::new();
        let mut failed = None;
        for account in &accounts {
            let old = catalog_of(account, &kept);
            if catalog_of(account, &catalogs).is_some() {
                continue;
            }
            if !all && old.is_some_and(|catalog| fresh(catalog, now)) {
                catalogs.extend(old.cloned());
                continue;
            }
            match listed(account, old, now).await {
                Ok((catalog, late)) => {
                    catalogs.push(catalog);
                    if let Some(late) = late {
                        failed.get_or_insert(late);
                    }
                }
                Err(error) => {
                    catalogs.extend(old.cloned());
                    failed.get_or_insert(error);
                }
            }
        }
        // An account connected while these were listed keeps the catalog its connecting kept.
        let serves =
            |one: &Catalog, other: &Catalog| one.kind == other.kind && one.url == other.url;
        for written in self.kept_catalogs().await? {
            let new = !kept.iter().any(|before| serves(before, &written));
            if new && !catalogs.iter().any(|own| serves(own, &written)) {
                catalogs.push(written);
            }
        }
        let changed = !same(&kept, &catalogs);
        if changed {
            self.catalogs
                .keep(catalogs)
                .await
                .map_err(|e| ApiError::internal(&e))?;
        }
        let next = if failed.is_some() { RETRY } else { DAY };
        *self.due.lock().unwrap_or_else(PoisonError::into_inner) = Some(Instant::now() + next);
        Ok(Refreshed { changed, failed })
    }
}

/// The catalog of `account`'s service, listed now with its key, and why it was listed only in
/// part, a router's models keeping the endpoints `old` held where theirs were not read in time;
/// refused when it lists no model glave can talk with, or a router's providers cannot be read.
async fn listed(
    account: &ProviderConnected,
    old: Option<&Catalog>,
    now: At,
) -> Result<(Catalog, Option<ApiError>), ApiError> {
    let endpoint = endpoint_of(account, "").ok_or_else(|| invalid(super::NOT_AN_ADDRESS))?;
    let mut models = check::models(account.kind, &endpoint)
        .await
        .map_err(|error| refused(account, &error))?;
    if models.is_empty() {
        return Err(provider_failed(
            Failure::UnknownModel,
            format!(
                "{} lists no models glave can talk with on this account any more",
                service(account)
            ),
        ));
    }
    let mut late = None;
    let providers = if account.kind == ProviderKind::Openrouter {
        let model = Model::for_endpoint(&endpoint).map_err(|e| ApiError::internal(&e))?;
        let before = old.map_or(&[][..], |old| old.models.as_slice());
        let unread = |why: String| {
            let service = service(account);
            provider_failed(
                Failure::Unavailable,
                format!("{service} could not say who serves its models: {why}"),
            )
        };
        match endpoints::read(model, &mut models, before).await {
            Ok((providers, whole)) => {
                if !whole {
                    late = Some(unread(
                        "some models' providers were not read in time".to_owned(),
                    ));
                }
                providers
            }
            // With no catalog yet its models' prices are worth keeping without their providers:
            // the listing is due again, and filters by country refuse until then.
            Err(why) if old.is_none() => {
                late = Some(unread(why));
                Vec::new()
            }
            Err(why) => return Err(unread(why)),
        }
    } else {
        Vec::new()
    };
    let catalog = Catalog {
        kind: account.kind,
        url: account.url.clone(),
        listed: now,
        models,
        providers,
    };
    Ok((catalog, late))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn catalog(kind: ProviderKind, listed: At, models: &[&str]) -> Catalog {
        Catalog {
            kind,
            url: None,
            listed,
            models: models.iter().map(|id| CatalogModel::new(*id)).collect(),
            providers: Vec::new(),
        }
    }

    fn at(seconds: i64) -> At {
        At::from(jiff::Timestamp::from_second(seconds).unwrap())
    }

    #[test]
    fn a_catalog_is_fresh_for_a_day_and_a_router_s_only_once_its_providers_are_listed() {
        let mut listed = catalog(ProviderKind::Openrouter, at(1_000_000), &["a/one"]);
        assert!(
            !fresh(&listed, at(1_000_000 + 60)),
            "added, its endpoints not read yet"
        );
        listed.providers.push(glave_event::CatalogProvider {
            slug: "acme".into(),
            name: "Acme".into(),
            headquarters: None,
            datacenters: None,
        });
        assert!(fresh(&listed, at(1_000_000 + 60)));
        assert!(!fresh(&listed, at(1_000_000 + 24 * 60 * 60)));
        assert!(!fresh(&listed, at(1_000_000 - 60)), "listed in the future");
    }

    #[test]
    fn catalogs_are_the_same_when_their_models_are_whenever_they_were_listed() {
        let before = [
            catalog(ProviderKind::Openrouter, at(1), &["a/one"]),
            catalog(ProviderKind::Openai, at(1), &["gpt-6"]),
        ];
        let relisted = [
            catalog(ProviderKind::Openai, at(9), &["gpt-6"]),
            catalog(ProviderKind::Openrouter, at(9), &["a/one"]),
        ];
        assert!(same(&before, &relisted));
        let added = [
            catalog(ProviderKind::Openrouter, at(9), &["a/two", "a/one"]),
            catalog(ProviderKind::Openai, at(9), &["gpt-6"]),
        ];
        assert!(!same(&before, &added));
        assert!(!same(&before, &before[..1]), "an account removed");
        let replaced = with(before.to_vec(), added[0].clone());
        assert_eq!(replaced.len(), 2);
        assert_eq!(replaced[1], added[0]);
    }

    #[test]
    fn a_refresh_is_claimed_once_until_it_is_due_again() {
        let due = Mutex::new(None);
        assert!(claim(&due), "due at once after a start");
        assert!(!claim(&due), "claimed already");
        *due.lock().unwrap() = Some(Instant::now());
        assert!(claim(&due));
    }
}
