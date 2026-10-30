//! What a request may carry besides its params: its `_meta`.

use serde::{Deserialize, Serialize};

/// What a request may carry besides its params, in `params._meta`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct Meta {
    /// For `turn/start`: the same key for the same thread returns the turn already started
    /// instead of starting another, so a client may retry safely.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub idempotency_key: Option<IdempotencyKey>,
    /// For `turn/start`: the W3C trace the turn belongs to (`00-<trace id>-<parent id>-<flags>`),
    /// recorded with the turn and joined by glave's spans. One that is not one is ignored.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub traceparent: Option<String>,
}

impl Meta {
    pub(crate) fn is_empty(&self) -> bool {
        self.idempotency_key.is_none() && self.traceparent.is_none()
    }
}

/// A client's name for one attempt at a turn: 1 to 255 bytes of text.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(try_from = "String", into = "String")]
pub struct IdempotencyKey(String);

impl IdempotencyKey {
    /// The longest key, in bytes.
    pub const MAX: usize = 255;

    pub fn new(key: impl Into<String>) -> Result<Self, String> {
        let key = key.into();
        if key.is_empty() || key.len() > Self::MAX {
            return Err(format!(
                "an idempotency key is 1 to {} bytes, got {}",
                Self::MAX,
                key.len()
            ));
        }
        Ok(Self(key))
    }
}

impl TryFrom<String> for IdempotencyKey {
    type Error = String;

    fn try_from(key: String) -> Result<Self, Self::Error> {
        Self::new(key)
    }
}

impl From<IdempotencyKey> for String {
    fn from(key: IdempotencyKey) -> Self {
        key.0
    }
}
