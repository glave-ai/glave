//! The connectors of glave's catalog (`specs/session.md`, "Connectors"): connecting one records
//! it in the person's accounts ledger, after they sign in to its server when it asks them to,
//! that sign-in kept beside it; disconnecting it records that and removes the sign-in,
//! destroying its token. Each is used everywhere, or only in one project of the person's, which
//! they change at once. A second account of one, named, has a sign-in of its own. A policy
//! narrows which are offered.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::PoisonError;

use glave_mcp::address;
use glave_session::protocol::{ApiError, ConnectionStatus, ConnectorSignIn, ConnectorStatus, code};
use glave_session::{Accounts, Catalog, Connecting, Connector, KeptConnector, OfferedAccount, Placement};

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

    /// The connectors offered here: each account of one connected, with where it is used and
    /// whether its sign-in works (one that no longer does, or whose server asked for more, is
    /// to sign in to again), and each one not connected once.
    pub(super) async fn listed_connectors(
        &self,
    ) -> Result<Vec<OfferedAccount>, ApiError> {
        let catalog = catalog()?;
        let accounts = self.accounts().await?;
        let none: BTreeMap<String, Remote> = BTreeMap::new();
        let offered = catalog.connectors.iter().filter(|it| self.offers(&it.id));
        let status = |connector: &Connector, kept: &KeptConnector| {
            let url = address(&connector.url).unwrap_or_default();
            let sign_in = accounts.sign_in(&url, kept.account.as_deref());
            match (connector.sign_in, sign_in) {
                (ConnectorSignIn::None, _) => ConnectorStatus::Connected,
                (ConnectorSignIn::Dcr, Some(sign_in)) => {
                    let more = self.more.of(&url, kept.account.as_deref()).is_some();
                    match described(sign_in, &none, more).status {
                        ConnectionStatus::SignedIn => ConnectorStatus::Connected,
                        _ => ConnectorStatus::SignInAgain,
                    }
                }
                (ConnectorSignIn::Dcr, None) => ConnectorStatus::SignInAgain,
            }
        };
        let mut listed = Vec::new();
        for connector in offered {
            let before = listed.len();
            let url = address(&connector.url).unwrap_or_default();
            for kept in accounts.accounts_of(&connector.id) {
                let who = accounts.sign_in(&url, kept.account.as_deref());
                listed.push(OfferedAccount {
                    who: who.and_then(|it| it.who.clone()),
                    ..OfferedAccount::new(kept.clone(), status(connector, kept))
                });
            }
            if listed.len() == before {
                let none = KeptConnector {
                    id: connector.id.clone(),
                    account: None,
                    only: None,
                };
                listed.push(OfferedAccount::new(none, ConnectorStatus::Available));
            }
        }
        Ok(listed)
    }

    /// Connects `kept`, to be used where it says: at once when it asks no one to sign in, else
    /// by signing in, the browser to come back to `redirect`. `another`, it is a new account of
    /// its service, unnamed, which takes its name as it is kept ([`named_new`]).
    pub(super) async fn connecting(
        &self,
        kept: KeptConnector,
        another: bool,
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
                let kept = match another {
                    true => named_new(&self.accounts().await?, kept, None, &connector.name),
                    false => kept,
                };
                kept_there(self.ledger.keep_connector(&kept, None).await?, &kept)?;
                Ok(Connecting::Connected)
            }
            ConnectorSignIn::Dcr => {
                let Some(redirect) = redirect else {
                    return Ok(Connecting::NoWayBack);
                };
                let url = at(connector)?;
                // A server of the person's own settings at its address is used in its stead,
                // so another account of it would never run.
                if (kept.account.is_some() || another) && self.remotes().await.contains_key(&url) {
                    let said = format!(
                        "your own settings name a server at {}'s address, which glave uses in \
                         its place: remove it there to connect another account",
                        connector.name
                    );
                    return Err(ApiError::new(code::INVALID_PARAMS, said));
                }
                let name = match &kept.account {
                    Some(account) => format!("{} ({account})", connector.name),
                    None => connector.name.clone(),
                };
                let link = self.sign_in(url, name, redirect, Some(kept), another).await?;
                Ok(Connecting::SignIn(link))
            }
        }
    }

    /// Uses the connected account `kept` names where it says from now on: the person's
    /// servers are reconciled at the next turn, and threads elsewhere go without it.
    pub(super) async fn placing(&self, kept: KeptConnector) -> Result<(), ApiError> {
        kept_there(self.ledger.place_connector(&kept).await?, &kept)
    }

    /// Calls the account `kept` names `name` from now on, its sign-in, and what its server
    /// asked for more, kept under the new name; the person's servers are reconciled at the next
    /// turn. Never named where a server of the person's own settings is at its address, which
    /// is used in its stead and shares its sign-in.
    pub(super) async fn renaming(
        &self,
        kept: KeptConnector,
        name: Option<String>,
    ) -> Result<(), ApiError> {
        let catalog = catalog()?;
        let url = catalog.connector(&kept.id).and_then(|it| address(&it.url));
        let url = url.unwrap_or_default();
        if name.is_some() && self.remotes().await.contains_key(&url) {
            let said = format!(
                "your own settings name a server at {}'s address, which glave uses in its place: \
                 remove it there to name this account",
                kept.id
            );
            return Err(ApiError::new(code::INVALID_PARAMS, said));
        }
        let renamed = self.ledger.rename_connector(&kept, name.as_deref(), &url);
        kept_there(renamed.await?, &kept)?;
        if let Some(asked) = self.more.of(&url, kept.account.as_deref()) {
            self.more.ask(&url, name.as_deref(), &asked.join(" "));
        }
        self.more.forget(&url, kept.account.as_deref());
        self.kept().await?;
        Ok(())
    }

    /// Disconnects the account `kept` names, removing its sign-in, so its token is destroyed
    /// now.
    pub(super) async fn disconnecting(&self, kept: KeptConnector) -> Result<(), ApiError> {
        let catalog = catalog()?;
        // One the catalog no longer has is disconnected all the same; a sign-in a server of
        // the person's own settings uses stays theirs, and is never a named account's.
        let url = catalog.connector(&kept.id).and_then(|it| address(&it.url));
        let theirs = match &url {
            Some(url) if kept.account.is_none() => self.remotes().await.contains_key(url),
            _ => false,
        };
        let url = url.filter(|_| !theirs).unwrap_or_default();
        if !self.ledger.remove_connector(&kept, &url).await? {
            return Err(not_connected(&kept));
        }
        self.more.forget(&url, kept.account.as_deref());
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
            .filter(|it| accounts.accounts_of(&it.id).next().is_some())
            .filter_map(|it| address(&it.url))
            .collect()
    }
}

