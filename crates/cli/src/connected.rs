//! The person's connections (`specs/session.md`, "Connections"): remote MCP servers they sign in to
//! in the browser, as MCP's authorization says (`flows.rs`). Each sign-in is kept sealed in
//! the accounts ledger under a key of its own, one for each server's address; its token is
//! renewed before it expires and handed to the person's own servers at that address with each
//! request (`tokens.rs`), never to a file, a command or the model. Their remote servers that
//! ask them to sign in, and are not signed in to, are listed beside the sign-ins, then the
//! connections glave suggests, on or off (`suggested.rs`).

mod flows;
mod suggested;
mod tokens;

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use glave_event::{ConnectionConnected, PrincipalId};
use glave_mcp::address;
use glave_ports::settings::mcp::ServerConfig;
use glave_ports::{BoxFuture, Clock as _, SystemClock};
use glave_session::protocol::{ApiError, ConnectionInfo, ConnectionStatus, SignInLink, code};
use glave_session::{Connections, Returned, Suggestion};
use tokio::sync::Semaphore;
use tokio::task::JoinSet;

use crate::accounts::Ledger;
use crate::credentials::renewing::Renewals;
use crate::data::Data;
use crate::mcp::{Removed, Writer};
use flows::Flows;

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
    /// The connections glave suggests here: none where the person may add none of their own.
    suggested: Vec<Suggestion>,
    /// What changes their `mcp.json` when they turn one on or off.
    writer: Writer,
}

impl Connected {
    /// The connections of `person` in `store`, whose servers `home`'s `mcp.json` names, when
    /// they may sign in to them here: everywhere but a server people sign in to with OIDC
    /// (`others_sign_in`), whose people's own servers it does not run. glave suggests
    /// `suggested` among them.
    pub(crate) async fn for_person(
        store: &Data,
        person: PrincipalId,
        home: &Path,
        others_sign_in: bool,
        suggested: Vec<Suggestion>,
    ) -> anyhow::Result<Option<Arc<Self>>> {
        if others_sign_in {
            return Ok(None);
        }
        let ledger = crate::accounts::open(store, person).await?;
        Ok(Some(Arc::new(Self {
            ledger,
            home: home.to_owned(),
            flows: Flows::default(),
            renewals: Renewals::default(),
            suggested,
            writer: Writer::default(),
        })))
    }

    /// The sign-ins kept: what the ledger last read, and what was appended to it since, by
    /// this process or another on the same store. The renewals of any no longer kept go.
    async fn kept(&self) -> Result<Vec<ConnectionConnected>, ApiError> {
        let read = Instant::now();
        let kept = self.ledger.accounts().await?.connections;
        let still = |signed_in| kept.iter().any(|kept| kept.connection == signed_in);
        self.renewals.only(read, still);
        Ok(kept)
    }

