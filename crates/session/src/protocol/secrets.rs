//! Secrets as clients see them (`secret/*`): each one's name, never its value.

use serde::{Deserialize, Serialize};

/// A secret the person added.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct SecretInfo {
    /// The variable commands see it as: `STRIPE_KEY`.
    pub name: String,
}
