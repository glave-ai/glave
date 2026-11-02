//! Secrets as clients see them (`secret/*`): each one's name and the hosts its value may reach,
//! never the value.

use serde::{Deserialize, Serialize};

/// A secret the person added.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct SecretInfo {
    /// The variable commands see it as: `STRIPE_KEY`.
    pub name: String,
    /// The hosts its value may be sent to: `api.stripe.com`, or `*.stripe.com` for any host
    /// under it.
    pub hosts: Vec<String>,
}
