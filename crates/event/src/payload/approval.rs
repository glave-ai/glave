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
    /// Whether the call would show the model a file that usually holds secrets.
    pub secret: bool,
    /// Why the call needs the person whatever the mode, when a caution says so, in sentences
    /// the person reads: "It deletes /home/ana/notes, outside the project." Since version 2.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub caution: Option<String>,
}

/// Version 1, before the caution had a field of its own: it was folded into `summary`.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ApprovalRequestedV1 {
    call_id: CallId,
    tool: String,
    summary: String,
    secret: bool,
}

impl From<ApprovalRequestedV1> for ApprovalRequested {
    /// Its summary keeps the caution it may end with, in brackets, as it was written.
    fn from(old: ApprovalRequestedV1) -> Self {
        Self {
            call_id: old.call_id,
            tool: old.tool,
            summary: old.summary,
            secret: old.secret,
            caution: None,
        }
    }
}

/// The answer to an approval.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ApprovalDecided {
    pub call_id: CallId,
    pub decision: Verdict,
    /// Who decided. Absent: the thread's `for`. Automatic outcomes (`no_one_there`,
    /// `cancelled`) are decided by glave. Since version 2.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub by: Option<Principal>,
}

/// Version 1, before a decision named who made it.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ApprovalDecidedV1 {
    call_id: CallId,
    decision: Verdict,
}

impl From<ApprovalDecidedV1> for ApprovalDecided {
    fn from(old: ApprovalDecidedV1) -> Self {
        let by = match old.decision {
            Verdict::NoOneThere | Verdict::Cancelled => Some(Principal::glave()),
            Verdict::AllowOnce | Verdict::AllowForThread | Verdict::Deny => None,
        };
        Self {
            call_id: old.call_id,
            decision: old.decision,
            by,
        }
    }
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
    const VERSION: u32 = 2;
}

impl Payload for ApprovalDecided {
    const TYPE: &'static str = DECIDED;
    const VERSION: u32 = 2;
}

pub(crate) fn decode_requested(
    version: u32,
    data: &RawValue,
) -> Option<serde_json::Result<ApprovalRequested>> {
    match version {
        1 => Some(parse::<ApprovalRequestedV1>(data).map(ApprovalRequested::from)),
        2 => Some(parse(data)),
        _ => None,
    }
}

pub(crate) fn decode_decided(
    version: u32,
    data: &RawValue,
) -> Option<serde_json::Result<ApprovalDecided>> {
    match version {
        1 => Some(parse::<ApprovalDecidedV1>(data).map(ApprovalDecided::from)),
        2 => Some(parse(data)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const V1: &str = r#"{"call_id":"call_9","tool":"bash","summary":"bash: git clean -fdx (it deletes untracked files)","secret":false}"#;

    #[test]
    fn version_1_reads_with_its_summary_as_written_and_no_caution_of_its_own() {
        let raw = RawValue::from_string(V1.to_owned()).unwrap();
        let upgraded = decode_requested(1, &raw).unwrap().unwrap();
        assert_eq!(
            upgraded.summary,
            "bash: git clean -fdx (it deletes untracked files)"
        );
        assert_eq!(upgraded.caution, None);
        let with_caution = V1.replacen('{', r#"{"caution":"It deletes untracked files.","#, 1);
        let raw = RawValue::from_string(with_caution).unwrap();
        assert!(
            decode_requested(1, &raw).unwrap().is_err(),
            "version 1 had no caution"
        );
        let upgraded = decode_requested(2, &raw).unwrap().unwrap();
        assert_eq!(
            upgraded.caution.as_deref(),
            Some("It deletes untracked files.")
        );
    }
}
