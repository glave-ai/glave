//! What reads a file a turn made apart from glave (`specs/session.md`, "Files it made"): its
//! title, its pages, sheets or slides, and its preview, read in a process of glave's own, so a
//! file crafted to take more memory or time than reading it should ends that process, never the
//! server.

use std::sync::Arc;

use glave_event::Parts;
use serde::{Deserialize, Serialize};

use crate::BoxFuture;

/// What reading a file a turn made found, as the process that read it prints it.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MadeRead {
    /// The title its own properties give it, when they give one that names it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// How many pages, sheets or slides it has, when that is known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parts: Option<Parts>,
    /// Its preview as Markdown, when glave previews a file of its kind.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub markdown: Option<String>,
    /// Whether the preview leaves out some of the file.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub cut: bool,
}

/// Reads a file a turn made, given its bytes and its media type: `None` when it couldn't be
/// read, in time or at all.
pub type ReadMade =
    Arc<dyn Fn(Vec<u8>, String) -> BoxFuture<'static, Option<MadeRead>> + Send + Sync>;
