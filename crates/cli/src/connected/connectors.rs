//! The connectors of glave's catalog (`specs/session.md`, "Connectors"): connecting one records
//! it in the person's accounts ledger, after they sign in to its server when it asks them to,
//! that sign-in kept beside it; disconnecting it records that and removes the sign-in,
//! destroying its token. Each is used everywhere, or only in one project of the person's, which
//! they change at once. A policy narrows which are offered.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::PoisonError;

use glave_mcp::address;
use glave_session::protocol::{ApiError, ConnectionStatus, ConnectorSignIn, ConnectorStatus, code};
use glave_session::{Catalog, Connecting, Connector, KeptConnector};

use super::{Connected, Remote, described};
use crate::mcp::Offered;

impl Connected {
    /// Offers the connectors `offered` names from now on, as the policy read again says.
    pub(crate) fn offer(&self, offered: Offered) {
        *self.offered.lock().unwrap_or_else(PoisonError::into_inner) = offered;
    }

    /// Whether the connector `id` is offered now.
    fn offers(&self, id: &str) -> bool {
        let offered = self.offered.lock().unwrap_or_else(PoisonError::into_inner);
        offered.allows(id)
    }

    /// The connectors offered here, each with whether it is connected, and where it is used:
    /// one whose sign-in no longer works, or whose server asked for more, is to sign in to
    /// again.
    pub(super) async fn listed_connectors(
        &self,
    ) -> Result<Vec<(KeptConnector, ConnectorStatus)>, ApiError> {
        let catalog = catalog()?;
        let accounts = self.accounts().await?;
        let none: BTreeMap<String, Remote> = BTreeMap::new();
        let offered = catalog.connectors.iter().filter(|it| self.offers(&it.id));
        Ok(offered
            .map(|connector| {
                let kept = accounts.connector(&connector.id);
                let connected = kept.is_some();
                let url = address(&connector.url).unwrap_or_default();
                let sign_in = accounts.connections.iter().find(|it| it.url == url);
                let status = match (connected, connector.sign_in, sign_in) {
                    (false, _, _) => ConnectorStatus::Available,
                    (true, ConnectorSignIn::None, _) => ConnectorStatus::Connected,
                    (true, ConnectorSignIn::Dcr, Some(sign_in)) => {
                        let more = self.more.of(&url).is_some();
                        match described(sign_in, &none, more).status {
                            ConnectionStatus::SignedIn => ConnectorStatus::Connected,
                            _ => ConnectorStatus::SignInAgain,
                        }
                    }
                    (true, ConnectorSignIn::Dcr, None) => ConnectorStatus::SignInAgain,
                };
                let kept = kept.cloned().unwrap_or_else(|| KeptConnector {
                    id: connector.id.clone(),
                    only: None,
                });
                (kept, status)
            })
            .collect())
    }

    /// Connects `kept`, to be used where it says: at once when it asks no one to sign in, else
    /// by signing in, the browser to come back to `redirect`.
    pub(super) async fn connecting(
        &self,
        kept: KeptConnector,
        redirect: Option<String>,
    ) -> Result<Connecting, ApiError> {
        let catalog = catalog()?;
        let id = &kept.id;
        let connector = catalog
            .connector(id)
            .filter(|_| self.offers(id))
            .ok_or_else(|| unknown(id))?;
        match connector.sign_in {
            ConnectorSignIn::None => {
                self.ledger.keep_connector(&kept, None).await?;
                Ok(Connecting::Connected)
            }
            ConnectorSignIn::Dcr => {
                let Some(redirect) = redirect else {
                    return Ok(Connecting::NoWayBack);
                };
                let url = at(connector)?;
                let name = connector.name.clone();
                let link = self.sign_in(url, name, redirect, Some(kept)).await?;
                Ok(Connecting::SignIn(link))
            }
        }
    }

    /// Uses the connected connector `kept.id` where `kept` says from now on: the person's
    /// servers are reconciled at the next turn, and threads elsewhere go without it.
    pub(super) async fn placing(&self, kept: KeptConnector) -> Result<(), ApiError> {
        if !self.ledger.place_connector(&kept).await? {
            return Err(not_connected(&kept.id));
        }
        Ok(())
    }

    /// Disconnects the connector `id`, removing its sign-in, so its token is destroyed now.
    pub(super) async fn disconnecting(&self, id: String) -> Result<(), ApiError> {
        let catalog = catalog()?;
        // One the catalog no longer has is disconnected all the same; a sign-in a server of
        // the person's own settings uses stays theirs.
        let url = catalog.connector(&id).and_then(|it| address(&it.url));
        let theirs = match &url {
            Some(url) => self.remotes().await.contains_key(url),
            None => false,
        };
        let url = url.filter(|_| !theirs).unwrap_or_default();
        if !self.ledger.remove_connector(&id, &url).await? {
            return Err(not_connected(&id));
        }
        self.more.forget(&url);
        self.kept().await?;
        Ok(())
    }

    /// The addresses of the connectors connected.
    pub(super) async fn connector_addresses(&self) -> BTreeSet<String> {
        let (Ok(catalog), Ok(accounts)) = (catalog(), self.accounts().await) else {
            return BTreeSet::new();
        };
        catalog
            .connectors
            .iter()
            .filter(|it| accounts.connector(&it.id).is_some())
            .filter_map(|it| address(&it.url))
            .collect()
    }
}

/// The catalog glave was built with.
pub(crate) fn catalog() -> Result<Catalog, ApiError> {
    Catalog::built_in().map_err(|error| ApiError::internal(&error))
}

/// `connector`'s address, as glave keeps addresses.
fn at(connector: &Connector) -> Result<String, ApiError> {
    address(&connector.url).ok_or_else(|| unknown(&connector.id))
}

/// The connector `id` is not connected.
fn not_connected(id: &str) -> ApiError {
    ApiError::new(code::NO_CREDENTIAL, format!("{id} is not connected here"))
}

/// The connector `id` is not offered here, or not in the catalog.
fn unknown(id: &str) -> ApiError {
    ApiError::new(
        code::NO_SERVER,
        format!("{id} is not one of the apps you can connect here"),
    )
}
