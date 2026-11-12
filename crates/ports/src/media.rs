//! What glave's media tools ask of a model provider, and what comes back (`specs/gateway.md`,
//! "Media tools"): a [`crate::ModelRequest`] whose purpose is the tool's, its prompt and images
//! in its one message, with what else the call asks beside it ([`MediaAsk`]); and the files
//! made, whole, with the call's record as a model call's ([`Made`]).

use std::fmt;

use serde_json::{Map, Value};

use crate::model::Completion;

/// How good, and so how slow and dear, what a media call makes is (`specs/gateway.md`, "Media
/// tools"): the server's settings map each tier to a model and its settings.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Quality {
    /// Seconds and cents, for trying ideas.
    Low,
    #[default]
    Medium,
    /// The slowest and dearest, for finals.
    High,
}

/// What a media call asks beyond the prompt and images its request holds.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MediaAsk {
    /// The image's shape, width to height, such as `16:9`; none leaves it to the model.
    pub aspect_ratio: Option<String>,
    pub quality: Quality,
    /// Whether the image's background is to be transparent.
    pub transparent: bool,
    /// What the server's settings add for the model that serves the call, as its service
    /// names them (`quality`, `resolution`): a gateway sets them from the target, a tool never.
    pub settings: Map<String, Value>,
}

/// One file a media call made, whole, as its provider made it.
#[derive(Clone, PartialEq, Eq)]
pub struct MadeFile {
    /// What it holds, as the provider says, such as `image/png`; none when it says nothing.
    pub media_type: Option<String>,
    /// Its bytes, in base64, as providers send them.
    pub data: String,
}

impl fmt::Debug for MadeFile {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MadeFile")
            .field("media_type", &self.media_type)
            .field("data", &self.data.len())
            .finish()
    }
}

/// What a media call made, in order, and its record as a model call's: what it cost and who
/// served it, for `model.responded`.
#[derive(Debug, Clone, PartialEq)]
pub struct Made {
    pub files: Vec<MadeFile>,
    pub completion: Completion,
}
