//! The person's choice whether their computer is kept awake while glave works: kept in the
//! settings ledger in their personal project, glave's own stream under a reserved id, whose
//! latest `awake.set` counts. With none, it is kept awake. Every server on one store reads the
//! same choice.

use anyhow::Context as _;
use glave_event::{
    At, AwakeSet, Body, EventId, Header, PrincipalId, RawEvent, Seq, decode, encode,
};
use glave_ports::stream::{Reserved, append_retrying};
use glave_ports::{Clock as _, EventLog as _, Scope, SystemClock};

use crate::accounts::random;
use crate::data::{Data, DataLog};

/// The type of the choice's event.
const SET: &str = "awake.set";

/// The person's settings ledger, in the store.
#[derive(Clone)]
pub(crate) struct Choice {
    log: DataLog,
    scope: Scope,
}

impl Choice {
    /// The settings ledger of `person`, in their personal project in `store`.
    pub(crate) async fn open(store: &Data, person: PrincipalId) -> anyhow::Result<Self> {
        let scope = crate::projects::personal(store, person).await?;
        Ok(Self {
            log: store.log(),
            scope,
        })
    }

    /// Whether the person wants their computer kept awake: yes until they choose.
    pub(crate) async fn on(&self) -> anyhow::Result<bool> {
        Ok(self.read().await?.is_none_or(|(on, _)| on))
    }

    /// Keeps `on` as the person's choice: appended after the ledger's last event, read again
    /// and tried again when another process wrote first.
    pub(crate) async fn keep(&self, on: bool) -> anyhow::Result<()> {
        let this = self;
        append_retrying(
            &self.log,
            &self.scope,
            Reserved::Settings.id(),
            move || async move {
                let next = this.read().await?.map_or(1, |(_, last)| last + 1);
                anyhow::Ok(Some((vec![event(AwakeSet { on }, next)?], ())))
            },
        )
        .await
        .context("could not keep whether this computer stays awake")?;
        Ok(())
    }

    /// The latest choice and the `seq` it was kept at, if any was.
    async fn read(&self) -> anyhow::Result<Option<(bool, u64)>> {
        let latest = self
            .log
            .latest_of(&self.scope, Reserved::Settings.id(), &[SET])
            .await
            .context("could not read whether this computer stays awake")?;
        let Some(raw) = latest else {
            return Ok(None);
        };
        let seq = raw.seq.get();
        Ok(Some(match decode(raw).map(|event| event.body) {
            Ok(Body::AwakeSet(set)) => (set.on, seq),
            // One this glave cannot read leaves it as it is with none: awake.
            _ => (true, seq),
        }))
    }
}

/// `set` as the ledger's event at `seq`.
fn event(set: AwakeSet, seq: u64) -> anyhow::Result<RawEvent> {
    let now = SystemClock.now();
    let header = Header {
        id: EventId::new(now, random()?)?,
        thread: Reserved::Settings.id(),
        seq: Seq::new(seq)?,
        at: At::from(now),
    };
    Ok(encode(header, &set)?)
}

#[cfg(test)]
mod tests {
    use glave_ports::block_on;

    use super::*;

    #[test]
    fn the_latest_choice_is_kept_in_the_store_for_every_server_on_it() {
        block_on!(async {
            let root = glave_keys::InMemoryRoot::new().unwrap();
            let keys = glave_keys::KeySettings::new(std::sync::Arc::new(root));
            let store = Data::Sqlite(glave_store_sqlite::Store::open_in_memory(keys).unwrap());
            let person = PrincipalId::new(SystemClock.now(), random().unwrap()).unwrap();
            let choice = Choice::open(&store, person).await.unwrap();
            assert!(choice.on().await.unwrap(), "awake until the person chooses");
            choice.keep(false).await.unwrap();
            assert!(!choice.on().await.unwrap());
            choice.keep(true).await.unwrap();
            choice.keep(false).await.unwrap();
            let again = Choice::open(&store, person).await.unwrap();
            assert!(!again.on().await.unwrap());
        });
    }
}
