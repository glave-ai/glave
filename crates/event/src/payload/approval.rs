//! `approval.requested` and `approval.decided`: a tool call that waits for the person, and
//! their answer. Linked by the call's `call_id`, which is also the approval's id.

use serde::{Deserialize, Serialize};
use serde_json::value::RawValue;

use crate::codec::{Payload, parse};
use crate::envelope::At;
use crate::id::CallId;
use crate::payload::caution::Caution;
use crate::payload::principal::Principal;

pub(crate) const REQUESTED: &str = "approval.requested";
pub(crate) const DECIDED: &str = "approval.decided";

crate::worded! {
    /// What an approval asks the person, by what would run: a command, an edit, a write, a
    /// read, a page, a service's tool by what it does and the service, work that would repeat,
    /// by when (or for a shape with no plain words, its next runs), or stop repeating (version
    /// 2), or any other tool.
    Ask {
        Command = "ask-command" "Run this command?",
        Edit = "ask-edit" "Edit this file?",
        Write = "ask-write" "Write this file?",
        Read = "ask-read" "Read this file?",
        Page = "ask-page" "Open this page?",
        With { act, service } = "ask-with" "{ $act } with { $service }?",
        Repeat { when } = "ask-repeat" "Repeat this { $when }?",
        RepeatNext { runs } = "ask-repeat-next" "Repeat this, next on { $runs }?",
        StopRepeating { name } = "ask-stop-repeating" "Stop repeating { $name }?",
        Allow = "ask-allow" "Allow this?",
    }
}

/// A tool call waits for the person to allow it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ApprovalRequested {
    pub call_id: CallId,
    pub tool: String,
    /// What would run, as the person is shown it: `bash: rm -rf ~/notes`. Never why it asks.
    pub summary: String,
    /// What it asks the person, in a sentence of glave's, by what would run: "Run this
    /// command?". Empty in requests recorded before it was.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub ask: String,
    /// What `ask` was written from, typed, so the session writes it again in the reader's
    /// language. Absent in requests recorded before it was, whose `ask` is sent as recorded.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub asking: Option<Ask>,
    /// What would run, as the object of `ask`: the summary without the tool's name it starts
    /// with, such as the command `rm -rf ~/notes`. Empty in requests recorded before it was.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub detail: String,
    /// Whether the call would show the model a secret: a file that usually holds one, or a
    /// command that prints one or could.
    pub secret: bool,
    /// Why the call needs the person in any mode that asks, when a caution says so, in sentences
    /// the person reads: "It deletes /home/ana/notes, outside the project."
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub caution: Option<String>,
    /// Why it asks, typed: what `caution` was written from, so the session writes it again in
    /// the reader's language. Absent where no caution asks, and in requests recorded before it
    /// was, whose `caution` is sent as it was recorded.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<Caution>,
    /// The command would run outside the sandbox the turn's commands run in, or in it with the
    /// network, as the model asked. Such a request asks every time, and is allowed only once.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outside_sandbox: Option<OutsideSandbox>,
    /// For a schedule's card (`ask-repeat`), what it was written from, so the session writes its
    /// question and its `detail` again in the reader's language (version 2).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repeating: Option<Repeating>,
    /// For a schedule's card, its lines, typed, as `detail` holds them a line each, so a client
    /// draws each by what it says; written again in the reader's language with `detail`
    /// (version 2).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lines: Option<CardLines>,
}

/// A schedule's card's lines, in the server's words: when it runs on whose clock, the chat its
/// runs go to, and what each run starts with.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CardLines {
    /// "Monday to Friday, 8:00 am your time. First run: Tuesday 17 November."
    pub when: String,
    /// "Each run is added to its own chat, “Morning check”."
    pub chat: String,
    /// "Each run starts with: “Check the build.”": glave always writes it (#1510).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt: Option<String>,
}

impl CardLines {
    /// The lines as `detail` holds them, one after another.
    pub fn joined(&self) -> String {
        let mut lines = vec![self.when.as_str(), self.chat.as_str()];
        lines.extend(self.prompt.as_deref());
        lines.join("\n")
    }
}

/// What a schedule's card was written from: when it repeats, on whose clocks, its chat's name,
/// its first run, and what each run starts with.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Repeating {
    /// A five-field cron expression, on the clocks of `zone`.
    pub cron: String,
    /// An IANA time zone.
    pub zone: String,
    /// The schedule's name, and its chat's.
    pub name: String,
    pub first: At,
    /// What each run starts with, in the model's words: glave always writes it (#1510).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt: Option<String>,
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
    const VERSION: u32 = 2;
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
        // Version 2 only adds what an approval may ask and why: version 1 reads as it is.
        1 | 2 => Some(parse(data)),
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