    /// The addresses signed in to: the person's servers there start again when it changes.
    pub(crate) async fn addresses(&self) -> BTreeSet<String> {
        match self.kept().await {
            Ok(kept) => kept.into_iter().map(|kept| kept.url).collect(),
            Err(error) => {
                tracing::warn!("could not read the servers signed in to: {}", error.message);
                BTreeSet::new()
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
    /// one they call that, or at that address.
    async fn resolved(&self, server: &str) -> Option<(String, String)> {
        let remotes = self.remotes().await;
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

/// `connection` as a client sees it, named as `remotes` say. A token that lapsed while it can
/// be renewed still works: the next request renews it.
fn described(
    connection: &ConnectionConnected,
    remotes: &BTreeMap<String, Remote>,
) -> ConnectionInfo {
    let now = SystemClock.now();
    let lapsed = connection
        .expires
        .is_some_and(|expires| expires.timestamp() <= now);
    let status = if lapsed && connection.refresh.is_none() {
        ConnectionStatus::Expired
    } else {
        ConnectionStatus::SignedIn
    };
    ConnectionInfo {
        url: connection.url.clone(),
        name: remotes
            .get(&connection.url)
            .map(|remote| remote.name.clone()),
        status,
        suggested: None,
        usage: None,
    }
}

/// Of the person's remote servers, by address, those that ask them to sign in in a way glave
/// can, as not signed in to, in the order of their addresses: [`ASKERS`] asked at once, each
/// for [`ASKING`] at most. One their settings give a key of their own is left out, as is one
/// that asks no one, or cannot say in time.
async fn signed_out(remotes: impl Iterator<Item = (String, Remote)>) -> Vec<ConnectionInfo> {
    let askers = Arc::new(Semaphore::new(ASKERS));
    let mut asking = JoinSet::new();
    for (url, Remote { name, keyed }) in remotes {
        if keyed {
            continue;
        }
        let askers = Arc::clone(&askers);
        asking.spawn(async move {
            let _asker = askers.acquire().await.ok()?;
            let asks = tokio::time::timeout(ASKING, glave_mcp::offers_sign_in(&url)).await;
            if let Ok(Err(error)) = &asks {
                tracing::debug!("could not tell whether {name} asks for a sign-in: {error}");
            }
            matches!(asks, Ok(Ok(true))).then_some(ConnectionInfo {
                url,
                name: Some(name),
                status: ConnectionStatus::SignedOut,
                suggested: None,
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
        Box::pin(async move {
            let polled = self.polled(flow).await?;
            Ok(polled.map(|connection| self.worded(connection)))
        })
    }

    fn finish(&self, returned: Returned) -> BoxFuture<'_, String> {
        Box::pin(self.finished(returned))
    }

    fn list(&self) -> BoxFuture<'_, Result<Vec<ConnectionInfo>, ApiError>> {
        Box::pin(async move {
            let remotes = self.remotes().await;
            let kept = self.kept().await?;
            let mut listed: Vec<_> = kept
                .iter()
                .map(|kept| self.worded(described(kept, &remotes)))
                .collect();
            let suggestions = self.suggestions(&listed, &remotes);
            // A suggestion's server is never asked anything to list it.
            let unkept = remotes.into_iter().filter(|(url, _)| {
                kept.iter().all(|kept| kept.url != *url) && self.suggested_at(url).is_none()
            });
            listed.extend(signed_out(unkept).await);
            listed.extend(suggestions);
            Ok(listed)
        })
    }

    fn add(&self, url: String) -> BoxFuture<'_, Result<ConnectionInfo, ApiError>> {
        Box::pin(async move { self.added(&url).await })
    }

    fn remove(&self, server: String) -> BoxFuture<'_, Result<(), ApiError>> {
        Box::pin(async move {
            // A sign-in outlives its server's settings, so its address alone names it too, and
            // a suggestion's name its address.
            let resolved = self.resolved(&server).await.map(|(url, _)| url);
            let resolved = resolved.or_else(|| address(&server));
            let url = resolved
                .or_else(|| self.suggestion_named(&server))
                .unwrap_or(server);
            let signed_out = self.ledger.remove_connection(&url).await?;
            // Read again, so its tokens are destroyed now, a renewal of it not kept too.
            self.kept().await?;
            match self.turned_off(&url).await? {
                Some(Removed::Gone) => Ok(()),
                _ if signed_out => Ok(()),
                Some(Removed::Theirs) => Err(ApiError::new(
                    code::NOT_AVAILABLE,
                    "you set this connection up in your own settings, so glave leaves it there: \
                     change it there",
                )),
                Some(Removed::NotThere) => Err(ApiError::new(
                    code::NO_CREDENTIAL,
                    "this connection is off already",
                )),
                None => Err(ApiError::new(
                    code::NO_CREDENTIAL,
                    format!("nothing at {url} is signed in to here"),
                )),
            }
        })
    }
}

#[cfg(test)]
mod tests;
