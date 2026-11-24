//! Sharing (`specs/session.md`, "Sharing"): which file a share is of, as `share/get` and
//! `share/set` name it, and a share as they answer it.

use glave_event::{AssetId, At, Audience, Sha256, ThreadId};
use glave_ports::ProjectId;
use serde::{Deserialize, Serialize};

use super::Told;

/// A file the caller may share, named as `thread/files` or `library/list` gives it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ShareOf {
    /// A file one of `thread`'s turns made, by the content `thread/files` gives it: the version
    /// a new share holds.
    File {
        thread: ThreadId,
        #[schemars(with = "String")]
        sha256: Sha256,
    },
    /// An item of the caller's Library.
    Item { project: ProjectId, asset: AssetId },
}

/// Who besides the caller a server lets them share with, as `server/discover` says.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ShareChoice {
    /// Specific people, by email, each proving it with a code sent there.
    People,
    /// Anyone with the link.
    Anyone,
}

/// A file shared at a link of its own.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct ShareInfo {
    /// Who besides its owner may open it.
    pub audience: Audience,
    /// Its link, the one address it is served at.
    pub link: String,
    /// When it was last shared, changed or moved to a newer version.
    pub shared_at: At,
    /// The content of the version it holds, which a client compares with a newer card's.
    #[schemars(with = "String")]
    pub sha256: Sha256,
    /// Who may open it, in a sentence: "Shared with 2 people".
    pub said: Told,
}

/// What `share/set` chooses for a file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct ShareSetting {
    #[serde(flatten)]
    pub of: ShareOf,
    /// Who besides the caller may open it; none stops sharing it, and its link stops working.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub audience: Option<Audience>,
    /// Whether a share of a made file moves to `of`'s version, keeping its link: else it holds
    /// the version it was shared at.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub update: bool,
}
