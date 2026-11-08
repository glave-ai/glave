//! `compact`: the person asks in their own words ("go ahead and compact, and keep the API
//! decisions front and centre"), and the model calls it with what they want kept as its
//! `focus`. The call only notes the ask and answers at once; at the next boundary between model
//! calls the whole conversation is summarized as it is at the threshold, the call and its
//! result inside it, the focus kept front and centre (`compaction.rs`). The model reads the ask
//! in any words and any language, as Claude Code's `/compact [instructions]` takes its focus.
//! Listed from a thread's first call, so it never changes a prefix mid-thread
//! (`specs/invariants.md`, 27); a subagent, whom no person talks to, never has it. A turn
//! compacts once for it: another call is answered without compacting.

use glave_event::Arguments;
use glave_ports::{ToolCall, ToolOutput, ToolSpec};
use serde_json::json;

use crate::turn::{Live, Step};

/// The tool's name.
pub(crate) const COMPACT: &str = "compact";

/// The longest focus kept, in characters: a line or two of what the person said.
const MOST_FOCUS: usize = 1_000;

/// A compaction the person asked for in a turn.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) enum Asked {
    /// Not asked for in this turn.
    #[default]
    No,
    /// Asked for, with what to keep front and centre if they said: run at the next boundary.
    Waiting(Option<String>),
    /// Run in this turn, or tried: another call is answered without compacting.
    Done,
}

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

/// `, keeping this front and centre: <focus>`, or nothing without a focus.
pub(super) fn keeping(focus: Option<&str>) -> String {
    focus.map_or_else(String::new, |focus| {
        format!(", keeping this front and centre: {focus}")
    })
}

/// Runs a `compact` call: notes the ask in `step`, for the next boundary between model calls,
/// and answers at once. One after the turn's first is answered without compacting.
pub(crate) fn compact_asked(
    call: &ToolCall,
    step: &mut Step,
    live: &mut (dyn FnMut(Live) + Send),
) -> ToolOutput {
    let summary = describe(&call.arguments);
    live(Live::ToolStarted {
        summary: summary.clone(),
    });
    let output = if step.compact == Asked::No {
        let focus = focus(&call.arguments);
        let kept = keeping(focus.as_deref());
        step.compact = Asked::Waiting(focus);
        ToolOutput::text(
            false,
            format!("Noted: the conversation is compacted before your next step{kept}."),
        )
    } else {
        ToolOutput::text(
            true,
            "Compaction was already asked for in this turn, and a turn compacts once: it is not \
             done again.",
        )
    };
    live(Live::ToolFinished {
        summary,
        is_error: output.is_error,
    });
    output
}

impl Asked {
    /// The ask waiting to run, if one waits, marked run: a turn compacts once for it, whether
    /// or not the summary comes. `No` when none waits.
    pub(crate) fn run(&mut self) -> Self {
        if matches!(self, Self::Waiting(_)) {
            std::mem::replace(self, Self::Done)
        } else {
            Self::No
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn arguments(value: &serde_json::Value) -> Arguments {
        Arguments::Json(value.as_object().cloned().unwrap_or_default())
    }

    #[test]
    fn an_ask_runs_once() {
        let mut asked = Asked::Waiting(Some("the API".into()));
        assert_eq!(asked.run(), Asked::Waiting(Some("the API".into())));
        assert_eq!((asked.run(), asked), (Asked::No, Asked::Done));
        assert_eq!(Asked::No.run(), Asked::No);
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
