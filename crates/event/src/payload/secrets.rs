//! `secret.added` and `secret.removed`: the secrets a person adds for the agent's work, such as
//! an API key, by name (`specs/model.md`, "Secrets"). Only the person adds one, from a client;
//! commands see its value in its variable. They are kept in the person's accounts ledger, each
//! sealed under a data key of its own, so removing one destroys its key and the value with it.

use serde::{Deserialize, Serialize};
use serde_json::value::RawValue;

use crate::codec::{Body, Payload, parse};
use crate::id::CredentialId;
use crate::payload::principal::Principal;
use crate::secret::Secret;

pub(crate) const ADDED: &str = "secret.added";
pub(crate) const REMOVED: &str = "secret.removed";

/// A secret was added, or replaced: a new one in its place. Sealed under the secret's own key.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SecretAdded {
    pub secret: CredentialId,
    /// The variable commands see it as: `STRIPE_KEY`.
    pub name: String,
    #[serde(serialize_with = "crate::secret::serialize_exposed")]
    pub value: Secret,
    /// Who added it.
    pub by: Principal,
}

/// A secret was removed, or replaced: its value was destroyed with it. Content-free.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SecretRemoved {
    pub secret: CredentialId,
    /// Who removed it.
    pub by: Principal,
}

impl Payload for SecretAdded {
    const TYPE: &'static str = ADDED;
    const VERSION: u32 = 1;
}

impl Payload for SecretRemoved {
    const TYPE: &'static str = REMOVED;
    const VERSION: u32 = 1;
}

/// Decodes `secret.added` and `secret.removed`; `None` for any other type or version.
pub(crate) fn decode(
    kind: &str,
    version: u32,
    data: &RawValue,
) -> Option<serde_json::Result<Body>> {
    if version != 1 {
        return None;
    }
    Some(match kind {
        ADDED => parse(data).map(Body::SecretAdded),
        REMOVED => parse(data).map(Body::SecretRemoved),
        _ => return None,
    })
}
