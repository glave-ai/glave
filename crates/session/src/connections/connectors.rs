//! The connectors of glave's catalog through the session API (`connector/*`, `specs/session.md`,
//! "Connectors"): each listed with its accounts and where each is used, and connected, moved and
//! disconnected through the host. A service may have several accounts (`specs/policy.md`,
//! "Connections"): a second takes a name of its own, and no two of a service's accounts are
//! used in one place, so a place's own account takes the place of the one used everywhere and a
//! thread never has two of one service.

use glave_ports::{BlobStore, Caller, EventLog, ModelProvider, ProjectId};

use super::{Connecting, Connections, OfferedAccount};
use crate::accounts::KeptConnector;
use crate::connectors::{Catalog, Connector, Narrowed};
use crate::protocol::{
    ApiError, ConnectorAccount, ConnectorCategory, ConnectorCategoryId, ConnectorInfo,
    ConnectorStatus, ConnectorTag, ConnectorTagId, Reply, Told, code,
};
use crate::server::Server;
use crate::words::Sentence;

/// The longest name an account takes, in characters.
const NAME: usize = 40;

/// What the host offers: each account of a connector connected, and each connector not.
type Offered = [OfferedAccount];

impl<P, L, B> Server<P, L, B>
where
    P: ModelProvider + 'static,
    L: EventLog + 'static,
    B: BlobStore + 'static,
{
    /// Answers `connector/list`: the catalog's connectors the host offers that have what the
    /// caller narrowed by, in its order, and the companies, categories and tags they have;
    /// `used_in` a project of the caller's, only those connected that its threads use.
    pub(crate) async fn list_connectors(
        &self,
        caller: &Caller,
        narrowed: &Narrowed,
        used_in: Option<ProjectId>,
    ) -> Result<Reply, ApiError> {
        let offered = self.connections_for(caller)?.connectors().await?;
        if let Some(project) = used_in {
            self.owned(caller, project).await?;
        }
        let catalog = built_in()?;
        let mut connectors = Vec::new();
        for connector in catalog.connectors.iter().filter(|it| narrowed.has(it)) {
            if let Some(info) = self.described(caller, connector, &offered, used_in).await {
                connectors.push(info);
            }
        }
        let companies = catalog
            .companies
            .iter()
            .filter(|company| connectors.iter().any(|it| it.company == company.id))
            .cloned()
            .collect();
        let categories = ConnectorCategoryId::ALL
            .iter()
            .filter(|id| connectors.iter().any(|it| it.categories.contains(id)))
            .map(|id| ConnectorCategory {
                id: *id,
                name: id.told(),
            })
            .collect();
        let tags = ConnectorTagId::ALL
            .iter()
            .filter(|id| connectors.iter().any(|it| it.tags.contains(id)))
            .map(|id| ConnectorTag {
                id: *id,
                name: id.told(),
            })
            .collect();
        Ok(Reply::Connectors {
            companies,
            categories,
            tags,
            connectors,
        })
    }

    /// `connector` as the caller sees it, with its accounts and where each is used, when the
    /// host offers it; for `used_in` a place, only when an account of it is used there, and
    /// only that account: the place's own, which takes the place of the one used everywhere.
    async fn described(
        &self,
        caller: &Caller,
        connector: &Connector,
        offered: &Offered,
        used_in: Option<ProjectId>,
    ) -> Option<ConnectorInfo> {
        let theirs: Vec<&OfferedAccount> = (offered.iter())
            .filter(|it| it.connector.id == connector.id)
            .collect();
        theirs.first()?;
        let mut connected: Vec<&OfferedAccount> = (theirs.into_iter())
            .filter(|it| it.status != ConnectorStatus::Available)
            .collect();
        let mut replaced = None;
        if let Some(place) = used_in {
            let own = connected.iter().find(|it| it.connector.only == Some(place));
            let everywhere = connected.iter().find(|it| it.connector.only.is_none());
            if own.is_some() && everywhere.is_some() {
                let service = connector.name.clone();
                replaced = Some(Sentence::AccountReplaced { service }.into());
            }
            connected = own.or(everywhere).copied().into_iter().collect();
            connected.first()?;
        }
        let status = match connected.as_slice() {
            [] => ConnectorStatus::Available,
            accounts
                if accounts
                    .iter()
                    .any(|it| it.status == ConnectorStatus::SignInAgain) =>
            {
                ConnectorStatus::SignInAgain
            }
            _ => ConnectorStatus::Connected,
        };
        let mut accounts = Vec::with_capacity(connected.len());
        for OfferedAccount {
            connector: kept,
            status,
            who,
        } in connected
        {
            let signed_in = (who.clone()).map(|who| Sentence::SignedInAs { who }.into());
            accounts.push(ConnectorAccount {
                name: kept.account.clone(),
                only: kept.only,
                used: self.used_where(caller, kept.only).await,
                status: *status,
                who: who.clone(),
                signed_in,
            });
        }
        Some(ConnectorInfo {
            accounts,
            replaced,
            ..info(connector, status)
        })
    }

    /// Answers `connector/connect`, for its threads everywhere or only in a project the
    /// caller owns; `another`, a new account of a service connected already.
    pub(crate) async fn connect_connector(
        &self,
        caller: &Caller,
        connector: KeptConnector,
        another: bool,
    ) -> Result<Reply, ApiError> {
        let joining = if another { Joining::Another } else { Joining::Connect };
        let (connections, found, connector) = self.placeable(caller, connector, joining).await?;
        let another = another && connector.account.is_none();
        match connections.connect(connector, another, self.redirect()).await? {
            Connecting::Connected => Ok(Reply::Connector {
                connector: self.as_now(caller, connections, &found).await?,
            }),
            Connecting::SignIn(sign_in) => Ok(Reply::SignIn { sign_in }),
            Connecting::NoWayBack => Err(self.no_way_back()),
        }
    }

    /// Answers `connector/place`: the connector, its account used where it now is.
    pub(crate) async fn place_connector(
        &self,
        caller: &Caller,
        connector: KeptConnector,
    ) -> Result<Reply, ApiError> {
        let (connections, found, connector) =
            self.placeable(caller, connector, Joining::Place).await?;
        connections.place(connector).await?;
        Ok(Reply::Connector {
            connector: self.as_now(caller, connections, &found).await?,
        })
    }

    /// Answers `connector/disconnect`.
    pub(crate) async fn disconnect_connector(
        &self,
        caller: &Caller,
        connector: KeptConnector,
    ) -> Result<Reply, ApiError> {
        let connector = KeptConnector {
            account: named(connector.account)?,
            ..connector
        };
        self.connections_for(caller)?.disconnect(connector).await?;
        Ok(Reply::Done {})
    }

    /// Answers `connector/rename`: the connector, its account under its new name, which no
    /// other account of the service has, in any case.
    pub(crate) async fn rename_connector(
        &self,
        caller: &Caller,
        connector: KeptConnector,
        name: Option<String>,
    ) -> Result<Reply, ApiError> {
        let connections = self.connections_for(caller)?;
        let found = built_in()?
            .connector(&connector.id)
            .cloned()
            .ok_or_else(|| no_connector(&connector.id))?;
        let connector = KeptConnector {
            account: named(connector.account)?,
            ..connector
        };
        let name = named(name)?;
        let folded = |it: &Option<String>| it.as_ref().map(|name| name.to_lowercase());
        let offered = connections.connectors().await?;
        let taken = (offered.iter())
            .filter(|it| it.status != ConnectorStatus::Available && it.connector.id == found.id)
            .any(|it| {
                !it.connector.same_account(&connector) && folded(&it.connector.account) == folded(&name)
            });
        if taken {
            let said = Sentence::AccountNameTaken {
                service: found.name.clone(),
                name: name.unwrap_or_else(|| found.name.clone()),
            };
            return Err(ApiError::new(code::INVALID_PARAMS, said));
        }
        connections.rename(connector, name).await?;
        Ok(Reply::Connector {
            connector: self.as_now(caller, connections, &found).await?,
        })
    }

    /// The caller's connections, `connector`'s entry in the catalog and the account, its name
    /// tidied, once it is one glave has, its place, if it has one, a project the caller owns,
    /// and no other account of the service is used there. Connecting it, a new account of a
    /// service that has one already takes a name, or is `Another`, named at its sign-in.
    async fn placeable(
        &self,
        caller: &Caller,
        connector: KeptConnector,
        joining: Joining,
    ) -> Result<(&dyn Connections, Connector, KeptConnector), ApiError> {
        let connections = self.connections_for(caller)?;
        let found = built_in()?
            .connector(&connector.id)
            .cloned()
            .ok_or_else(|| no_connector(&connector.id))?;
        if let Some(project) = connector.only {
            self.owned(caller, project).await?;
        }
        let offered = connections.connectors().await?;
        let connected: Vec<&KeptConnector> = (offered.iter())
            .filter(|it| it.status != ConnectorStatus::Available && it.connector.id == found.id)
            .map(|it| &it.connector)
            .collect();
        // A name the service has, in any case, is that account's, as it was spelled.
        let account = named(connector.account)?.map(|name| {
            let had = (connected.iter()).find_map(|kept| {
                kept.account
                    .as_ref()
                    .filter(|it| it.to_lowercase() == name.to_lowercase())
            });
            had.cloned().unwrap_or(name)
        });
        let connector = KeptConnector {
            account,
            ..connector
        };
        // Another account, unnamed, is none of those the service has.
        let unnamed = joining == Joining::Another && connector.account.is_none();
        let (same, others): (Vec<_>, Vec<_>) = (connected.into_iter())
            .partition(|kept| !unnamed && kept.same_account(&connector));
        let service = found.name.clone();
        let connecting = joining != Joining::Place;
        if connecting && !unnamed && same.is_empty() && connector.account.is_none() && !others.is_empty() {
            let said = Sentence::AccountUnnamed { service };
            return Err(ApiError::new(code::INVALID_PARAMS, said));
        }
        if let Some(other) = others.iter().find(|other| other.only == connector.only) {
            let said = match other.only {
                None => Sentence::AccountTakenEverywhere { service },
                Some(project) => match self.owned(caller, project).await {
                    Ok(place) => Sentence::AccountTakenIn {
                        service,
                        place: place.name,
                    },
                    Err(_) => Sentence::AccountTakenGone { service },
                },
            };
            return Err(ApiError::new(code::INVALID_PARAMS, said));
        }
        Ok((connections, found, connector))
    }

    /// `connector` as it is now, after a change.
    async fn as_now(
        &self,
        caller: &Caller,
        connections: &dyn Connections,
        connector: &Connector,
    ) -> Result<ConnectorInfo, ApiError> {
        let offered = connections.connectors().await?;
        let described = self.described(caller, connector, &offered, None).await;
        described.ok_or_else(|| no_connector(&connector.id))
    }

    /// Where a connection the caller's threads use everywhere, or `only` in one project, is
    /// used, in one line: the project by its name, which may be gone.
    async fn used_where(&self, caller: &Caller, only: Option<ProjectId>) -> Told {
        let Some(project) = only else {
            return Sentence::UsedEverywhere.into();
        };
        match self.owned(caller, project).await {
            Ok(found) => Sentence::UsedOnlyIn { place: found.name }.into(),
            Err(_) => Sentence::UsedOnlyInGone.into(),
        }
    }
}

