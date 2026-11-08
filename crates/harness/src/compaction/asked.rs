//! `compact`: the person asks, in their own words ("go ahead and compact, and keep the API
//! decisions front and centre"), and the model compacts the thread, passing what they want kept
//! as its `focus`, which shapes the summary. There is no command for it, as Claude Code's
//! `/compact [instructions]` and Codex's `/compact` are: the model reads the request in any
//! words and any language. Listed from a thread's first call, so it never changes a prefix
//! mid-thread (`specs/invariants.md`, 27); a subagent, whom no person talks to, never has it.
//!
//! The call compacts at once: everything before the response that made it is summarized, and
//! that response and the results after it stay, so its own result follows it and says what
//! was kept.

use glave_event::Arguments;
use glave_ports::{BlobStore, EventLog, Message, ModelProvider, ToolCall, ToolOutput, ToolSpec};
use serde_json::json;

use super::Covers;
use crate::approval::Approver;
use crate::cancel::Cancel;
use crate::harness::{Harness, HarnessError, Thread};
use crate::turn::{Live, Step};

/// The tool's name.
pub(crate) const COMPACT: &str = "compact";

/// The longest focus kept, in characters: a line or two of what the person said.
const MOST_FOCUS: usize = 1_000;

pub(crate) fn spec() -> ToolSpec {
    ToolSpec {
        name: COMPACT.to_owned(),
        description: "Compact this conversation: summarize everything so far to free space, and \
                      go on from the summary. Only when the person asks for it, in any words \
                      (\"go ahead and compact\"); never on your own, since space is freed \
                      automatically. Put what they want kept front and centre in focus, in \
                      their words."
            .to_owned(),
        parameters: json!({
            "type": "object",
            "properties": {
                "focus": {
                    "type": "string",
                    "description": "What the person wants the summary to keep front and \
                                    centre, if they said."
                }
            },
            "additionalProperties": false
        }),
    }
}

/// What the person wants kept front and centre, as the call gives it: trimmed, cut short, none
/// when empty.
fn focus(arguments: &Arguments) -> Option<String> {
    let Arguments::Json(map) = arguments else {
        return None;
    };
    let focus = map.get("focus")?.as_str()?.trim();
    (!focus.is_empty()).then(|| focus.chars().take(MOST_FOCUS).collect())
}

/// A `compact` call in a line, for the person: what it keeps, when it says.
pub(crate) fn describe(arguments: &Arguments) -> String {
    focus(arguments).map_or_else(
        || "Compact the conversation".to_owned(),
        |focus| format!("Compact the conversation, keeping {focus}"),
    )
}

impl<P, L, B, A> Harness<P, L, B, A>
where
    P: ModelProvider,
    L: EventLog,
    B: BlobStore,
    A: Approver,
{
    /// Runs a `compact` call: everything before the response that made it summarized, with the
    /// call's focus, in place of any summary being written in the background. A summary that
    /// fails leaves the thread as it was and says why.
    pub(crate) async fn compact_asked(
        &self,
        thread: &mut Thread,
        call: &ToolCall,
        step: &mut Step,
        cancel: &Cancel,
        live: &mut (dyn FnMut(Live) + Send),
    ) -> Result<ToolOutput, HarnessError> {
        let focus = focus(&call.arguments);
        let covers = (thread.messages.iter())
            .rposition(|message| matches!(message, Message::Assistant { .. }))
            .unwrap_or_default();
        if covers == 0 {
            return Ok(ToolOutput::text(
                false,
                "There is nothing before this to compact yet.",
            ));
        }
        thread.background.stop();
        let covers_what = Covers::Asked {
            focus: focus.as_deref(),
        };
        let compacted = self
            .compact_older(thread, &mut step.toolset, covers, covers_what, cancel)
            .await;
        let compacted = match compacted {
            Ok(compacted) => compacted,
            Err(HarnessError::Provider(error)) => {
                return Ok(ToolOutput::text(
                    true,
                    format!("The conversation could not be compacted: {error}"),
                ));
            }
            Err(error) => return Err(error),
        };
        step.summary.model_calls = step.summary.model_calls.saturating_add(1);
        step.summary.take_in(&compacted.completion.responded());
        live(Live::Compacted {
            before_tokens: compacted.before_tokens,
            after_tokens: compacted.after_tokens,
            waited: true,
        });
        let kept = focus.map_or_else(String::new, |focus| {
            format!(", keeping this front and centre: {focus}")
        });
        Ok(ToolOutput::text(
            false,
            format!(
                "Compacted: everything before this call is now the summary that opens the \
                 conversation{kept}. Tell the person it is done, in a line, and go on with \
                 anything else they asked."
            ),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn arguments(value: &serde_json::Value) -> Arguments {
        Arguments::Json(value.as_object().cloned().unwrap_or_default())
    }

    #[test]
    fn a_call_says_what_it_keeps() {
        let asked = arguments(&json!({ "focus": "  the API decisions " }));
        assert_eq!(
            describe(&asked),
            "Compact the conversation, keeping the API decisions"
        );
        assert_eq!(describe(&arguments(&json!({}))), "Compact the conversation");
        assert_eq!(
            describe(&arguments(&json!({ "focus": " " }))),
            "Compact the conversation"
        );
        assert_eq!(focus(&Arguments::Malformed("{".into())), None);
    }
}
