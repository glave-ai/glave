//! `approval.requested` and `approval.decided`: a tool call that waits for the person, and
//! their answer. Linked by the call's `call_id`, which is also the approval's id.

use serde::{Deserialize, Serialize};
use serde_json::value::RawValue;

use crate::codec::{Payload, parse};
use crate::id::CallId;
use crate::payload::principal::Principal;

pub(crate) const REQUESTED: &str = "approval.requested";
pub(crate) const DECIDED: &str = "approval.decided";

/// A tool call waits for the person to allow it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ApprovalRequested {
    pub call_id: CallId,
    pub tool: String,
    /// What would run, as the person is shown it: `bash: rm -rf ~/notes`. Never why it asks.
    pub summary: String,
    /// Whether the call would show the model a secret: a file that usually holds one, or a
    /// command that prints one or could.
    pub secret: bool,
    /// Why the call needs the person in any mode that asks, when a caution says so, in sentences
    /// the person reads: "It deletes /home/ana/notes, outside the project."
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub caution: Option<String>,
    /// The command would run outside the sandbox the turn's commands run in, or in it with the
    /// network, as the model asked. Such a request asks every time, and is allowed only once.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outside_sandbox: Option<OutsideSandbox>,
}

/// A request to run one command outside the sandbox, or in it with the network.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OutsideSandbox {
    /// Why, in the model's own words as it wrote them: shown as the model's, never as glave's.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    /// What of the sandbox it runs without: all of it, unless only the network.
    #[serde(default, skip_serializing_if = "Lifted::is_sandbox")]
    pub lifted: Lifted,
}

/// What of the sandbox one command runs without.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Lifted {
    /// The whole sandbox: the command runs as the person, with none of its limits.
    #[default]
    Sandbox,
    /// Only its wall around the network: the command reaches the internet, and stays in the
    /// sandbox otherwise.
    Network,
}

impl Lifted {
    #[expect(
        clippy::trivially_copy_pass_by_ref,
        reason = "serde's skip_serializing_if"
    )]
    fn is_sandbox(&self) -> bool {
        *self == Self::Sandbox
    }
}

/// The answer to an approval.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ApprovalDecided {
    pub call_id: CallId,
    pub decision: Verdict,
    /// Who decided. Absent: the thread's `for`. Automatic outcomes (`no_one_there`,
    /// `cancelled`) are decided by glave.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub by: Option<Principal>,
}

/// How an approval was answered.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Verdict {
    /// Allowed this once.
    AllowOnce,
    /// Allowed, and the same tool needs no asking for the rest of the thread.
    AllowForThread,
    Deny,
    /// No one was there to answer, so the call did not run.
    NoOneThere,
    /// The turn was cancelled while the approval waited.
    Cancelled,
}

impl Payload for ApprovalRequested {
    const TYPE: &'static str = REQUESTED;
    const VERSION: u32 = 1;
}

impl Payload for ApprovalDecided {
    const TYPE: &'static str = DECIDED;
    const VERSION: u32 = 1;
}

pub(crate) fn decode_requested(
    version: u32,
    data: &RawValue,
) -> Option<serde_json::Result<ApprovalRequested>> {
    match version {
        1 => Some(parse(data)),
        _ => None,
    }
}

pub(crate) fn decode_decided(
    version: u32,
    data: &RawValue,
) -> Option<serde_json::Result<ApprovalDecided>> {
    match version {
        1 => Some(parse(data)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_request_to_run_outside_the_sandbox_keeps_the_model_s_reason_apart() {
        let read = |data: &str| {
            let raw = RawValue::from_string(data.to_owned()).unwrap();
            decode_requested(1, &raw).unwrap().unwrap()
        };
        let asked = read(
            r#"{"call_id":"call_4","tool":"bash","summary":"bash: cargo add serde","secret":false,"caution":"It runs outside the sandbox.","outside_sandbox":{"reason":"cargo add writes ~/.cargo/registry"}}"#,
        );
        assert_eq!(asked.summary, "bash: cargo add serde");
        assert_eq!(
            asked.outside_sandbox.and_then(|outside| outside.reason),
            Some("cargo add writes ~/.cargo/registry".to_owned())
        );
        let without_reason = read(
            r#"{"call_id":"call_4","tool":"bash","summary":"bash: cargo add serde","secret":false,"outside_sandbox":{}}"#,
        );
        assert_eq!(
            without_reason.outside_sandbox,
            Some(OutsideSandbox::default())
        );
        let network = read(
            r#"{"call_id":"call_5","tool":"bash","summary":"bash: npm install","secret":false,"outside_sandbox":{"reason":"npm install downloads packages","lifted":"network"}}"#,
        );
        assert_eq!(
            network.outside_sandbox.map(|outside| outside.lifted),
            Some(Lifted::Network)
        );
        let whole = serde_json::to_string(&OutsideSandbox::default()).unwrap();
        assert_eq!(whole, "{}", "the whole sandbox, as before");
    }
}
