//! What a reply drew on from the web, as `thread/sources` gives it.

use glave_event::{Seq, WebPage};
use serde::{Deserialize, Serialize};

use super::Told;

/// The pages one reply drew on: each one its turn read with `web_fetch` before it, once, in the
/// order first read. A page that failed to load is none, nor is a search's result the agent
/// did not read.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct ReplySources {
    /// The `seq` of the reply's `message.added`: the last words of its turn.
    pub reply: Seq,
    /// What they are, in a sentence: "Sources: 3 pages".
    pub sentence: Told,
    pub pages: Vec<WebPage>,
}
