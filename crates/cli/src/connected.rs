//! The person's connections (`specs/session.md`, "Connections"): remote MCP servers they sign in to
//! in the browser, as MCP's authorization says (`flows.rs`). Each sign-in is kept sealed in
//! the accounts ledger under a key of its own, one for each server's address; its token is
//! renewed before it expires and handed to the person's own servers at that address with each
//! request (`tokens.rs`), never to a file, a command or the model.

mod flows;
mod tokens;

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use glave_event::{ConnectionConnected, PrincipalId};
use glave_mcp::address;
use glave_ports::settings::mcp::ServerConfig;
use glave_ports::{BoxFuture, Clock as _, SystemClock};
use glave_session::protocol::{ApiError, ConnectionInfo, ConnectionStatus, SignInLink, code};
use glave_session::{Connections, Returned};

use crate::accounts::Ledger;
use crate::credentials::renewing::Renewals;
use crate::data::Data;
use flows::Flows;

/// The person's connections.
pub(crate) struct Connected {
    ledger: Ledger,
    /// glave's folder, whose `mcp.json` names the person's servers.
    home: PathBuf,
    flows: Flows,
    renewals: Renewals<ConnectionConnected>,
}

impl Connected {
    /// The connections of `person` in `store`, whose servers `home`'s `mcp.json` names, when
    /// they may sign in to them here: everywhere but a server people sign in to with OIDC
    /// (`others_sign_in`), whose people's own servers it does not run.
    pub(crate) async fn for_person(
        store: &Data,
        person: PrincipalId,
        home: &Path,
        others_sign_in: bool,
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

    /// What the person's settings call each of their remote servers, by address.
    async fn names(&self) -> BTreeMap<String, String> {
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
                ServerConfig::Http { url, .. } => Some((address(&url)?, name)),
                ServerConfig::Stdio { .. } => None,
            })
            .collect()
    }

    /// The address of the person's remote server `server` and what their settings call it: the
    /// one they call that, or at that address.
    async fn resolved(&self, server: &str) -> Option<(String, String)> {
        let names = self.names().await;
        let at = address(server).and_then(|url| names.get_key_value(&url));
        let at = at.map(|(url, name)| (url.clone(), name.clone()));
        at.or_else(|| names.into_iter().find(|(_, name)| name == server.trim()))
    }
}

/// `connection` as a client sees it, named as `names` say. A token that lapsed while it can be
/// renewed still works: the next request renews it.
fn described(connection: &ConnectionConnected, names: &BTreeMap<String, String>) -> ConnectionInfo {
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
        name: names.get(&connection.url).cloned(),
        status,
    }
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
            let names = self.names().await;
            let kept = self.kept().await?;
            Ok(kept.iter().map(|kept| described(kept, &names)).collect())
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
            // Read again, so its tokens are destroyed now, a renewal of it not kept too.
            self.kept().await?;
            Ok(())
        })
    }
}

#[cfg(test)]
mod tests;
