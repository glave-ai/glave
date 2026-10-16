//! `provider.connected` and `provider.removed`: the model accounts a person connects, so glave
//! reaches their models with their own key (`specs/session.md`, "Model accounts"). They are
//! kept in the person's provider ledger, glave's own stream under a reserved id in their
//! personal space, never in a conversation. A connected account is sealed under a data key of
//! its own, so removing it destroys the key, and the account with it.

use serde::{Deserialize, Serialize};
use serde_json::value::RawValue;
use uuid::Uuid;

use crate::codec::{Payload, parse};
use crate::envelope::RawEvent;
use crate::id::{CredentialId, ProviderId};
use crate::payload::credential;
use crate::payload::principal::Principal;
use crate::secret::Secret;

pub(crate) const CONNECTED: &str = "provider.connected";
pub(crate) const REMOVED: &str = "provider.removed";

/// Which service a model account is with.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ProviderKind {
    Openai,
    Anthropic,
    Openrouter,
    /// Any other server that speaks OpenAI's API, at a URL the person gives.
    Compatible,
}

/// A model account was connected: the person's key for a service, checked before it was kept.
/// Sealed under the account's own key.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProviderConnected {
    pub provider: ProviderId,
    pub kind: ProviderKind,
    /// The server's API, for a `compatible` account; the service's own otherwise.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(serialize_with = "crate::secret::serialize_exposed")]
    pub key: Secret,
    /// The models the account offered when it was connected, the default first.
    pub models: Vec<String>,
    /// Who connected it.
    pub by: Principal,
}

/// A model account was removed: its key was destroyed with it. Content-free.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProviderRemoved {
    pub provider: ProviderId,
    /// Who removed it.
    pub by: Principal,
}

impl Payload for ProviderConnected {
    const TYPE: &'static str = CONNECTED;
    const VERSION: u32 = 1;
}

impl Payload for ProviderRemoved {
    const TYPE: &'static str = REMOVED;
    const VERSION: u32 = 1;
}

pub(crate) fn decode_connected(
    version: u32,
    data: &RawValue,
) -> Option<serde_json::Result<ProviderConnected>> {
    (version == 1).then(|| parse(data))
}

pub(crate) fn decode_removed(
    version: u32,
    data: &RawValue,
) -> Option<serde_json::Result<ProviderRemoved>> {
    (version == 1).then(|| parse(data))
}

/// The account whose own data key seals `event`, when it is a `provider.connected` (a model
/// account) or a `credential.connected` (a git host); every other event is sealed under its
/// month's key.
pub fn sealed_under_account(event: &RawEvent) -> Option<Uuid> {
    #[derive(Deserialize)]
    struct Provider {
        provider: ProviderId,
    }
    #[derive(Deserialize)]
    struct Credential {
        credential: CredentialId,
    }
    let data = event.data.get();
    match event.kind.as_str() {
        CONNECTED => serde_json::from_str::<Provider>(data)
            .ok()
            .map(|named| named.provider.as_uuid()),
        credential::CONNECTED => serde_json::from_str::<Credential>(data)
            .ok()
            .map(|named| named.credential.as_uuid()),
        _ => None,
    }
}
