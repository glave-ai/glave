//! An environment: the recipe a computer is made from, however it was written down
//! (`.glave/environment.toml` or a devcontainer.json).

use std::collections::BTreeMap;
use std::fmt;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// The recipe a computer is made from.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Environment {
    /// What the computer runs: the host itself, or an OCI image.
    pub image: Image,
    /// Shell commands run once, in order, when the computer is made.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub setup: Vec<String>,
    #[serde(default)]
    pub size: Size,
    /// How commands may reach the network; `None` leaves it to policy.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub network: Option<Network>,
    /// The hosts commands may reach when the network is an allowlist.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub allow: Vec<String>,
    /// Names of secrets commands may ask for; values come through the credential helper.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub secrets: Vec<String>,
    /// Minutes without work before the computer sleeps; `None` leaves it to policy.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub idle_minutes: Option<u32>,
    /// Variables every command sees. Never secrets: those are named in `secrets`.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub env: BTreeMap<String, String>,
}

impl Environment {
    /// The machine glave runs on, as it is: the default on a laptop.
    pub fn host() -> Self {
        Self::default()
    }
}

/// Whether `name` can be an environment variable's name: a letter or `_`, then letters,
/// digits and `_`.
pub fn is_variable_name(name: &str) -> bool {
    let mut chars = name.chars();
    chars
        .next()
        .is_some_and(|first| first.is_ascii_alphabetic() || first == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// What a computer runs.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum Image {
    /// The machine the computer is on, as it is.
    #[default]
    Host,
    /// An OCI image reference, such as `ghcr.io/acme/rust:1.99`.
    Oci(String),
}

impl fmt::Display for Image {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Host => f.write_str("host"),
            Self::Oci(reference) => f.write_str(reference),
        }
    }
}

impl Image {
    /// `host` names the machine itself; anything else is an image reference. Empty text is
    /// not an image.
    pub fn named(name: &str) -> Option<Self> {
        match name.trim() {
            "" => None,
            "host" => Some(Self::Host),
            reference => Some(Self::Oci(reference.to_owned())),
        }
    }
}

impl Serialize for Image {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for Image {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let name = String::deserialize(deserializer)?;
        Self::named(&name).ok_or_else(|| serde::de::Error::custom("the image is empty"))
    }
}

/// How big a computer is; `None` leaves each to policy.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Size {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cpus: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub memory_gb: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub disk_gb: Option<u32>,
}

/// How commands may reach the network.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Network {
    Off,
    /// Only the hosts the environment allows.
    Allowlist,
    On,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn environments_round_trip_as_json() {
        let mut environment = Environment {
            image: Image::Oci("ghcr.io/acme/rust:1.99".into()),
            setup: vec!["scripts/setup.sh".into()],
            network: Some(Network::Allowlist),
            allow: vec!["crates.io".into()],
            ..Environment::host()
        };
        environment.size.cpus = Some(2);
        environment.env.insert("RUST_LOG".into(), "info".into());
        let json = serde_json::to_string(&environment).unwrap();
        assert!(
            json.contains(r#""image":"ghcr.io/acme/rust:1.99""#),
            "{json}"
        );
        let back: Environment = serde_json::from_str(&json).unwrap();
        assert_eq!(back, environment);
        let host: Environment = serde_json::from_str(r#"{"image":"host"}"#).unwrap();
        assert_eq!(host, Environment::host());
        assert!(serde_json::from_str::<Environment>(r#"{"image":" "}"#).is_err());
    }
}
