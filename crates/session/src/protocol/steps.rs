//! A thread's turns in steps, as `thread/steps` gives it: each run of tool calls in a line, and
//! which of the model's messages is its turn's reply.

use glave_event::{CallId, Seq};
use serde::{Deserialize, Serialize};

use super::Told;

/// Each run of a thread's tool calls, and its turns' replies.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct ThreadSteps {
    /// Each run of calls, in order.
    pub runs: Vec<CallRun>,
    /// The `seq` of each reply: the last `message.added` of the model's with words in a turn
    /// that ended. Every other message of the model's is said along the way.
    pub replies: Vec<Seq>,
}

/// Tool calls one after another in a turn, with nothing said between them: no message with
/// words, and no questions for the person, the model's or a service's.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct CallRun {
    /// The `seq` of its first call's `tool.call.requested`.
    pub first: Seq,
    /// Its calls, in order.
    pub calls: Vec<CallId>,
    /// What they did, in a line: "Ran 2 commands (1 failed)", "Ran 1 command, read 1 file";
    /// in the present while one of them runs.
    pub sentence: Told,
}
