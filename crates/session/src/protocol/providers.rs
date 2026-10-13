//! Model accounts as clients see them (`provider/connect`, `provider/list`, `provider/remove`):
//! which service, and a hint to tell keys apart, never the key.

use glave_event::{ProviderId, ProviderKind};
use serde::{Deserialize, Serialize};

/// A model account the person connected.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct ProviderInfo {
    pub id: ProviderId,
    pub kind: ProviderKind,
    /// The server's API, for a `compatible` account.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    /// The key's last four characters, to tell keys apart; absent for a key too short to show
    /// any of.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hint: Option<String>,
    /// The models it offers, the one glave uses by default first.
    pub models: Vec<String>,
}

/// The shortest key whose last four characters are shown.
const SHOWN_FROM: usize = 16;

/// The last four characters of `key`, when it is long enough that they give nothing away.
pub fn hint(key: &str) -> Option<String> {
    let count = key.chars().count();
    (count >= SHOWN_FROM).then(|| key.chars().skip(count - 4).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_hint_is_the_last_four_characters_of_a_long_key() {
        assert_eq!(
            hint("sk-proj-0123456789abcdefa1b2").as_deref(),
            Some("a1b2")
        );
        assert_eq!(hint("short-key"), None);
        assert_eq!(hint(""), None);
    }
}