/// A new account of `kept`'s service, which the person left unnamed, named as `who` it is
/// signed in as, where the service said, else as `service` and a number (`Notion 2`): unique
/// among the service's accounts in any case, a number added when another has the name, and a
/// name's length at most. One the person named keeps its name, as does the service's first,
/// unnamed, when nothing says who it is.
pub(super) fn named_new(
    accounts: &Accounts,
    kept: KeptConnector,
    who: Option<&str>,
    service: &str,
) -> KeptConnector {
    let first_of_its = accounts.accounts_of(&kept.id).next().is_none();
    if kept.account.is_some() || (first_of_its && who.is_none()) {
        return kept;
    }
    let taken: BTreeSet<String> = (accounts.accounts_of(&kept.id))
        .filter_map(|it| it.account.as_ref())
        .map(|name| name.to_lowercase())
        .collect();
    let (base, first) = match who.map(str::trim).filter(|it| !it.is_empty()) {
        Some(who) => (who, 1),
        None => (service, 2),
    };
    // Each number one more than the last, until one is free: no more than there are accounts.
    let named = (first..=first + taken.len()).map(|n| {
        let number = if n == 1 { String::new() } else { format!(" {n}") };
        let room = NAME.saturating_sub(number.chars().count());
        let cut: String = base.chars().take(room).collect();
        format!("{}{number}", cut.trim_end())
    });
    let account = named.into_iter().find(|name| !taken.contains(&name.to_lowercase()));
    KeptConnector { account, ..kept }
}

/// The longest name an account takes, in characters, as the session API holds names.
const NAME: usize = 40;

/// The catalog glave was built with.
pub(crate) fn catalog() -> Result<Catalog, ApiError> {
    Catalog::built_in().map_err(|error| ApiError::internal(&error))
}

/// `connector`'s address, as glave keeps addresses.
fn at(connector: &Connector) -> Result<String, ApiError> {
    address(&connector.url).ok_or_else(|| unknown(&connector.id))
}

/// Whether `kept` was kept where it says, as `placement` says: refused, in plain words, when it
/// is not connected or another account of its service is used there.
pub(super) fn kept_there(placement: Placement, kept: &KeptConnector) -> Result<(), ApiError> {
    match placement {
        Placement::Placed => Ok(()),
        Placement::NotConnected => Err(not_connected(kept)),
        Placement::Taken => Err(ApiError::new(
            code::INVALID_PARAMS,
            format!(
                "another {} account is used there already: use this one somewhere else, or move \
                 that one first",
                kept.id
            ),
        )),
        Placement::NameTaken => Err(ApiError::new(
            code::INVALID_PARAMS,
            format!(
                "another {} account has that name already: give this one another",
                kept.id
            ),
        )),
    }
}

/// The account `kept` names is not connected.
fn not_connected(kept: &KeptConnector) -> ApiError {
    let said = match &kept.account {
        Some(account) => format!(
            "no {} account called {account:?} is connected here",
            kept.id
        ),
        None => format!("{} is not connected here", kept.id),
    };
    ApiError::new(code::NO_CREDENTIAL, said)
}

/// The connector `id` is not offered here, or not in the catalog.
fn unknown(id: &str) -> ApiError {
    ApiError::new(
        code::NO_SERVER,
        format!("{id} is not one of the apps you can connect here"),
    )
}
