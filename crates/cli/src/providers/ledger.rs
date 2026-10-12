//! Where model accounts are kept: the provider ledger in the person's personal space, glave's
//! own stream under a reserved id. Each `provider.connected` is sealed under a key of its own;
//! removing an account records `provider.removed` and destroys that key, then rotates the
//! space key, so neither the store nor a backup can read the account again.

use anyhow::Context as _;
use glave_event::{
    At, Body, EventId, Header, Principal, PrincipalId, ProviderConnected, ProviderId,
    ProviderRemoved, RawEvent, Seq, ThreadId, decode, encode,
};
use glave_ports::{EventLog, LogError, Scope};
use glave_session::provider_ledger;
use jiff::Timestamp;

use crate::data::{Data, DataLog};

/// Ledger events read per page.
const PAGE: usize = 500;
/// Appends tried before giving up on a ledger another process keeps moving.
const ATTEMPTS: usize = 8;

/// The person's model accounts, in the store.
#[derive(Clone)]
pub struct Ledger {
    store: Data,
    log: DataLog,
    scope: Scope,
    person: PrincipalId,
}

/// The accounts the ledger holds, oldest first, and the `seq` its next event takes.
struct Read {
    accounts: Vec<ProviderConnected>,
    next: u64,
}

impl Ledger {
    /// The ledger of `person`, in their personal space in `store`.
    pub async fn open(store: &Data, person: PrincipalId) -> anyhow::Result<Self> {
        let scope = crate::spaces::personal(store, person).await?;
        Ok(Self {
            store: store.clone(),
            log: store.log(),
            scope,
            person,
        })
    }

    /// The person whose accounts these are.
    pub fn person(&self) -> PrincipalId {
        self.person
    }

    /// The accounts kept, oldest first.
    pub async fn accounts(&self) -> anyhow::Result<Vec<ProviderConnected>> {
        Ok(self.read().await?.accounts)
    }

    /// Keeps `account`, replacing any account it stands in for, the same service at the same
    /// URL, whose key is then destroyed.
    pub async fn keep(&self, account: &ProviderConnected) -> anyhow::Result<()> {
        let replaced: Vec<ProviderId> = self
            .append(|accounts| {
                let replaced: Vec<ProviderId> = accounts
                    .iter()
                    .filter(|kept| kept.kind == account.kind && kept.url == account.url)
                    .map(|kept| kept.provider)
                    .collect();
                let mut events = vec![Event::Connected(account.clone())];
                events.extend(replaced.iter().map(|id| Event::Removed(*id)));
                Some((events, replaced))
            })
            .await?
            .unwrap_or_default();
        for id in replaced {
            self.store.erase_provider(&self.scope, id).await?;
        }
        Ok(())
    }

    /// Removes the account `provider`, destroying its key. False when no such account is kept.
    pub async fn remove(&self, provider: ProviderId) -> anyhow::Result<bool> {
        let removed = self
            .append(|accounts| {
                accounts
                    .iter()
                    .any(|kept| kept.provider == provider)
                    .then(|| (vec![Event::Removed(provider)], ()))
            })
            .await?;
        if removed.is_some() {
            self.store.erase_provider(&self.scope, provider).await?;
        }
        Ok(removed.is_some())
    }

    /// Appends what `change` makes of the accounts kept, read again and tried again when
    /// another process wrote first; nothing when it makes nothing.
    async fn append<T>(
        &self,
        change: impl Fn(&[ProviderConnected]) -> Option<(Vec<Event>, T)>,
    ) -> anyhow::Result<Option<T>> {
        let ledger = ledger()?;
        for _ in 0..ATTEMPTS {
            let read = self.read().await?;
            let Some((events, made)) = change(&read.accounts) else {
                return Ok(None);
            };
            let mut raw = Vec::with_capacity(events.len());
            for (n, event) in events.into_iter().enumerate() {
                let seq = read.next.saturating_add(u64::try_from(n)?);
                raw.push(event.raw(ledger, seq, Principal::person(self.person))?);
            }
            match self.log.append(&self.scope, &raw).await {
                Ok(()) => return Ok(Some(made)),
                Err(LogError::Conflict { .. }) => {}
                Err(error) => return Err(error).context("could not keep the model account"),
            }
        }
        anyhow::bail!("the model accounts kept changing while glave wrote to them: try again")
    }

    /// Reads the whole ledger. An account whose key was destroyed reads as erased and is left
    /// out with its removal.
    async fn read(&self) -> anyhow::Result<Read> {
        let ledger = ledger()?;
        let mut accounts: Vec<ProviderConnected> = Vec::new();
        let (mut after, mut next) = (None, 1);
        loop {
            let page = self
                .log
                .read(&self.scope, ledger, after, PAGE)
                .await
                .context("could not read the model accounts")?;
            for raw in &page {
                next = raw.seq.get().saturating_add(1);
                match decode(raw.clone()).map(|event| event.body) {
                    Ok(Body::ProviderConnected(connected)) => {
                        accounts.retain(|kept| kept.provider != connected.provider);
                        accounts.push(connected);
                    }
                    Ok(Body::ProviderRemoved(removed)) => {
                        accounts.retain(|kept| kept.provider != removed.provider);
                    }
                    _ => {}
                }
            }
            if page.len() < PAGE {
                return Ok(Read { accounts, next });
            }
            after = page.last().map(|event| event.seq);
        }
    }
}

/// One event to append to the ledger.
enum Event {
    Connected(ProviderConnected),
    Removed(ProviderId),
}

impl Event {
    fn raw(self, ledger: ThreadId, seq: u64, by: Principal) -> anyhow::Result<RawEvent> {
        let now = Timestamp::now();
        let header = Header {
            id: EventId::new(now, random())?,
            thread: ledger,
            seq: Seq::new(seq)?,
            at: At::from(now),
        };
        Ok(match self {
            Self::Connected(connected) => encode(header, &connected)?,
            Self::Removed(provider) => encode(header, &ProviderRemoved { provider, by })?,
        })
    }
}

/// The ledger's reserved id.
fn ledger() -> anyhow::Result<ThreadId> {
    provider_ledger().context("the provider ledger has no id")
}

/// Random bytes for a new id.
pub fn random() -> [u8; 10] {
    let mut bytes = [0u8; 10];
    let _ = getrandom::fill(&mut bytes);
    bytes
}
