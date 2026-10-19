//! Where a person's project trust is kept: the trust ledger in their personal project, glave's
//! own stream under a reserved id, each `trust.decided` replacing what was decided before about
//! its project. Every server on one store reads the same decisions.

use anyhow::Context as _;
use glave_config::trust::{Entry, Record};
use glave_event::{
    At, Body, EventId, Header, Principal, PrincipalId, RawEvent, Seq, decode, encode,
};
use glave_ports::Scope;
use glave_ports::stream::{Reserved, append_retrying, catch_up};
use jiff::Timestamp;

use crate::accounts::random;
use crate::data::{Data, DataLog};

/// The person's trust ledger, in the store.
#[derive(Clone)]
pub struct Trust {
    log: DataLog,
    scope: Scope,
    person: PrincipalId,
}

impl Trust {
    /// The trust ledger of `person`, in their personal project in `store`.
    pub async fn open(store: &Data, person: PrincipalId) -> anyhow::Result<Self> {
        let scope = crate::projects::personal(store, person).await?;
        Ok(Self {
            log: store.log(),
            scope,
            person,
        })
    }

    /// Every project's latest decisions.
    pub async fn record(&self) -> anyhow::Result<Record> {
        Ok(self.read().await?.0)
    }

    /// Keeps `entry`, replacing what was decided about its project: appended after the
    /// ledger's last event, read again and tried again when another process wrote first.
    pub async fn keep(&self, mut entry: Entry) -> anyhow::Result<()> {
        entry.by = Some(Principal::person(self.person));
        let (this, entry) = (self, &entry);
        append_retrying(
            &self.log,
            &self.scope,
            Reserved::Trust.id(),
            move || async move {
                let (_, next) = this.read().await?;
                anyhow::Ok(Some((vec![event(entry, next)?], ())))
            },
        )
        .await
        .context("could not keep the project's trust")?;
        Ok(())
    }

    /// Reads the whole ledger: the record, and the `seq` its next event takes.
    async fn read(&self) -> anyhow::Result<(Record, u64)> {
        let mut record = Record::default();
        let mut next = 1;
        catch_up(&self.log, &self.scope, Reserved::Trust.id(), None, |page| {
            for raw in page {
                next = raw.seq.get().saturating_add(1);
                if let Ok(event) = decode(raw)
                    && let Body::TrustDecided(decided) = event.body
                {
                    record.take(decided);
                }
            }
        })
        .await
        .context("could not read project trust")?;
        Ok((record, next))
    }
}

/// `entry` as the ledger's event at `seq`.
fn event(entry: &Entry, seq: u64) -> anyhow::Result<RawEvent> {
    let now = Timestamp::now();
    let header = Header {
        id: EventId::new(now, random()?)?,
        thread: Reserved::Trust.id(),
        seq: Seq::new(seq)?,
        at: At::from(now),
    };
    Ok(encode(header, entry)?)
}

#[cfg(test)]
impl Trust {
    /// A new person's trust ledger on a store in memory; and the store.
    pub async fn in_memory() -> anyhow::Result<(Data, Self)> {
        let root = glave_keys::InMemoryRoot::new()?;
        let keys = glave_keys::KeySettings::new(std::sync::Arc::new(root));
        let store = Data::Sqlite(glave_store_sqlite::Store::open_in_memory(keys)?);
        let person = PrincipalId::new(Timestamp::now(), random()?)?;
        let trust = Self::open(&store, person).await?;
        Ok((store, trust))
    }
}

#[cfg(test)]
mod tests;
