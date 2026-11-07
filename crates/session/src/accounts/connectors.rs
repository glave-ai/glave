//! The accounts of glave's connectors in the ledger (`specs/policy.md`, "Connections"): each
//! connected account of a service, where its threads use it, its sign-in kept beside it under
//! its address and name, no two of a service's accounts in one place, and renaming one.

use glave_event::{AccountRenamed, ConnectionConnected, ConnectorConnected, Principal};
use glave_ports::{EventLog, ProjectId};

use super::{AccountLedger, Accounts, AccountsError, Event, replaced};

/// An account of a connector of glave's catalog the person connected, and where their threads
/// use it (`specs/policy.md`, "Connections").
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct KeptConnector {
    /// Its id in the catalog.
    pub id: String,
    /// The person's name for this account of it, `Acme`: none for one never named. Each of a
    /// service's accounts has a name of its own.
    pub account: Option<String>,
    /// The one project whose threads alone use it, their Chats among them: none where every
    /// thread of theirs does.
    pub only: Option<ProjectId>,
}

/// How keeping an account of a connector where it says went.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Placement {
    /// Kept there.
    Placed,
    /// It is not connected: nothing changed.
    NotConnected,
    /// Another account of the service is used there: nothing changed.
    Taken,
    /// Another account of the service has the name asked for: nothing changed.
    NameTaken,
}

impl KeptConnector {
    /// Whether this is the account `other` names: the same connector, the same name.
    pub fn same_account(&self, other: &Self) -> bool {
        self.id == other.id && self.account == other.account
    }

    /// Its `connector.connected`, by `by`: what it was called when `renamed`.
    pub(super) fn connected(
        self,
        renamed: Option<AccountRenamed>,
        by: Principal,
    ) -> ConnectorConnected {
        ConnectorConnected {
            connector: self.id,
            account: self.account,
            only: self.only.map(ProjectId::as_uuid),
            renamed,
            by,
        }
    }
}

impl Accounts {
    /// The account of a connector `connector` names, when it is connected, wherever it is used.
    pub(crate) fn connector(&self, connector: &KeptConnector) -> Option<&KeptConnector> {
        self.connectors
            .iter()
            .find(|kept| kept.same_account(connector))
    }

    /// Another account of `connector`'s service used where `connector` says, which no two
    /// accounts of one service are (`specs/policy.md`, "Connections").
    fn taken(&self, connector: &KeptConnector) -> bool {
        self.connectors.iter().any(|kept| {
            kept.id == connector.id
                && kept.account != connector.account
                && kept.only == connector.only
        })
    }

    /// Takes `connected` in: one used elsewhere now, or renamed, keeps its place among the
    /// connected.
    pub(super) fn placed(&mut self, connected: ConnectorConnected) {
        let kept = KeptConnector {
            id: connected.connector,
            account: connected.account,
            only: connected.only.map(ProjectId::from_uuid),
        };
        let was = match connected.renamed {
            Some(renamed) => renamed.from,
            None => kept.account.clone(),
        };
        let had = (self.connectors.iter_mut()).find(|it| it.id == kept.id && it.account == was);
        match had {
            Some(had) => *had = kept,
            None => self.connectors.push(kept),
        }
    }

    /// The accounts of the connector `id` connected, in the order connected.
    pub fn accounts_of<'a>(&'a self, id: &'a str) -> impl Iterator<Item = &'a KeptConnector> {
        self.connectors.iter().filter(move |kept| kept.id == id)
    }
}

