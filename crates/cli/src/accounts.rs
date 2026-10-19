//! Where a person's accounts are kept: the accounts ledger in their personal project, glave's
//! own stream under a reserved id, holding their model accounts and git hosts. Each
//! `provider.connected` and `credential.connected` is sealed under a key of its own; removing
//! an account records its removal and destroys that key, then rotates the project key, so
//! neither the store nor a backup can read it again. A removed account whose key still opens,
//! as after a crash between the two, has its key destroyed the next time the ledger is read.

use anyhow::Context as _;
use glave_event::{
    At, Body, CredentialConnected, CredentialId, CredentialRemoved, EventId, Header, Principal,
    PrincipalId, ProviderConnected, ProviderId, ProviderRemoved, RawEvent, Seq, ThreadId, decode,
    encode,
};
use glave_ports::Scope;
use glave_ports::stream::{Reserved, append_retrying, catch_up};
use jiff::Timestamp;
use uuid::Uuid;

use crate::data::{Data, DataLog};

/// The renewals remembered, so a client naming a credential since renewed still names it.
const RENAMED: usize = 64;

/// The person's accounts, in the store.
#[derive(Clone)]
pub struct Ledger {
    store: Data,
    log: DataLog,
    scope: Scope,
    person: PrincipalId,
}

/// The accounts kept, oldest first.
#[derive(Debug, Default)]
pub struct Accounts {
    pub providers: Vec<ProviderConnected>,
    pub credentials: Vec<CredentialConnected>,
    /// Credentials renewed, each with the kept one it became, the latest last.
    renamed: Vec<(CredentialId, CredentialId)>,
}

impl Accounts {
    /// The git host kept as `credential`, or as the one it was renewed into.
    pub fn credential(&self, credential: CredentialId) -> Option<&CredentialConnected> {
        let current = self
            .renamed
            .iter()
            .find(|(old, _)| *old == credential)
            .map_or(credential, |(_, new)| *new);
        self.credentials
            .iter()
            .find(|kept| kept.credential == current)
    }

    /// Takes the removal of `old`: names that named it name what `renewed` it, and `old` names
    /// it too; with nothing renewing it, they name nothing.
    fn renamed(&mut self, old: CredentialId, renewed: Option<CredentialId>) {
        let Some(new) = renewed else {
            self.renamed.retain(|(_, named)| *named != old);
            return;
        };
        for (_, named) in &mut self.renamed {
            if *named == old {
                *named = new;
            }
        }
        self.renamed.push((old, new));
        let over = self.renamed.len().saturating_sub(RENAMED);
        self.renamed.drain(..over);
    }
}

/// The accounts the ledger holds, the `seq` its next event takes, and the accounts removed
/// whose keys still open.
struct Read {
    accounts: Accounts,
    next: u64,
    unerased: Vec<Uuid>,
}

