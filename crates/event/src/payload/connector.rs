//! `connector.connected` and `connector.disconnected`: the connectors of glave's catalog a
//! person connected (`specs/session.md`, "Connectors"), kept in their accounts ledger. A
//! connector that asks for a sign-in has its token kept beside it, as a `connection.connected`
//! at its address; disconnecting it records `connector.disconnected` and removes that sign-in,
//! destroying its token. Both are content-free: the connector is named by its id in the catalog.

use serde::{Deserialize, Serialize};
use serde_json::value::RawValue;

use crate::codec::{Payload, parse};
use crate::payload::principal::Principal;

pub(crate) const CONNECTED: &str = "connector.connected";
pub(crate) const DISCONNECTED: &str = "connector.disconnected";

/// A connector of glave's catalog was connected.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConnectorConnected {
    /// Its id in the catalog: `linear`.
    pub connector: String,
    /// Who connected it.
    pub by: Principal,
}

/// A connector of glave's catalog was disconnected.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConnectorDisconnected {
    /// Its id in the catalog.
    pub connector: String,
    /// Who disconnected it.
    pub by: Principal,
}

impl Payload for ConnectorConnected {
    const TYPE: &'static str = CONNECTED;
    const VERSION: u32 = 1;
}

impl Payload for ConnectorDisconnected {
    const TYPE: &'static str = DISCONNECTED;
    const VERSION: u32 = 1;
}

pub(crate) fn decode_connected(
    version: u32,
    data: &RawValue,
) -> Option<serde_json::Result<ConnectorConnected>> {
    (version == 1).then(|| parse(data))
}

pub(crate) fn decode_disconnected(
    version: u32,
    data: &RawValue,
) -> Option<serde_json::Result<ConnectorDisconnected>> {
    (version == 1).then(|| parse(data))
}
