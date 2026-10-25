//! The connections glave suggests, among the person's (`specs/session.md`, "Connections"): each
//! off until their `mcp.json` has a server at its address, which turning it on puts there and
//! turning it off takes away, with any sign-in to it. One the person set up there themselves
//! stays when it is turned off. glave asks nothing of a suggestion's server to list it.

use std::collections::BTreeMap;

use glave_mcp::address;
use glave_session::Suggestion;
use glave_session::protocol::{ApiError, ConnectionInfo, ConnectionStatus, code};

use super::{Connected, Remote};
use crate::mcp::Removed;

impl Connected {
    /// The suggestion at `url`, an address as glave keeps them.
    pub(crate) fn suggested_at(&self, url: &str) -> Option<&Suggestion> {
        let url = address(url)?;
        self.suggested
            .iter()
            .find(|suggestion| address(&suggestion.url).as_deref() == Some(url.as_str()))
    }

    /// The address of the suggestion called `name`.
    pub(super) fn suggestion_named(&self, name: &str) -> Option<String> {
        let found = self.suggested.iter().find(|it| it.name == name.trim());
        found.and_then(|suggestion| address(&suggestion.url))
    }

    /// `connection` with what its suggestion is for, when glave suggests it.
    pub(super) fn worded(&self, mut connection: ConnectionInfo) -> ConnectionInfo {
        if let Some(suggestion) = self.suggested_at(&connection.url) {
            connection.suggested = Some(suggestion.suggested());
        }
        connection
    }

    /// The suggestions not in `listed`, signed in to: on when `remotes`, the person's servers
    /// by address, hold one at its address, under the name they give it, else off.
    pub(super) fn suggestions(
        &self,
        listed: &[ConnectionInfo],
        remotes: &BTreeMap<String, Remote>,
    ) -> Vec<ConnectionInfo> {
        let mut found = Vec::new();
        for suggestion in &self.suggested {
            let Some(url) = address(&suggestion.url) else {
                continue;
            };
            if listed.iter().any(|connection| connection.url == url) {
                continue;
            }
            let mut connection = suggestion.listed(ConnectionStatus::Off);
            if let Some(remote) = remotes.get(&url) {
                connection.status = ConnectionStatus::On;
                connection.name = Some(remote.name.clone());
            }
            found.push(connection);
        }
        found
    }

    /// Turns on the suggestion at `url`: into the person's settings, unless a server is there.
    pub(super) async fn added(&self, url: &str) -> Result<ConnectionInfo, ApiError> {
        let suggestion = self.suggested_at(url).ok_or_else(|| {
            ApiError::new(
                code::NO_SERVER,
                format!("glave suggests no connection at {}", url.trim()),
            )
        })?;
        let name = self
            .writer
            .add(&self.home, suggestion.name, &suggestion.url)
            .await?;
        let at = address(&suggestion.url).unwrap_or_default();
        let signed_in = self.kept().await?.iter().any(|kept| kept.url == at);
        let status = if signed_in {
            ConnectionStatus::SignedIn
        } else {
            ConnectionStatus::On
        };
        let mut connection = suggestion.listed(status);
        connection.name = Some(name);
        Ok(connection)
    }

    /// Turns off the suggestion at `url`, out of the person's settings, unless they set it up
    /// there themselves, when glave suggests one there: none when it does not.
    pub(super) async fn turned_off(&self, url: &str) -> Result<Option<Removed>, ApiError> {
        let Some(suggestion) = self.suggested_at(url) else {
            return Ok(None);
        };
        Ok(Some(self.writer.remove(&self.home, &suggestion.url).await?))
    }
}
