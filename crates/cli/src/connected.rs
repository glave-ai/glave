//! The person's connections (`specs/session.md`, "Connections"): remote MCP servers they sign in to
//! in the browser, as MCP's authorization says (`flows.rs`). Each sign-in is kept sealed in
//! the accounts ledger under a key of its own, one for each server's address and account (a
//! connector's accounts the person named); its token is renewed before it expires and handed to
//! the person's own servers at that address, as that account, with each request (`tokens.rs`),
//! never to a file, a command or the model. Their remote servers that
//! ask them to sign in, and are not signed in to, are listed beside the sign-ins. Web search is
//! a connection too, under the hood, which the person turns on and off as a setting
//! (`search.rs`), never listed among them. The connectors of glave's catalog are connected
//! here too, a record in the accounts ledger beside their sign-ins (`connectors.rs`).

mod connectors;
mod flows;
mod search;
mod tokens;

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use glave_event::{ConnectionConnected, PrincipalId};
use glave_mcp::{Reachable, address};
use glave_ports::settings::mcp::ServerConfig;
use glave_ports::{BoxFuture, Clock as _, SystemClock};
use glave_session::protocol::{
    ApiError, ConnectionInfo, ConnectionStatus, ConnectorStatus, SignInLink, code,
};
use glave_session::{
    Accounts, Connecting, Connections, KeptConnector, OfferedAccount, Returned, Searching,
};
use tokio::sync::Semaphore;
use tokio::task::JoinSet;

use crate::accounts::Ledger;
use crate::credentials::renewing::Renewals;
use crate::data::Data;
use crate::mcp::{Offered, Writer};
use flows::Flows;
pub(crate) use search::WebSearch;

/// The longest one of the person's remote servers is given to say whether it asks them to
/// sign in, as `connection/list` lists it.
const ASKING: Duration = Duration::from_secs(5);
/// How many of them are asked at once.
const ASKERS: usize = 8;

/// The person's connections.
pub(crate) struct Connected {
    ledger: Ledger,
    /// glave's folder, whose `mcp.json` names the person's servers.
    home: PathBuf,
    flows: Flows,
    renewals: Renewals<ConnectionConnected>,
    /// Web search, the person's to turn on: none where they may add no servers of their own.
    search: Option<WebSearch>,
    /// What changes their `mcp.json` when they turn it on or off.
    writer: Writer,
    /// What their servers asked for that their sign-ins there do not grant.
    more: MoreAsked,
    /// The addresses their remote servers may be at.
    reachable: Reachable,
    /// The connectors of glave's catalog they may connect, as the policy last read says.
    offered: Mutex<Offered>,
}

/// What the person signed in to and connected: their servers start again when it changes.
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash)]
pub(crate) struct Signed {
    /// The addresses signed in to, each with the account of a connector signed in to there
    /// as, when the person named one.
    pub(crate) addresses: BTreeSet<(String, Option<String>)>,
    /// The connectors of glave's catalog connected, and where each is used, in the order
    /// connected.
    pub(crate) connectors: Vec<KeptConnector>,
}

/// The most scopes kept for what one server asked for.
const MORE_ASKED: usize = 32;

/// What each server, by its address and the account signed in to there, asked for with `403
/// insufficient_scope` since the person last signed in to it as that account, as MCP's step-up
/// authorization says: the scopes a sign-in there asks for next, beside those it grants. One
/// entry for each of the person's remote servers and accounts at most.
#[derive(Clone, Default)]
pub(super) struct MoreAsked(Arc<Mutex<BTreeMap<SignedAs, Vec<String>>>>);

/// A server's address, and the account of a connector signed in to there as, when named.
type SignedAs = (String, Option<String>);

impl MoreAsked {
    fn lock(&self) -> std::sync::MutexGuard<'_, BTreeMap<SignedAs, Vec<String>>> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The server at `url` asked `account` for `scope`, which the sign-in there lacks: added to
    /// what it asked for before, `MORE_ASKED` scopes at most.
    pub(super) fn ask(&self, url: &str, account: Option<&str>, scope: &str) {
        let mut asked = self.lock();
        let scopes = asked.entry(signed_as(url, account)).or_default();
        for scope in scope.split_whitespace() {
            if scopes.len() < MORE_ASKED && !scopes.iter().any(|kept| kept == scope) {
                scopes.push(scope.to_owned());
            }
        }
    }

    /// What the server at `url` asked for since the person last signed in to it as `account`.
    pub(super) fn of(&self, url: &str, account: Option<&str>) -> Option<Vec<String>> {
        self.lock().get(&signed_as(url, account)).cloned()
    }

    /// The person signed in to the server at `url` as `account` again, or removed that sign-in:
    /// what it asked for is asked for no more.
    pub(super) fn forget(&self, url: &str, account: Option<&str>) {
        self.lock().remove(&signed_as(url, account));
    }
}

/// The server at `url` as `account`.
fn signed_as(url: &str, account: Option<&str>) -> SignedAs {
    (url.to_owned(), account.map(str::to_owned))
}

