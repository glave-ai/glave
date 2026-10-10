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
    /// What the call would do, as the person is shown it.
    pub summary: String,
    /// Whether the call would show the model a file that usually holds secrets.
    pub secret: bool,
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
    const VERSION: u32 = 1;
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
        1 => Some(parse(data)),
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
