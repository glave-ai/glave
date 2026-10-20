//! Where the catalogs of a person's model accounts are kept: the catalog ledger in their
//! personal project, glave's own stream under a reserved id, each `catalog.refreshed` holding
//! every service's catalog and replacing the one before. Only the latest is ever read, so the
//! ledger costs one event to read however often it changed. Every server on one store reads
//! the same catalogs.

use anyhow::Context as _;
use glave_event::{
    At, Body, Catalog, CatalogRefreshed, EventId, Header, PrincipalId, RawEvent, Seq, decode,
    encode,
};
use glave_ports::stream::{Reserved, append_retrying};
use glave_ports::{Clock as _, EventLog as _, Scope, SystemClock};

use crate::accounts::random;
use crate::data::{Data, DataLog};

/// The type of the ledger's one event.
const REFRESHED: &str = "catalog.refreshed";

/// The person's catalog ledger, in the store.
#[derive(Clone)]
pub(crate) struct Catalogs {
    log: DataLog,
    scope: Scope,
}

impl Catalogs {
    /// The catalog ledger of `person`, in their personal project in `store`.
    pub(crate) async fn open(store: &Data, person: PrincipalId) -> anyhow::Result<Self> {
        let scope = crate::projects::personal(store, person).await?;
        Ok(Self {
            log: store.log(),
            scope,
        })
    }

    /// The catalogs kept: none before the first is.
    pub(crate) async fn latest(&self) -> anyhow::Result<Vec<Catalog>> {
        Ok(self
            .read()
            .await?
            .map(|(catalogs, _)| catalogs)
            .unwrap_or_default())
    }

    /// Keeps `catalogs` in place of those kept: appended after the ledger's last event, read
    /// again and tried again when another process wrote first.
    pub(crate) async fn keep(&self, catalogs: Vec<Catalog>) -> anyhow::Result<()> {
        let (this, refreshed) = (self, &CatalogRefreshed { catalogs });
        append_retrying(
            &self.log,
            &self.scope,
            Reserved::Catalogs.id(),
            move || async move {
                let next = this.read().await?.map_or(1, |(_, last)| last + 1);
                anyhow::Ok(Some((vec![event(refreshed, next)?], ())))
            },
        )
        .await
        .context("could not keep the models' catalogs")?;
        Ok(())
    }

    /// The latest catalogs and the `seq` they were kept at, if any were.
    async fn read(&self) -> anyhow::Result<Option<(Vec<Catalog>, u64)>> {
        let ledger = Reserved::Catalogs.id();
        let latest = self
            .log
            .latest_of(&self.scope, ledger, &[REFRESHED])
            .await
            .context("could not read the models' catalogs")?;
        let Some(raw) = latest else {
            return Ok(None);
        };
        let seq = raw.seq.get();
        Ok(Some(match decode(raw).map(|event| event.body) {
            Ok(Body::CatalogRefreshed(refreshed)) => (refreshed.catalogs, seq),
            // One this glave cannot read is refreshed in its place.
            _ => (Vec::new(), seq),
        }))
    }
}

/// `refreshed` as the ledger's event at `seq`.
fn event(refreshed: &CatalogRefreshed, seq: u64) -> anyhow::Result<RawEvent> {
    let now = SystemClock.now();
    let header = Header {
        id: EventId::new(now, random()?)?,
        thread: Reserved::Catalogs.id(),
        seq: Seq::new(seq)?,
        at: At::from(now),
    };
    Ok(encode(header, refreshed)?)
}

#[cfg(test)]
mod tests {
    use glave_event::{CatalogModel, ProviderKind};
    use glave_ports::block_on;

    use super::*;

    fn catalog(kind: ProviderKind, models: &[&str]) -> Catalog {
        Catalog {
            kind,
            url: None,
            listed: At::from(SystemClock.now()),
            models: models.iter().map(|id| CatalogModel::new(*id)).collect(),
        }
    }

    #[test]
    fn the_latest_catalogs_are_kept_in_the_store_for_every_server_on_it() {
        block_on!(async {
            let root = glave_keys::InMemoryRoot::new().unwrap();
            let keys = glave_keys::KeySettings::new(std::sync::Arc::new(root));
            let store = Data::Sqlite(glave_store_sqlite::Store::open_in_memory(keys).unwrap());
            let person = PrincipalId::new(SystemClock.now(), random().unwrap()).unwrap();
            let catalogs = Catalogs::open(&store, person).await.unwrap();
            assert_eq!(catalogs.latest().await.unwrap(), Vec::<Catalog>::new());
            let first = vec![catalog(ProviderKind::Openrouter, &["a/one"])];
            catalogs.keep(first).await.unwrap();
            let second = vec![
                catalog(ProviderKind::Openrouter, &["a/two", "a/one"]),
                catalog(ProviderKind::Openai, &["gpt-6"]),
            ];
            catalogs.keep(second.clone()).await.unwrap();
            let again = Catalogs::open(&store, person).await.unwrap();
            assert_eq!(again.latest().await.unwrap(), second);
        });
    }
}
