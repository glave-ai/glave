//! Sharing (`specs/session.md`, "Sharing"): a file a person shares at a link of its own, under
//! the shares ledger's id in the project that holds it. Each `share.set` holds a frozen copy of
//! the file, and its preview, under a holder of its own, and replaces the share's ones before; a
//! `share.stopped` ends it. The shares' list, in the server's own organization's scope, maps each
//! link by the SHA-256 of its id alone to the project that holds it (`share.placed`,
//! `share.unplaced`), so a server finds a link's share without knowing whose it is.

use serde::{Deserialize, Serialize};
use serde_json::value::RawValue;
use uuid::Uuid;

use crate::blob::BlobRef;
use crate::codec::{Body, Payload, parse};
use crate::envelope::At;
use crate::id::{AssetId, PrincipalId, ShareId, ThreadId};
use crate::payload::{AssetKind, MadePreview};

pub(crate) const SET: &str = "share.set";
pub(crate) const STOPPED: &str = "share.stopped";
pub(crate) const PLACED: &str = "share.placed";
pub(crate) const UNPLACED: &str = "share.unplaced";
pub(crate) const LINK_SENT: &str = "share.link.sent";
pub(crate) const OPENED: &str = "share.opened";

/// What a share is of: a file one of a chat's turns made, by the chat and its path there, or an
/// item of the Library.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum SharedFrom {
    File { thread: ThreadId, path: String },
    Item { asset: AssetId },
}

/// Who may open a share besides its owner.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Audience {
    /// Anyone with the link, without signing in.
    Anyone,
    /// Only these addresses, each proved with a code sent there.
    People { people: Vec<String> },
}

/// A person shared a file, changed who may open it, or moved it to a newer version.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ShareSet {
    pub share: ShareId,
    /// The link's id: the name under the share domain it is served at.
    pub link: String,
    pub of: SharedFrom,
    /// What it is called, as its card names it.
    pub name: String,
    /// The name its file saves as.
    pub file_name: String,
    pub kind: AssetKind,
    /// The copy shared.
    pub blob: BlobRef,
    /// Its preview as Markdown, where glave kept one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preview: Option<MadePreview>,
    /// The id its copy is held under, its own, so it outlives the chat.
    pub holder: ThreadId,
    pub audience: Audience,
    /// Who shares it: the project's owner, whose link stops opening once they leave its
    /// organization.
    pub owner: PrincipalId,
}

/// A share was stopped: its link opens nothing, and its holder let go of its copy.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ShareStopped {
    pub share: ShareId,
}

/// glave emailed a one-time link to an address a share lists: each kept as its SHA-256 alone.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ShareLinkSent {
    pub share: ShareId,
    /// The SHA-256 of the address, as glave reads addresses, in hex.
    pub to: String,
    /// The SHA-256 of the link's token, in hex.
    pub token: String,
    /// The `seq` of the share's list of people the link is for: changed since, it opens nothing.
    pub given: u64,
}

/// Someone proved an address a share lists, and their browser holds the proof: each kept as
/// its SHA-256 alone.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ShareOpened {
    pub share: ShareId,
    /// The SHA-256 of the address, in hex.
    pub to: String,
    /// The SHA-256 of the proof its browser holds, in hex.
    pub proof: String,
    /// The `seq` of the `share.link.sent` it spends.
    pub link: u64,
    /// The `seq` of the share's list of people it is for, as its link's.
    pub given: u64,
    /// When the proof stops working.
    pub until: At,
}

/// A link was made, which the shares' list now holds.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SharePlaced {
    /// The SHA-256 of the link's id, in hex.
    pub link: String,
    pub org: Uuid,
    pub project: Uuid,
    /// Who shares it, whose links are counted together.
    pub owner: PrincipalId,
}

/// A link stopped, and the shares' list no longer holds it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ShareUnplaced {
    /// The SHA-256 of the link's id, in hex.
    pub link: String,
    /// Who shared it, whose links count one fewer.
    pub owner: PrincipalId,
}

impl Payload for ShareSet {
    const TYPE: &'static str = SET;
    const VERSION: u32 = 1;
}

impl Payload for ShareStopped {
    const TYPE: &'static str = STOPPED;
    const VERSION: u32 = 1;
}

impl Payload for ShareLinkSent {
    const TYPE: &'static str = LINK_SENT;
    const VERSION: u32 = 1;
}

impl Payload for ShareOpened {
    const TYPE: &'static str = OPENED;
    const VERSION: u32 = 1;
}

impl Payload for SharePlaced {
    const TYPE: &'static str = PLACED;
    const VERSION: u32 = 1;
}

impl Payload for ShareUnplaced {
    const TYPE: &'static str = UNPLACED;
    const VERSION: u32 = 1;
}

/// Decodes the shares ledger's and the shares' list's events; `None` for any other type or
/// version.
pub(crate) fn decode(
    kind: &str,
    version: u32,
    data: &RawValue,
) -> Option<serde_json::Result<Body>> {
    if version != 1 {
        return None;
    }
    Some(match kind {
        SET => parse(data).map(Body::ShareSet),
        STOPPED => parse(data).map(Body::ShareStopped),
        PLACED => parse(data).map(Body::SharePlaced),
        UNPLACED => parse(data).map(Body::ShareUnplaced),
        LINK_SENT => parse(data).map(Body::ShareLinkSent),
        OPENED => parse(data).map(Body::ShareOpened),
        _ => return None,
    })
}
