//! A request that failed, as JSON-RPC carries it.

use glave_ports::Failure;
use serde::{Deserialize, Serialize};

use super::{Told, UnknownPasskey, code};
use crate::words::Sentence;

/// A request that failed, as JSON-RPC carries it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct ApiError {
    pub code: i64,
    /// What a person is told, in their language where it is the server's own sentence.
    pub message: Told,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<ErrorData>,
}

/// What a failure says beyond its sentence, for a client to act on.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(untagged)]
pub enum ErrorData {
    /// Why a model call failed, for a client to say what to fix.
    #[schemars(title = "ModelFailure")]
    Failed { failure: Failure },
    /// The passkey a browser signed in with that the server does not know.
    #[schemars(title = "UnknownPasskeyFailure")]
    UnknownPasskey { unknown_passkey: UnknownPasskey },
    /// The organization the request named, which the caller is not in (any more): a client
    /// stops naming it.
    #[schemars(title = "NotInOrganization")]
    NotIn { not_in: OrgNamed },
}

/// That a request named an organization, in `ErrorData::NotIn`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum OrgNamed {
    /// The one `Glave-Organization` named.
    Organization,
}

impl ApiError {
    pub fn new(code: i64, message: impl Into<Told>) -> Self {
        Self {
            code,
            message: message.into(),
            data: None,
        }
    }

    /// For a client following a turn on a connection that carries another turn's notifications
    /// too: its message was queued to a turn already running, which it cannot follow.
    pub(crate) fn queued_elsewhere() -> Self {
        Self::new(code::TURN_RUNNING, Sentence::QueuedElsewhere)
    }
}

impl std::fmt::Display for ApiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for ApiError {}
