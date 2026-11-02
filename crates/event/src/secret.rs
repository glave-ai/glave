//! A key or token: what a model provider proves who is calling with. It never shows in
//! `Debug`, has no `Display`, and is not `Serialize`: a field that must carry it, to the server
//! it is for or into a payload sealed at rest, says so with [`serialize_exposed`].

use std::collections::BTreeMap;
use std::fmt;

use serde::{Deserialize, Deserializer, Serializer};

/// A key or token. Its `Debug` never shows it, and it has no `Display` and no `Serialize`.
#[derive(Clone, PartialEq, Eq)]
pub struct Secret(String);

impl Secret {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    /// The secret itself, to send. Never to log or show.
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Secret([redacted])")
    }
}

/// Serializes `secret` as its plain text, for `#[serde(serialize_with)]` on a field that must
/// carry it: a request to the server it is for, or a payload sealed at rest.
pub fn serialize_exposed<S: Serializer>(secret: &Secret, serializer: S) -> Result<S::Ok, S::Error> {
    serializer.serialize_str(&secret.0)
}

/// [`serialize_exposed`] for a map of them by name, each value in the clear: the variables a
/// command is sent to the computer that runs it with.
pub fn serialize_exposed_values<S: Serializer>(
    secrets: &BTreeMap<String, Secret>,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    serializer.collect_map(secrets.iter().map(|(name, secret)| (name, &secret.0)))
}

/// [`serialize_exposed`] for a field that may hold none, skipped when it does
/// (`skip_serializing_if = "Option::is_none"`).
#[expect(
    clippy::ref_option,
    reason = "serde's serialize_with hands the field over by reference"
)]
pub(crate) fn serialize_exposed_some<S: Serializer>(
    secret: &Option<Secret>,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    match secret {
        Some(secret) => serializer.serialize_some(&secret.0),
        None => serializer.serialize_none(),
    }
}

impl<'de> Deserialize<'de> for Secret {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        String::deserialize(deserializer).map(Self)
    }
}

impl schemars::JsonSchema for Secret {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        "Secret".into()
    }

    fn json_schema(_: &mut schemars::SchemaGenerator) -> schemars::Schema {
        schemars::json_schema!({
            "type": "string",
            "description": "A key or token: sent once, never shown again."
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_secret_never_shows_in_debug_and_travels_as_its_string() {
        let secret = Secret::new("hunter2");
        assert_eq!(format!("{secret:?}"), "Secret([redacted])");
        let json = serialize_exposed(&secret, serde_json::value::Serializer)
            .unwrap()
            .to_string();
        assert_eq!(json, "\"hunter2\"");
        let back: Secret = serde_json::from_str(&json).unwrap();
        assert_eq!(back.expose(), "hunter2");
    }
}
