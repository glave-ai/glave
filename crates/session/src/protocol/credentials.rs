//! Git hosts as clients see them (`credential/*`): which host, whom it said the token is
//! for, and a hint to tell tokens apart, never the token. And the code a person enters to
//! sign in to GitHub in the browser.

use glave_event::{CredentialId, Forge};
use serde::{Deserialize, Serialize};

/// A git host the person connected.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct CredentialInfo {
    pub id: CredentialId,
    pub forge: Forge,
    /// Where the host is: `https://github.com`.
    pub url: String,
    /// Whom the host said the token is for, when it said.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub login: Option<String>,
    /// The token's last four characters, to tell tokens apart; absent for a token too short
    /// to show any of.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hint: Option<String>,
}

/// Signing in to GitHub in the browser (its device flow): the person opens `url` and enters
/// `code`, then the client waits on `flow` with `credential/device/poll`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct DeviceCode {
    /// Names this sign-in to `credential/device/poll`.
    pub flow: String,
    /// What the person enters on GitHub's page, such as `WDJB-MJHT`.
    pub code: String,
    /// GitHub's page where the code is entered.
    pub url: String,
    /// Seconds until the code expires.
    pub expires_in: u64,
}
