//! A command running in the background as `command/list` gives it: the harness's record of it,
//! with where it listens and the preview of what it serves there (`specs/session.md`, "Where
//! they listen", "Previews").

use glave_harness::RunningCommand;
use serde::{Deserialize, Serialize};

use super::Told;

/// A command a thread keeps running in the background, with where it listens.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct ListedCommand {
    #[serde(flatten)]
    pub command: RunningCommand,
    /// The TCP ports it listens on, lowest first, where the caller reaches this computer's
    /// loopback: looked for only then, and on the server's own computer.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub ports: Vec<Port>,
    /// Where it listens, in a sentence: "Listening on 4200 and 4201."
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub listens: Option<Told>,
}

/// A port a command listens on, and the preview of what it serves there, where glave
/// previews it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct Port {
    pub port: u16,
    /// Where a browser on this computer opens what it serves: the server's own address, on
    /// this computer's loopback; none where glave previews nothing.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    /// Whether a frame of the app shows it too, as its Content Security Policy allows.
    pub framed: bool,
    /// Why a frame can't show it, in a sentence, where it opens in a new tab alone.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unframed: Option<Told>,
}
