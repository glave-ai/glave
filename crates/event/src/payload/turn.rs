//! `turn.started` and `turn.ended`: the bounds of one turn.

use serde::{Deserialize, Serialize};
use serde_json::value::RawValue;

use super::model::ModelRef;
use crate::blob::BlobRef;
use crate::codec::{Payload, parse};
use crate::error::InvalidValue;
use crate::payload::principal::open_kind;

pub(crate) const STARTED: &str = "turn.started";
pub(crate) const ENDED: &str = "turn.ended";

/// A turn begins. `config` points to the turn's full configuration (system prompt, tools,
/// parameters), stored once by hash so every turn can be reproduced.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TurnStarted {
    pub model: ModelRef,
    pub config: BlobRef,
    /// The W3C trace the turn belongs to, linking it to OpenTelemetry spans. Since version 2.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trace: Option<TraceId>,
    /// How the caller who started the turn signed in. Never a token. Since version 2.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auth: Option<Auth>,
}

/// Version 1, before a turn recorded its trace and its caller's sign-in.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TurnStartedV1 {
    model: ModelRef,
    config: BlobRef,
}

impl From<TurnStartedV1> for TurnStarted {
    fn from(old: TurnStartedV1) -> Self {
        Self {
            model: old.model,
            config: old.config,
            trace: None,
            auth: None,
        }
    }
}

/// A W3C trace id: 32 lowercase hex digits, not all zero.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct TraceId(String);

impl TraceId {
    pub fn new(id: impl Into<String>) -> Result<Self, InvalidValue> {
        let id = id.into();
        let hex = id.len() == 32 && id.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'));
        if !hex || id.bytes().all(|b| b == b'0') {
            return Err(InvalidValue::new(
                "trace id",
                format!("must be 32 lowercase hex digits, not all zero, got {id:?}"),
            ));
        }
        Ok(Self(id))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for TraceId {
    type Error = InvalidValue;

    fn try_from(id: String) -> Result<Self, Self::Error> {
        Self::new(id)
    }
}

impl From<TraceId> for String {
    fn from(id: TraceId) -> Self {
        id.0
    }
}

/// How a caller signed in. Names the login, never a secret.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Auth {
    pub method: AuthMethod,
    /// The identity provider, for `oidc` and `saml`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub issuer: Option<String>,
    /// The caller's stable id at the identity provider.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subject: Option<String>,
    /// The sign-in session.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session: Option<String>,
}

open_kind! {
    /// How a caller signed in.
    pub enum AuthMethod {
        /// The local person: the operating system user is the proof.
        Local => "local",
        /// OpenID Connect.
        Oidc => "oidc",
        /// SAML.
        Saml => "saml",
    }
}

impl Payload for TurnStarted {
    const TYPE: &'static str = STARTED;
    const VERSION: u32 = 2;
}

pub(crate) fn decode_started(
    version: u32,
    data: &RawValue,
) -> Option<serde_json::Result<TurnStarted>> {
    match version {
        1 => Some(parse::<TurnStartedV1>(data).map(TurnStarted::from)),
        2 => Some(parse(data)),
        _ => None,
    }
}

/// A turn ends.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TurnEnded {
    pub outcome: Outcome,
}

/// How a turn ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    Completed,
    Cancelled,
    Failed,
    Interrupted,
    LimitReached,
}

impl Payload for TurnEnded {
    const TYPE: &'static str = ENDED;
    const VERSION: u32 = 1;
}

pub(crate) fn decode_ended(version: u32, data: &RawValue) -> Option<serde_json::Result<TurnEnded>> {
    match version {
        1 => Some(parse(data)),
        _ => None,
    }
}