impl Connected {
    /// The connections of `person` in `store`, whose servers `home`'s `mcp.json` names, where
    /// they may sign in to them here (everywhere but a server people sign in to with OIDC,
    /// whose people's own servers it does not run); with `search`, the person's to turn on;
    /// asking only at addresses `reachable` allows; offering the connectors of glave's catalog
    /// `offered` names.
    pub(crate) async fn for_person(
        store: &Data,
        person: PrincipalId,
        home: &Path,
        search: Option<WebSearch>,
        reachable: Reachable,
        offered: Offered,
    ) -> anyhow::Result<Arc<Self>> {
        let ledger = crate::accounts::open(store, person).await?;
        Ok(Arc::new(Self {
            ledger,
            home: home.to_owned(),
            flows: Flows::default(),
            renewals: Renewals::default(),
            search,
            writer: Writer::default(),
            more: MoreAsked::default(),
            reachable,
            offered: Mutex::new(offered),
        }))
    }

    /// The accounts kept: what the ledger last read, and what was appended to it since, by
    /// this process or another on the same store. The renewals of sign-ins no longer kept go.
    async fn accounts(&self) -> Result<Accounts, ApiError> {
        let read = Instant::now();
        let accounts = self.ledger.accounts().await?;
        let kept = &accounts.connections;
        let still = |signed_in| kept.iter().any(|kept| kept.connection == signed_in);
        self.renewals.only(read, still);
        Ok(accounts)
    }

    /// The sign-ins kept, as [`Self::accounts`] reads them.
    async fn kept(&self) -> Result<Vec<ConnectionConnected>, ApiError> {
        Ok(self.accounts().await?.connections)
    }

    /// The addresses signed in to and the connectors connected: the person's servers start
    /// again when it changes.
    pub(crate) async fn signed(&self) -> Signed {
        match self.accounts().await {
            Ok(accounts) => Signed {
                addresses: (accounts.connections.into_iter())
                    .map(|it| (it.url, it.account))
                    .collect(),
                connectors: accounts.connectors,
            },
            Err(error) => {
                tracing::warn!("could not read the servers signed in to: {}", error.message);
                Signed::default()
            }
        }
    }

    /// The person's remote servers, by address, as their settings give them.
    async fn remotes(&self) -> BTreeMap<String, Remote> {
        let file = self.home.join(crate::mcp::FILE);
        let text = tokio::fs::read_to_string(file).await.unwrap_or_default();
        if text.trim().is_empty() {
            return BTreeMap::new();
        }
        let configs = glave_ports::settings::mcp::parse(&text, &crate::mcp::environment());
        configs
            .unwrap_or_else(|error| {
                tracing::warn!("could not read your MCP servers to sign in to them: {error}");
                BTreeMap::new()
            })
            .into_iter()
            .filter_map(|(name, config)| match config {
                ServerConfig::Http { url, headers } => {
                    let keyed = headers
                        .keys()
                        .any(|it| it.eq_ignore_ascii_case("authorization"));
                    Some((address(&url)?, Remote { name, keyed }))
                }
                ServerConfig::Stdio { .. } => None,
            })
            .collect()
    }

    /// The address of the person's remote server `server` and what their settings call it: the
    /// one they call that, or at that address. Web search's is none: no connection of theirs.
    async fn resolved(&self, server: &str) -> Option<(String, String)> {
        let mut remotes = self.remotes().await;
        if let Some(url) = self.search_address() {
            remotes.remove(&url);
        }
        let at = address(server).and_then(|url| remotes.get_key_value(&url));
        let at = at.map(|(url, remote)| (url.clone(), remote.name.clone()));
        at.or_else(|| {
            let mut remotes = remotes.into_iter();
            remotes
                .find(|(_, remote)| remote.name == server.trim())
                .map(|(url, remote)| (url, remote.name))
        })
    }
}

/// One of the person's remote servers, as their settings give it.
struct Remote {
    /// What their settings call it.
    name: String,
    /// Whether their settings give it an `Authorization` header: a key of their own, which a
    /// sign-in's token would take the place of.
    keyed: bool,
}

/// `connection` as a client sees it, named as `remotes` say, its server having asked for
/// more than it grants when `more`. A token that lapsed while it can be renewed still works:
/// the next request renews it.
fn described(
    connection: &ConnectionConnected,
    remotes: &BTreeMap<String, Remote>,
    more: bool,
) -> ConnectionInfo {
    let now = SystemClock.now();
    let lapsed = connection
        .expires
        .is_some_and(|expires| expires.timestamp() <= now);
    let status = if lapsed && connection.refresh.is_none() {
        ConnectionStatus::Expired
    } else if more {
        ConnectionStatus::AsksMore
    } else {
        ConnectionStatus::SignedIn
    };
    ConnectionInfo {
        url: connection.url.clone(),
        name: remotes
            .get(&connection.url)
            .map(|remote| remote.name.clone()),
        status,
        usage: None,
    }
}