impl<L: EventLog> AccountLedger<L> {
    /// Records `connector`, an account of a connector of glave's catalog, as connected and used
    /// where it says, unless it is already, there; and keeps `sign_in`, the person's sign-in to
    /// its server as that account, when it asks for one, replacing any kept for the same server
    /// and account; nothing, `Taken`, while another account of the service is used there.
    pub async fn keep_connector(
        &self,
        connector: &KeptConnector,
        sign_in: Option<&ConnectionConnected>,
    ) -> Result<Placement, AccountsError> {
        let kept = self
            .append(|accounts| {
                if accounts.taken(connector) {
                    return Some((Vec::new(), Placement::Taken));
                }
                let mut events = Vec::new();
                if accounts.connector(connector) != Some(connector) {
                    events.push(Event::ConnectorConnected(connector.clone()));
                }
                if let Some(sign_in) = sign_in {
                    events.push(Event::ConnectionConnected(sign_in.clone()));
                    events.extend(replaced(accounts, sign_in));
                }
                Some((events, Placement::Placed))
            })
            .await?;
        Ok(kept.unwrap_or(Placement::Placed))
    }

    /// Uses the account of a connector `connector` names where it says from now on, while it is
    /// connected and no other account of the service is used there.
    pub async fn place_connector(
        &self,
        connector: &KeptConnector,
    ) -> Result<Placement, AccountsError> {
        let placed = self
            .append(|accounts| {
                let Some(kept) = accounts.connector(connector) else {
                    return Some((Vec::new(), Placement::NotConnected));
                };
                if accounts.taken(connector) {
                    return Some((Vec::new(), Placement::Taken));
                }
                let moved =
                    (kept != connector).then(|| Event::ConnectorConnected(connector.clone()));
                Some((moved.into_iter().collect(), Placement::Placed))
            })
            .await?;
        Ok(placed.unwrap_or(Placement::NotConnected))
    }

    /// Records the account of a connector `connector` names as disconnected and removes its
    /// sign-in to its server at `url`, which is erased when the accounts are next read. False
    /// when it was not connected.
    pub async fn remove_connector(
        &self,
        connector: &KeptConnector,
        url: &str,
    ) -> Result<bool, AccountsError> {
        let removed = self
            .append(|accounts| {
                accounts.connector(connector)?;
                let mut events = vec![Event::ConnectorDisconnected(connector.clone())];
                events.extend(
                    accounts
                        .connections
                        .iter()
                        .filter(|kept| kept.url == url && kept.account == connector.account)
                        .map(|kept| Event::ConnectionRemoved(kept.connection)),
                );
                Some((events, ()))
            })
            .await?;
        Ok(removed.is_some())
    }

    /// Calls the account `connector` names `name` from now on (none: unnamed), where it is
    /// used, its sign-in to its server at `url` kept under its new name; with `shared`, the
    /// person's own server at that address keeps the sign-in it shared too. `NameTaken` when
    /// another account of the service has that name, and `NotConnected` when it is not.
    pub async fn rename_connector(
        &self,
        connector: &KeptConnector,
        name: Option<&str>,
        url: &str,
        shared: bool,
    ) -> Result<Placement, AccountsError> {
        let fresh = self.new_id()?;
        let renamed = self
            .append(|accounts| {
                let Some(kept) = accounts.connector(connector) else {
                    return Some((Vec::new(), Placement::NotConnected));
                };
                let new = KeptConnector {
                    account: name.map(str::to_owned),
                    ..kept.clone()
                };
                if new.account == kept.account {
                    return Some((Vec::new(), Placement::Placed));
                }
                if accounts.connector(&new).is_some() {
                    return Some((Vec::new(), Placement::NameTaken));
                }
                let mut events = vec![Event::ConnectorRenamed(new.clone(), kept.account.clone())];
                if let Some(sign_in) = accounts.sign_in(url, kept.account.as_deref()) {
                    let moved = ConnectionConnected {
                        connection: fresh,
                        account: new.account.clone(),
                        ..sign_in.clone()
                    };
                    events.push(Event::ConnectionConnected(moved));
                    if !(shared && kept.account.is_none()) {
                        events.push(Event::ConnectionRemoved(sign_in.connection));
                    }
                }
                Some((events, Placement::Placed))
            })
            .await?;
        Ok(renamed.unwrap_or(Placement::NotConnected))
    }
}
