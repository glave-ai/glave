//! Daily work in every organization on the store, one at a time, each under a lease of its own
//! kind, so two servers on one store never do one organization's at once. One another server
//! holds, or one that fails, waits for the next day, and work that outruns its lease is stopped.

use std::future::Future;
use std::time::Duration;

use anyhow::Context as _;
use glave_ports::{Holder, Leased, Leases, OrgId, spoken};
use tokio::time::timeout;

/// Does `work` in each of `organizations` whose lease of `kind` no other server holds, as
/// `holder`, for at most `within`, and tells `said` what each came to: what it did, none while
/// another server holds its lease, or why it failed.
pub(super) async fn every_organization<
    T,
    E: Into<anyhow::Error>,
    F: Future<Output = Result<T, E>>,
>(
    organizations: Vec<OrgId>,
    leases: &dyn Leases,
    (holder, kind, within): (Holder, Leased, Duration),
    work: impl Fn(OrgId) -> F,
    said: impl Fn(OrgId, anyhow::Result<Option<T>>),
) {
    for org in organizations {
        let done = async {
            let taken = leases.acquire(org, kind, holder, within).await;
            let Some(lease) = taken.context("could not take its lease")? else {
                return Ok(None);
            };
            let done = timeout(within, work(org)).await;
            if let Err(error) = leases.release(&lease).await {
                tracing::warn!(%org, %error, ?kind, "a lease could not be let go; it lapses");
            }
            let done =
                done.with_context(|| format!("it was not done within {}", spoken(within)))?;
            Ok(Some(done.map_err(Into::into)?))
        };
        said(org, done.await);
    }
}
