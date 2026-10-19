//! Where a person's project trust is kept: the trust ledger in their personal space, glave's
//! own stream under a reserved id, each `trust.decided` replacing what was decided before about
//! its project. Every server on one store reads the same decisions. A `trust.toml` an earlier
//! glave kept in its folder is moved in the first time the ledger is opened, then deleted:
//! decisions about projects the ledger already knows stay the ledger's, so moving it again
//! after a crash changes nothing.

use std::io::ErrorKind;
use std::path::Path;

use anyhow::Context as _;
use glave_config::trust::{Entry, FILE, Record};
use glave_event::{
    At, Body, EventId, Header, Principal, PrincipalId, RawEvent, Seq, decode, encode,
};
use glave_ports::{EventLog, LogError, Scope};
use glave_session::trust_ledger;
use jiff::Timestamp;

use crate::accounts::random;
use crate::data::{Data, DataLog};

/// Ledger events read per page.
const PAGE: usize = 500;
/// Appends tried before giving up on a ledger another process keeps moving.
const ATTEMPTS: usize = 8;

/// The person's trust ledger, in the store.
#[derive(Clone)]
pub struct Trust {
    log: DataLog,
    scope: Scope,
    person: PrincipalId,
}

impl Trust {
    /// The trust ledger of `person`, in their personal space in `store`, with what `home`'s
    /// earlier `trust.toml` held moved in.
    pub async fn open(store: &Data, person: PrincipalId, home: &Path) -> anyhow::Result<Self> {
        let scope = crate::spaces::personal(store, person).await?;
        let trust = Self {
            log: store.log(),
            scope,
            person,
        };
        trust.move_in(&home.join(FILE)).await?;
        Ok(trust)
    }

    /// Every project's latest decisions.
    pub async fn record(&self) -> anyhow::Result<Record> {
        Ok(self.read().await?.0)
    }

    /// Keeps `entry`, replacing what was decided about its project.
    pub async fn keep(&self, entry: Entry) -> anyhow::Result<()> {
        self.append(|_| vec![entry.clone()]).await
    }

    /// Moves the decisions of the file at `path` into the ledger, all in one append, then
    /// deletes it.
    async fn move_in(&self, path: &Path) -> anyhow::Result<()> {
        let text = match tokio::fs::read_to_string(path).await {
            Ok(text) => text,
            Err(error) if error.kind() == ErrorKind::NotFound => return Ok(()),
            Err(error) => {
                return Err(error).with_context(|| format!("could not read {}", path.display()));
            }
        };
        let earlier = Record::earlier(&text, path)?;
        self.append(|record| {
            earlier
                .iter()
                .filter(|entry| !record.entries().iter().any(|kept| kept.path == entry.path))
                .cloned()
                .collect()
        })
        .await?;
        match tokio::fs::remove_file(path).await {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == ErrorKind::NotFound => Ok(()),
            Err(error) => {
                Err(error).with_context(|| format!("could not remove {}", path.display()))
            }
        }
    }

    /// Appends the entries `change` makes of the record, read again and tried again when
    /// another process wrote first; nothing when it makes none.
    async fn append(&self, change: impl Fn(&Record) -> Vec<Entry>) -> anyhow::Result<()> {
        for _ in 0..ATTEMPTS {
            let (record, next) = self.read().await?;
            let entries = change(&record);
            if entries.is_empty() {
                return Ok(());
            }
            let by = Principal::person(self.person);
            let mut raw = Vec::with_capacity(entries.len());
            for (n, mut entry) in entries.into_iter().enumerate() {
                entry.by = Some(by.clone());
                let seq = next.saturating_add(u64::try_from(n)?);
                raw.push(event(&entry, seq)?);
            }
            match self.log.append(&self.scope, &raw).await {
                Ok(()) => return Ok(()),
                Err(LogError::Conflict { .. }) => {}
                Err(error) => return Err(error).context("could not keep the project's trust"),
            }
        }
        anyhow::bail!("project trust kept changing while glave wrote to it: try again")
    }

    /// Reads the whole ledger: the record, and the `seq` its next event takes.
    async fn read(&self) -> anyhow::Result<(Record, u64)> {
        let ledger = ledger()?;
        let mut record = Record::default();
        let mut next = 1;
        let mut after = None;
        loop {
            let page = self
                .log
                .read(&self.scope, ledger, after, PAGE)
                .await
                .context("could not read project trust")?;
            for raw in &page {
                next = raw.seq.get().saturating_add(1);
                if let Ok(event) = decode(raw.clone())
                    && let Body::TrustDecided(decided) = event.body
                {
                    record.take(decided);
                }
            }
            if page.len() < PAGE {
                return Ok((record, next));
            }
            after = page.last().map(|event| event.seq);
        }
    }
}

/// `entry` as the ledger's event at `seq`.
fn event(entry: &Entry, seq: u64) -> anyhow::Result<RawEvent> {
    let now = Timestamp::now();
    let header = Header {
        id: EventId::new(now, random()?)?,
        thread: ledger()?,
        seq: Seq::new(seq)?,
        at: At::from(now),
    };
    Ok(encode(header, entry)?)
}

/// The ledger's reserved id.
fn ledger() -> anyhow::Result<glave_event::ThreadId> {
    trust_ledger().context("the trust ledger has no id")
}

#[cfg(test)]
impl Trust {
    /// A new person's trust ledger on a store in memory, with what `home`'s earlier file
    /// held moved in; and the store.
    pub async fn in_memory(home: &Path) -> anyhow::Result<(Data, Self)> {
        let root = glave_keys::InMemoryRoot::new()?;
        let keys = glave_keys::KeySettings::new(std::sync::Arc::new(root));
        let store = Data::Sqlite(glave_store_sqlite::Store::open_in_memory(keys)?);
        let person = PrincipalId::new(Timestamp::now(), random()?)?;
        let trust = Self::open(&store, person, home).await?;
        Ok((store, trust))
    }
}

#[cfg(test)]
mod tests;