impl Ledger {
    /// The ledger of `person`, in their personal project in `store`.
    pub async fn open(store: &Data, person: PrincipalId) -> anyhow::Result<Self> {
        let scope = crate::projects::personal(store, person).await?;
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

    /// The accounts kept, oldest first, once the keys of any removed ones are destroyed.
    pub async fn accounts(&self) -> anyhow::Result<Accounts> {
        let read = self.read().await?;
        for account in read.unerased {
            self.store.erase_account(&self.scope, account).await?;
        }
        Ok(read.accounts)
    }

    /// Keeps the model account `account`, replacing any it stands in for, the same service at
    /// the same URL, whose key is destroyed when the accounts are next read.
    pub async fn keep_provider(&self, account: &ProviderConnected) -> anyhow::Result<()> {
        self.append(|accounts| {
            let mut events = vec![Event::ProviderConnected(account.clone())];
            events.extend(
                accounts
                    .providers
                    .iter()
                    .filter(|kept| kept.kind == account.kind && kept.url == account.url)
                    .map(|kept| Event::ProviderRemoved(kept.provider)),
            );
            Some((events, ()))
        })
        .await?;
        Ok(())
    }

    /// Keeps the git host `credential`, replacing any kept for the same host, whose token is
    /// destroyed when the accounts are next read.
    pub async fn keep_credential(&self, credential: &CredentialConnected) -> anyhow::Result<()> {
        self.append(|accounts| {
            let mut events = vec![Event::CredentialConnected(credential.clone())];
            events.extend(
                accounts
                    .credentials
                    .iter()
                    .filter(|kept| kept.url == credential.url)
                    .map(|kept| Event::CredentialRemoved(kept.credential, None)),
            );
            Some((events, ()))
        })
        .await?;
        Ok(())
    }

    /// Removes the model account `provider`; its key is destroyed when the accounts are next
    /// read. False when no such account is kept.
    pub async fn remove_provider(&self, provider: ProviderId) -> anyhow::Result<bool> {
        let removed = self
            .append(|accounts| {
                accounts
                    .providers
                    .iter()
                    .any(|kept| kept.provider == provider)
                    .then(|| (vec![Event::ProviderRemoved(provider)], ()))
            })
            .await?;
        Ok(removed.is_some())
    }

    /// Keeps `fresh`, the renewal of the git host `old`, in its place, only while `old` is
    /// still kept: false, keeping nothing, when it was removed or replaced meanwhile.
    pub async fn renew_credential(
        &self,
        old: CredentialId,
        fresh: &CredentialConnected,
    ) -> anyhow::Result<bool> {
        let renewed = self
            .append(|accounts| {
                let kept = accounts
                    .credentials
                    .iter()
                    .any(|kept| kept.credential == old);
                kept.then(|| {
                    let events = vec![
                        Event::CredentialConnected(fresh.clone()),
                        Event::CredentialRemoved(old, Some(fresh.credential)),
                    ];
                    (events, ())
                })
            })
            .await?;
        Ok(renewed.is_some())
    }

    /// Removes the git host `credential`, or the one it was renewed into; its token is
    /// destroyed when the accounts are next read. False when no such host is kept.
    pub async fn remove_credential(&self, credential: CredentialId) -> anyhow::Result<bool> {
        let removed = self
            .append(|accounts| {
                accounts
                    .credential(credential)
                    .map(|kept| (vec![Event::CredentialRemoved(kept.credential, None)], ()))
            })
            .await?;
        Ok(removed.is_some())
    }

    /// Appends what `change` makes of the accounts kept, read again and tried again when
    /// another process wrote first; nothing when it makes nothing.
    async fn append<T>(
        &self,
        change: impl Fn(&Accounts) -> Option<(Vec<Event>, T)>,
    ) -> anyhow::Result<Option<T>> {
        let ledger = Reserved::Accounts.id();
        let (this, change) = (self, &change);
        let appended = append_retrying(&self.log, &self.scope, ledger, move || async move {
            let read = this.read().await?;
            let Some((events, made)) = change(&read.accounts) else {
                return anyhow::Ok(None);
            };
            let mut raw = Vec::with_capacity(events.len());
            for (n, event) in events.into_iter().enumerate() {
                let seq = read.next.saturating_add(u64::try_from(n)?);
                raw.push(event.raw(ledger, seq, Principal::person(this.person))?);
            }
            Ok(Some((raw, made)))
        })
        .await
        .context("could not keep the account")?;
        Ok(appended.map(|(_, made)| made))
    }

    /// Reads the whole ledger. An account whose key was destroyed reads as erased and is left
    /// out with its removal.
    async fn read(&self) -> anyhow::Result<Read> {
        let mut read = Read {
            accounts: Accounts::default(),
            next: 1,
            unerased: Vec::new(),
        };
        catch_up(
            &self.log,
            &self.scope,
            Reserved::Accounts.id(),
            None,
            |page| {
                for raw in page {
                    read.next = raw.seq.get().saturating_add(1);
                    if let Ok(event) = decode(raw) {
                        read.take(event.body);
                    }
                }
            },
        )
        .await
        .context("could not read the accounts")?;
        Ok(read)
    }
}

impl Read {
    /// Takes one ledger event into the accounts.
    fn take(&mut self, body: Body) {
        let accounts = &mut self.accounts;
        match body {
            Body::ProviderConnected(connected) => {
                accounts
                    .providers
                    .retain(|kept| kept.provider != connected.provider);
                accounts.providers.push(connected);
            }
            Body::ProviderRemoved(removed) => {
                let before = accounts.providers.len();
                accounts
                    .providers
                    .retain(|kept| kept.provider != removed.provider);
                if accounts.providers.len() < before {
                    self.unerased.push(removed.provider.as_uuid());
                }
            }
            Body::CredentialConnected(connected) => {
                accounts
                    .credentials
                    .retain(|kept| kept.credential != connected.credential);
                accounts.credentials.push(connected);
            }
            Body::CredentialRemoved(removed) => {
                let before = accounts.credentials.len();
                accounts
                    .credentials
                    .retain(|kept| kept.credential != removed.credential);
                accounts.renamed(removed.credential, removed.renewed);
                if accounts.credentials.len() < before {
                    self.unerased.push(removed.credential.as_uuid());
                }
            }
            _ => {}
        }
    }
}

/// One event to append to the ledger.
enum Event {
    ProviderConnected(ProviderConnected),
    ProviderRemoved(ProviderId),
    CredentialConnected(CredentialConnected),
    /// A git host removed, and the one that renewed it in its place.
    CredentialRemoved(CredentialId, Option<CredentialId>),
}

impl Event {
    fn raw(self, ledger: ThreadId, seq: u64, by: Principal) -> anyhow::Result<RawEvent> {
        let now = Timestamp::now();
        let header = Header {
            id: EventId::new(now, random()?)?,
            thread: ledger,
            seq: Seq::new(seq)?,
            at: At::from(now),
        };
        Ok(match self {
            Self::ProviderConnected(connected) => encode(header, &connected)?,
            Self::ProviderRemoved(provider) => encode(header, &ProviderRemoved { provider, by })?,
            Self::CredentialConnected(connected) => encode(header, &connected)?,
            Self::CredentialRemoved(credential, renewed) => {
                let removed = CredentialRemoved {
                    credential,
                    renewed,
                    by,
                };
                encode(header, &removed)?
            }
        })
    }
}

/// Random bytes for a new id, from the operating system.
pub fn random() -> anyhow::Result<[u8; 10]> {
    let mut bytes = [0u8; 10];
    getrandom::fill(&mut bytes)
        .map_err(|e| anyhow::anyhow!("could not get random bytes for a new id: {e}"))?;
    Ok(bytes)
}

#[cfg(test)]
impl Ledger {
    /// A new person's ledger, on a store in memory.
    pub async fn in_memory() -> anyhow::Result<Self> {
        let root = glave_keys::InMemoryRoot::new()?;
        let keys = glave_keys::KeySettings::new(std::sync::Arc::new(root));
        let store = Data::Sqlite(glave_store_sqlite::Store::open_in_memory(keys)?);
        let person = PrincipalId::new(Timestamp::now(), random()?)?;
        Self::open(&store, person).await
    }
}

#[cfg(test)]
mod tests;
