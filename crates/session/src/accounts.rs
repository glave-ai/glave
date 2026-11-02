//! Where a person's accounts are kept: the accounts ledger in their personal project, glave's
//! own stream under a reserved id, holding their model accounts, git hosts, sign-ins to
//! connections and secrets. Each `provider.connected`, `credential.connected`,
//! `connection.connected` and `secret.added` is sealed under a key of its own; removing an
//! account records its removal and erases it ([`Eraser::erase`]), so neither the store nor a
//! backup can read it again. A removed account whose key still opens, as after a crash between
//! the two, is erased the next time the ledger is read.

use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use glave_event::{
    At, Body, ConnectionConnected, ConnectionRemoved, CredentialConnected, CredentialId,
    CredentialRemoved, EncodeError, EventId, Header, InvalidValue, Principal, PrincipalId,
    ProviderConnected, ProviderId, ProviderRemoved, RawEvent, Secret, SecretAdded, SecretRemoved,
    Seq, ThreadId, decode, encode,
};
use glave_ports::stream::{Reserved, StreamError, append_retrying, catch_up};
use glave_ports::{Clock, Entropy, EntropyError, Erasable, EraseError, Eraser, EventLog, Scope};
use thiserror::Error;
use uuid::Uuid;

/// The renewals remembered, so a client naming a credential since renewed still names it.
const RENAMED: usize = 64;