/// What is asked of an account of a connector.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Joining {
    /// Connect it: an account the service has, again, or a new one by its name.
    Connect,
    /// Connect a new account of the service, named at its sign-in when no name is given.
    Another,
    /// Use it somewhere else.
    Place,
}

/// An account's name as given, its ends trimmed: none when it is empty, and refused when it is
/// longer than [`NAME`] or holds a control character.
fn named(account: Option<String>) -> Result<Option<String>, ApiError> {
    let Some(name) = account.map(|it| it.trim().to_owned()) else {
        return Ok(None);
    };
    if name.is_empty() {
        return Ok(None);
    }
    if name.chars().count() > NAME || name.chars().any(char::is_control) {
        let said = Sentence::AccountNameShape { most: NAME };
        return Err(ApiError::new(code::INVALID_PARAMS, said));
    }
    Ok(Some(name))
}

/// The catalog glave was built with: one that does not read is glave's own fault.
fn built_in() -> Result<Catalog, ApiError> {
    Catalog::built_in().map_err(|error| ApiError::internal(&error))
}

/// The connector `connector` as a client sees it, with no account yet.
fn info(connector: &Connector, status: ConnectorStatus) -> ConnectorInfo {
    ConnectorInfo {
        id: connector.id.clone(),
        name: connector.name.clone(),
        company: connector.company.clone(),
        url: connector.url.clone(),
        categories: connector.categories.clone(),
        tags: connector.tags.clone(),
        returns: connector.returns,
        sign_in: connector.sign_in,
        description: connector.told(),
        docs: connector.docs.clone(),
        status,
        accounts: Vec::new(),
        replaced: None,
    }
}

/// glave offers no connector called `connector`.
fn no_connector(connector: &str) -> ApiError {
    ApiError::new(
        code::NO_SERVER,
        Sentence::NoConnector {
            connector: connector.to_owned(),
        },
    )
}
