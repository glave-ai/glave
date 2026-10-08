//! Errors raised while building, encoding and decoding events.

use thiserror::Error;

use crate::id::EventId;

/// A value that breaks one of the format's rules, such as a UUID that is not version 7.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("invalid {what}: {reason}")]
pub struct InvalidValue {
    what: &'static str,
    reason: String,
}

impl InvalidValue {
    pub(crate) fn new(what: &'static str, reason: impl Into<String>) -> Self {
        Self {
            what,
            reason: reason.into(),
        }
    }
}

/// Why a payload could not be encoded into an event.
#[derive(Debug, Error)]
pub enum EncodeError {
    /// The payload type declares a name or version the format does not allow.
    #[error("payload type {kind:?} version {version} breaks the format's rules: {source}")]
    TypeInfo {
        kind: &'static str,
        version: u32,
        source: InvalidValue,
    },
    /// The payload could not be serialized to JSON.
    #[error("could not serialize the {kind} payload: {source}")]
    Serialize {
        kind: &'static str,
        source: serde_json::Error,
    },
}

/// Why stored bytes could not be read as an event.
#[derive(Debug, Error)]
pub enum DecodeError {
    /// The bytes are not a valid envelope.
    #[error("not a valid event envelope: {source}")]
    Envelope { source: serde_json::Error },
    /// The envelope is a version this reader does not know.
    #[error("event {id} uses envelope version {found}; this reader supports version {supported}")]
    UnsupportedEnvelope {
        id: EventId,
        found: u32,
        supported: u32,
    },
    /// The envelope is fine but the payload does not match its type and version.
    #[error("event {id} ({kind} v{version}) has an invalid payload: {source}")]
    Payload {
        id: EventId,
        kind: String,
        version: u32,
        source: serde_json::Error,
    },
}