/// Why the accounts could not be read or kept.
#[derive(Debug, Error)]
pub enum AccountsError {
    /// The ledger could not be read or written.
    #[error("could not read or keep your accounts: {0}")]
    Stream(#[from] StreamError),
    /// A removed account's key could not be destroyed.
    #[error("could not erase a removed account: {0}")]
    Erase(#[from] EraseError),
    #[error(transparent)]
    Entropy(#[from] EntropyError),
    /// An event could not be made: the ledger is full, or a value would not encode.
    #[error("could not record the account: {0}")]
    Event(String),
}

impl From<InvalidValue> for AccountsError {
    fn from(error: InvalidValue) -> Self {
        Self::Event(error.to_string())
    }
}

impl From<EncodeError> for AccountsError {
    fn from(error: EncodeError) -> Self {
        Self::Event(error.to_string())
    }
}

/// A person's accounts, in the store.
#[derive(Clone)]
pub struct AccountLedger<L> {
    log: L,
    eraser: Arc<dyn Eraser>,
    scope: Scope,
    person: PrincipalId,
    clock: Arc<dyn Clock>,
    entropy: Arc<dyn Entropy>,
    /// What it last read, which each read catches up from, shared by its clones: a read takes
    /// in only what was appended since, by this process or another, so it is never stale and
    /// costs one look at the store when nothing changed.
    last: Arc<Mutex<Option<Read>>>,
}

/// The accounts kept, oldest first.
#[derive(Debug, Default, Clone)]
pub struct Accounts {
    pub providers: Vec<ProviderConnected>,
    pub credentials: Vec<CredentialConnected>,
    /// The connections signed in to, one for each server's address.
    pub connections: Vec<ConnectionConnected>,
    /// The secrets the person added, one for each name.
    pub secrets: Vec<SecretAdded>,
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
#[derive(Clone)]
struct Read {
    accounts: Accounts,
    next: u64,
    unerased: Vec<Uuid>,
}

impl<L: EventLog> AccountLedger<L> {
    /// The ledger of `person`, kept in `log` in their personal project `scope`, erased through
    /// `eraser`.
    pub fn new(
        log: L,
        eraser: Arc<dyn Eraser>,
        scope: Scope,
        person: PrincipalId,
        clock: Arc<dyn Clock>,
        entropy: Arc<dyn Entropy>,
    ) -> Self {
        Self {
            log,
            eraser,
            scope,
            person,
            clock,
            entropy,
            last: Arc::default(),
        }
    }

    /// The person whose accounts these are.
    pub fn person(&self) -> PrincipalId {
        self.person
    }

    /// The accounts kept, oldest first, once any removed ones are erased.
    pub async fn accounts(&self) -> Result<Accounts, AccountsError> {
        let read = self.read().await?;
        for account in &read.unerased {
            self.eraser
                .erase(self.scope, Erasable::Account(*account))
                .await?;
        }
        if let Some(last) = self.last().as_mut() {
            last.unerased
                .retain(|account| !read.unerased.contains(account));
        }
        Ok(read.accounts)
    }

    fn last(&self) -> MutexGuard<'_, Option<Read>> {
        self.last.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Keeps the model account `account`, replacing any it stands in for, the same service at
    /// the same URL, which is erased when the accounts are next read.
    pub async fn keep_provider(&self, account: &ProviderConnected) -> Result<(), AccountsError> {
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

    /// Keeps the git host `credential`, replacing any kept for the same host, which is erased
    /// when the accounts are next read.
    pub async fn keep_credential(
        &self,
        credential: &CredentialConnected,
    ) -> Result<(), AccountsError> {
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

    /// Removes the model account `provider`; it is erased when the accounts are next read.
    /// False when no such account is kept.
    pub async fn remove_provider(&self, provider: ProviderId) -> Result<bool, AccountsError> {
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
    ) -> Result<bool, AccountsError> {
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

    /// Removes the git host `credential`, or the one it was renewed into; it is erased when the
    /// accounts are next read. False when no such host is kept.
    pub async fn remove_credential(&self, credential: CredentialId) -> Result<bool, AccountsError> {
        let removed = self
            .append(|accounts| {
                accounts
                    .credential(credential)
                    .map(|kept| (vec![Event::CredentialRemoved(kept.credential, None)], ()))
            })
            .await?;
        Ok(removed.is_some())
    }

    /// Keeps the sign-in `connection`, replacing any kept for the same server, which is erased
    /// when the accounts are next read.
    pub async fn keep_connection(
        &self,
        connection: &ConnectionConnected,
    ) -> Result<(), AccountsError> {
        self.append(|accounts| {
            let mut events = vec![Event::ConnectionConnected(connection.clone())];
            events.extend(
                accounts
                    .connections
                    .iter()
                    .filter(|kept| kept.url == connection.url)
                    .map(|kept| Event::ConnectionRemoved(kept.connection)),
            );
            Some((events, ()))
        })
        .await?;
        Ok(())
    }

    /// Keeps `fresh`, the renewal of the sign-in `old`, in its place under a key of its
    /// own, only while `old` is still kept: false, keeping nothing, when it was removed or
    /// replaced meanwhile.
    pub async fn renew_connection(
        &self,
        old: CredentialId,
        fresh: &ConnectionConnected,
    ) -> Result<bool, AccountsError> {
        let renewed = self
            .append(|accounts| {
                let kept = accounts
                    .connections
                    .iter()
                    .any(|kept| kept.connection == old);
                kept.then(|| {
                    let events = vec![
                        Event::ConnectionConnected(fresh.clone()),
                        Event::ConnectionRemoved(old),
                    ];
                    (events, ())
                })
            })
            .await?;
        Ok(renewed.is_some())
    }

    /// Removes the sign-in at `url`; it is erased when the accounts are next read. False when
    /// none is kept there.
    pub async fn remove_connection(&self, url: &str) -> Result<bool, AccountsError> {
        let removed = self
            .append(|accounts| {
                let events: Vec<Event> = accounts
                    .connections
                    .iter()
                    .filter(|kept| kept.url == url)
                    .map(|kept| Event::ConnectionRemoved(kept.connection))
                    .collect();
                (!events.is_empty()).then_some((events, ()))
            })
            .await?;
        Ok(removed.is_some())
    }

    /// Keeps the secret `name`, sent only to `hosts`, under a key of its own. False, keeping
    /// nothing, when a secret of that name is kept already.
    pub(crate) async fn add_secret(
        &self,
        name: &str,
        hosts: &[String],
        value: &Secret,
    ) -> Result<bool, AccountsError> {
        let secret = self.new_id()?;
        let added = self
            .append(|accounts| {
                let kept = accounts.secrets.iter().any(|kept| kept.name == name);
                (!kept).then(|| {
                    let added = self.secret(secret, name, hosts.to_vec(), value.clone());
                    (vec![Event::SecretAdded(added)], ())
                })
            })
            .await?;
        Ok(added.is_some())
    }

    /// Keeps a new secret in place of the one called `name`, under a key of its own, with
    /// `value`, and `hosts` where given, else the old one's; the old one is erased when the
    /// accounts are next read. False when no secret of that name is kept.
    pub(crate) async fn replace_secret(
        &self,
        name: &str,
        hosts: Option<&[String]>,
        value: &Secret,
    ) -> Result<bool, AccountsError> {
        let secret = self.new_id()?;
        let replaced = self
            .append(|accounts| {
                let old = accounts.secrets.iter().find(|kept| kept.name == name)?;
                let hosts = hosts.map_or_else(|| old.hosts.clone(), <[String]>::to_vec);
                let events = vec![
                    Event::SecretAdded(self.secret(secret, name, hosts, value.clone())),
                    Event::SecretRemoved(old.secret),
                ];
                Some((events, ()))
            })
            .await?;
        Ok(replaced.is_some())
    }

    /// Removes the secret called `name`; it is erased when the accounts are next read. False
    /// when none is kept by that name.
    pub(crate) async fn remove_secret(&self, name: &str) -> Result<bool, AccountsError> {
        let removed = self
            .append(|accounts| {
                let kept = accounts.secrets.iter().find(|kept| kept.name == name)?;
                Some((vec![Event::SecretRemoved(kept.secret)], ()))
            })
            .await?;
        Ok(removed.is_some())
    }

    /// A new secret's event, added by the ledger's person.
    fn secret(
        &self,
        secret: CredentialId,
        name: &str,
        hosts: Vec<String>,
        value: Secret,
    ) -> SecretAdded {
        SecretAdded {
            secret,
            name: name.to_owned(),
            hosts,
            value,
            by: Principal::person(self.person),
        }
    }

    /// A new id for something the ledger keeps under a key of its own.
    fn new_id(&self) -> Result<CredentialId, AccountsError> {
        Ok(CredentialId::new(self.clock.now(), self.entropy.bytes()?)?)
    }

    /// Appends what `change` makes of the accounts kept, read again and tried again when
    /// another process wrote first; nothing when it makes nothing.
    async fn append<T>(
        &self,
        change: impl Fn(&Accounts) -> Option<(Vec<Event>, T)>,
    ) -> Result<Option<T>, AccountsError> {
        let ledger = Reserved::Accounts.id();
        let (this, change) = (self, &change);
        let appended = append_retrying(&self.log, &self.scope, ledger, move || async move {
            let read = this.read().await?;
            let Some((events, made)) = change(&read.accounts) else {
                return Ok::<_, AccountsError>(None);
            };
            let mut raw = Vec::with_capacity(events.len());
            let mut seq = Seq::new(read.next)?;
            for event in events {
                raw.push(this.raw(event, ledger, seq)?);
                seq = seq
                    .next()
                    .ok_or_else(|| AccountsError::Event("the accounts ledger is full".into()))?;
            }
            Ok(Some((raw, made)))
        })
        .await?;
        Ok(appended.map(|(_, made)| made))
    }

    /// Reads the ledger: what it last read, and what was appended since. An account whose key
    /// was destroyed reads as erased and is left out with its removal.
    async fn read(&self) -> Result<Read, AccountsError> {
        let last = self.last().clone();
        let mut read = last.unwrap_or_else(|| Read {
            accounts: Accounts::default(),
            next: 1,
            unerased: Vec::new(),
        });
        let after = match read.next.saturating_sub(1) {
            0 => None,
            seq => Some(Seq::new(seq)?),
        };
        catch_up(
            &self.log,
            &self.scope,
            Reserved::Accounts.id(),
            after,
            |page| {
                for raw in page {
                    read.next = raw.seq.get().saturating_add(1);
                    if let Ok(event) = decode(raw) {
                        read.take(event.body);
                    }
                }
            },
        )
        .await?;
        let mut last = self.last();
        if last.as_ref().is_none_or(|kept| kept.next <= read.next) {
            *last = Some(read.clone());
        }
        drop(last);
        Ok(read)
    }

    /// `event` as the ledger's event at `seq`.
    fn raw(&self, event: Event, ledger: ThreadId, seq: Seq) -> Result<RawEvent, AccountsError> {
        let now = self.clock.now();
        let header = Header {
            id: EventId::new(now, self.entropy.bytes()?)?,
            thread: ledger,
            seq,
            at: At::from(now),
        };
        let by = Principal::person(self.person);
        Ok(match event {
            Event::ProviderConnected(connected) => encode(header, &connected)?,
            Event::ProviderRemoved(provider) => encode(header, &ProviderRemoved { provider, by })?,
            Event::CredentialConnected(connected) => encode(header, &connected)?,
            Event::CredentialRemoved(credential, renewed) => {
                let removed = CredentialRemoved {
                    credential,
                    renewed,
                    by,
                };
                encode(header, &removed)?
            }
            Event::ConnectionConnected(connected) => encode(header, &connected)?,
            Event::ConnectionRemoved(connection) => {
                encode(header, &ConnectionRemoved { connection, by })?
            }
            Event::SecretAdded(added) => encode(header, &added)?,
            Event::SecretRemoved(secret) => encode(header, &SecretRemoved { secret, by })?,
        })
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
            Body::ConnectionConnected(connected) => {
                accounts
                    .connections
                    .retain(|kept| kept.connection != connected.connection);
                accounts.connections.push(connected);
            }
            Body::ConnectionRemoved(removed) => {
                let before = accounts.connections.len();
                accounts
                    .connections
                    .retain(|kept| kept.connection != removed.connection);
                if accounts.connections.len() < before {
                    self.unerased.push(removed.connection.as_uuid());
                }
            }
            Body::SecretAdded(added) => {
                accounts.secrets.retain(|kept| kept.secret != added.secret);
                accounts.secrets.push(added);
            }
            Body::SecretRemoved(removed) => {
                let before = accounts.secrets.len();
                accounts
                    .secrets
                    .retain(|kept| kept.secret != removed.secret);
                if accounts.secrets.len() < before {
                    self.unerased.push(removed.secret.as_uuid());
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
    ConnectionConnected(ConnectionConnected),
    ConnectionRemoved(CredentialId),
    SecretAdded(SecretAdded),
    SecretRemoved(CredentialId),
}

#[cfg(test)]
mod tests;
