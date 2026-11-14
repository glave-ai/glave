//! The person's accounts ledger (`glave_session::AccountLedger`), in their personal project in
//! the one data store.

use std::sync::Arc;

use glave_event::PrincipalId;
use glave_ports::{OrgId, OsEntropy, Scope, SystemClock};

use crate::data::{Data, DataLog};

/// The person's accounts, in the store.
pub(crate) type Ledger = glave_session::AccountLedger<DataLog>;

/// The ledger of `person`, in their personal project in `store`.
pub(crate) async fn open(store: &Data, person: PrincipalId) -> anyhow::Result<Ledger> {
    let scope = crate::projects::personal(store, person).await?;
    Ok(in_scope(store, scope, person))
}

/// The ledger of `person` in `org`, in their personal project there.
pub(crate) async fn open_in(
    store: &Data,
    org: OrgId,
    person: PrincipalId,
) -> anyhow::Result<Ledger> {
    let scope = crate::projects::personal_in(store, org, person).await?;
    Ok(in_scope(store, scope, person))
}

/// The ledger of `person` in their personal project `scope`.
fn in_scope(store: &Data, scope: Scope, person: PrincipalId) -> Ledger {
    Ledger::new(
        store.log(),
        store.eraser(),
        scope,
        person,
        Arc::new(SystemClock),
        Arc::new(OsEntropy),
    )
}

/// Random bytes for a new id, from the operating system.
pub(crate) fn random() -> anyhow::Result<[u8; 10]> {
    let mut bytes = [0u8; 10];
    getrandom::fill(&mut bytes)
        .map_err(|e| anyhow::anyhow!("could not get random bytes for a new id: {e}"))?;
    Ok(bytes)
}

/// A new person's ledger, on a store in memory.
#[cfg(test)]
pub(crate) async fn in_memory() -> anyhow::Result<Ledger> {
    use glave_ports::Clock as _;
    let root = glave_keys::InMemoryRoot::new()?;
    let keys = glave_keys::KeySettings::new(Arc::new(root));
    let store = Data::Sqlite(glave_store_sqlite::Store::open_in_memory(keys)?);
    let person = PrincipalId::new(SystemClock.now(), random()?)?;
    open(&store, person).await
}
