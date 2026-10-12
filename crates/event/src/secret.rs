//! A key or token: what a model provider proves who is calling with. It never shows in
//! `Debug`, has no `Display`, and is serialized only where it must travel or be sealed.

use std::fmt;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// A key or token. Its `Debug` never shows it, and it has no `Display`. It serializes as the
/// plain string, so it may go only where it must: sent to the server it is for, or into a
/// payload that is sealed at rest (`provider.connected`).
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

impl Serialize for Secret {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.0)
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
        let json = serde_json::to_string(&secret).unwrap();
        assert_eq!(json, "\"hunter2\"");
        let back: Secret = serde_json::from_str(&json).unwrap();
        assert_eq!(back.expose(), "hunter2");
    }
}