/// Of the person's remote servers, by address, those that ask them to sign in in a way glave
/// can, as not signed in to, in the order of their addresses: [`ASKERS`] asked at once, each
/// for [`ASKING`] at most. One their settings give a key of their own is left out, as is one
/// that asks no one, or cannot say in time.
async fn signed_out(
    remotes: impl Iterator<Item = (String, Remote)>,
    reachable: Reachable,
) -> Vec<ConnectionInfo> {
    let askers = Arc::new(Semaphore::new(ASKERS));
    let mut asking = JoinSet::new();
    for (url, Remote { name, keyed }) in remotes {
        if keyed {
            continue;
        }
        let askers = Arc::clone(&askers);
        asking.spawn(async move {
            let _asker = askers.acquire().await.ok()?;
            let asks =
                tokio::time::timeout(ASKING, glave_mcp::offers_sign_in(&url, reachable)).await;
            if let Ok(Err(error)) = &asks {
                tracing::debug!("could not tell whether {name} asks for a sign-in: {error}");
            }
            matches!(asks, Ok(Ok(true))).then_some(ConnectionInfo {
                url,
                name: Some(name),
                status: ConnectionStatus::SignedOut,
                usage: None,
            })
        });
    }
    let mut listed = Vec::new();
    while let Some(asked) = asking.join_next().await {
        listed.extend(asked.ok().flatten());
    }
    listed.sort_by(|one, other| one.url.cmp(&other.url));
    listed
}

impl Connections for Connected {
    fn start(
        &self,
        server: String,
        redirect: Option<String>,
    ) -> BoxFuture<'_, Result<Option<SignInLink>, ApiError>> {
        Box::pin(self.started(server, redirect))
    }

    fn poll(&self, flow: String) -> BoxFuture<'_, Result<Option<ConnectionInfo>, ApiError>> {
        Box::pin(self.polled(flow))
    }

    fn finish(&self, returned: Returned) -> BoxFuture<'_, String> {
        Box::pin(self.finished(returned))
    }

    fn list(&self) -> BoxFuture<'_, Result<Vec<ConnectionInfo>, ApiError>> {
        Box::pin(async move {
            let mut remotes = self.remotes().await;
            let mut kept = self.kept().await?;
            // Web search is a setting of the person's, not a connection they see; its server
            // is never asked anything to list it.
            if let Some(url) = self.search_address() {
                remotes.remove(&url);
                kept.retain(|kept| kept.url != url);
            }
            // A connector's sign-in is listed with the connector (`connector/list`), unless the
            // person's settings name a server of their own at its address; a named account's
            // is always the connector's.
            let connectors = self.connector_addresses().await;
            kept.retain(|kept| {
                kept.account.is_none()
                    && (remotes.contains_key(&kept.url) || !connectors.contains(&kept.url))
            });
            let mut listed: Vec<_> = kept
                .iter()
                .map(|kept| described(kept, &remotes, self.more.of(&kept.url, None).is_some()))
                .collect();
            let unkept = remotes
                .into_iter()
                .filter(|(url, _)| kept.iter().all(|kept| kept.url != *url));
            listed.extend(signed_out(unkept, self.reachable).await);
            Ok(listed)
        })
    }

    fn remove(&self, server: String) -> BoxFuture<'_, Result<(), ApiError>> {
        Box::pin(async move {
            // A sign-in outlives its server's settings, so its address alone names it too.
            let resolved = self.resolved(&server).await.map(|(url, _)| url);
            let url = resolved.or_else(|| address(&server)).unwrap_or(server);
            if !self.ledger.remove_connection(&url).await? {
                return Err(ApiError::new(
                    code::NO_CREDENTIAL,
                    format!("nothing at {url} is signed in to here"),
                ));
            }
            self.more.forget(&url, None);
            // Read again, so its tokens are destroyed now, a renewal of it not kept too.
            self.kept().await?;
            Ok(())
        })
    }

    fn search(&self) -> BoxFuture<'_, Result<Option<Searching>, ApiError>> {
        Box::pin(self.searching())
    }

    fn set_search(&self, on: bool) -> BoxFuture<'_, Result<Option<Searching>, ApiError>> {
        Box::pin(self.searching_set(on))
    }

    fn connectors(&self) -> BoxFuture<'_, Result<Vec<OfferedAccount>, ApiError>> {
        Box::pin(self.listed_connectors())
    }

    fn connect(
        &self,
        connector: KeptConnector,
        another: bool,
        redirect: Option<String>,
    ) -> BoxFuture<'_, Result<Connecting, ApiError>> {
        Box::pin(self.connecting(connector, another, redirect))
    }

    fn place(&self, connector: KeptConnector) -> BoxFuture<'_, Result<(), ApiError>> {
        Box::pin(self.placing(connector))
    }

    fn disconnect(&self, connector: KeptConnector) -> BoxFuture<'_, Result<(), ApiError>> {
        Box::pin(self.disconnecting(connector))
    }

    fn rename(
        &self,
        connector: KeptConnector,
        name: Option<String>,
    ) -> BoxFuture<'_, Result<(), ApiError>> {
        Box::pin(self.renaming(connector, name))
    }
}

#[cfg(test)]
mod tests;
