//! What a compaction says: what the summary call is asked for, and the message the conversation
//! restarts from, for each way a compaction comes about.

use std::fmt::Write as _;

/// What a summary covers, which decides what it is asked for and how the restart reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Covers<'a> {
    /// The whole conversation, as it outgrows the space: today's compaction.
    Whole,
    /// All but its latest messages, written in the background: those follow the restart as they
    /// were.
    Older {
        /// The most words the summary may take: a third of what it replaces.
        words: u64,
    },
    /// Everything before the response that asked for it, because the person asked, with what
    /// they want kept front and centre, if they said.
    Asked { focus: Option<&'a str> },
}

/// What each heading of a summary asks for, so the work can go on from it alone.
const REQUEST: &str = "Request and rules: what the person wants, and every rule and constraint \
     they gave, word for word where the wording matters. They hold to the end.";
const DONE: &str = "Done: each piece finished, with the files changed and how it was checked. \
     It is not to be done again.";
const IN_PROGRESS: &str = "In progress: exactly where the current piece stands, what is changed \
     and what is not yet.";
const NEXT: &str = "Next: the steps left, in order, the very next one first: one small enough to \
     finish before the space runs out again.";
/// For the older part, whose next steps are in the latest messages.
const WHERE_IT_STOOD: &str = "Where it stood: what was in progress when this part ends.";
const FAILED: &str = "Failed: approaches that did not work and why, so they are not tried again.";
const FILES: &str = "Files read: each file and what was learned from it that the work still \
     needs (names, signatures, values, where things are), so it needn't be read again unless it \
     must be changed.";
const KEEP: &str = "Keep the exact file paths, names, numbers and error messages that still \
     matter.";
const TEXT_ONLY: &str = "Do not call any tools: reply with the summary only.";

/// `items` as numbered headings, then what to keep and leave out: `left_out`.
fn headings(items: &[&str], left_out: &str) -> String {
    let mut text = String::new();
    for (number, item) in (1..).zip(items) {
        let _ = writeln!(text, "{number}. {item}");
    }
    format!("{text}\n{KEEP} {left_out} {TEXT_ONLY}")
}

/// What the summary call is asked, as the conversation's last message.
pub(super) fn instruction(covers: Covers<'_>) -> String {
    match covers {
        Covers::Whole => format!(
            "This conversation is about to outgrow the space available. Write a handoff summary \
             that the work can continue from alone: the conversation above will be replaced by \
             it, and whoever continues will know only what it says. Use these headings:\n\n{}",
            headings(
                &[REQUEST, DONE, IN_PROGRESS, NEXT, FAILED, FILES],
                "Leave out anything not needed to continue."
            )
        ),
        Covers::Older { words } => format!(
            "The conversation above is the older part of a longer one: a newer part follows it, \
             and stays as it is. Write a summary of the part above that the newer part can go on \
             from: the part above will be replaced by it. Keep it under {words} words. Use these \
             headings:\n\n{}",
            headings(
                &[REQUEST, DONE, WHERE_IT_STOOD, FAILED, FILES],
                "Leave out anything the newer part will not need."
            )
        ),
        Covers::Asked { focus } => {
            let mut text = format!(
                "The person asked for this conversation to be compacted. Write a handoff summary \
                 that the work can continue from alone: the conversation above will be replaced \
                 by it, and whoever continues will know only what it says. Use these \
                 headings:\n\n{}",
                headings(
                    &[REQUEST, DONE, IN_PROGRESS, NEXT, FAILED, FILES],
                    "Leave out anything not needed to continue."
                )
            );
            if let Some(focus) = focus {
                let _ = write!(
                    text,
                    "\n\nThe person wants this kept front and centre, in full, whatever else is \
                     left out: {focus}"
                );
            }
            text
        }
    }
}

/// The message a compacted conversation restarts from: the summary, the latest tool results as
/// they were when the whole was summarized, and the person's latest request when the summary
/// covers it.
pub(super) fn restart(
    covers: Covers<'_>,
    summary: &str,
    recent: Option<&str>,
    request: Option<&str>,
) -> String {
    let opening = match covers {
        Covers::Whole => "[Earlier parts of this conversation were summarized to save space. The \
             summary holds what was done and learned: go on from its next step. Read only what \
             that step needs, one file at a time, not everything again at once, or the space runs \
             out again before anything is done.]"
            .to_owned(),
        Covers::Older { .. } => "[The older part of this conversation was summarized to save \
             space. The summary holds what was done and learned before the messages that follow, \
             which are as they were: go on from the latest of them.]"
            .to_owned(),
        Covers::Asked { focus } => {
            let kept = focus.map_or_else(String::new, |focus| {
                format!(", keeping this front and centre: {focus}")
            });
            format!(
                "[The person asked for this conversation to be compacted, and it is done: what \
                 came before your call to compact is summarized here{kept}. Do not compact again \
                 unless they ask again.]"
            )
        }
    };
    let mut text = format!("{opening}\n\n{}", summary.trim());
    if let Some(recent) = recent {
        let _ = write!(
            text,
            "\n\nYour latest tool calls and what they returned, as they were:{recent}"
        );
    }
    if let Some(request) = request {
        let _ = write!(
            text,
            "\n\nThe person's latest request, word for word:\n{request}"
        );
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_restart_message_keeps_the_request_word_for_word() {
        let text = restart(
            Covers::Whole,
            "  Did A and B.\n",
            Some("\n\n`read` a:\nA"),
            Some("fix the build"),
        );
        assert!(text.starts_with("[Earlier parts of this conversation were summarized"));
        assert!(
            text.ends_with("]\n\nDid A and B.\n\nYour latest tool calls and what they returned, as they were:\n\n`read` a:\nA\n\nThe person's latest request, word for word:\nfix the build"),
            "{text}"
        );
        let bare = restart(Covers::Whole, "Did A.", None, None);
        assert!(!bare.contains("word for word") && !bare.contains("latest tool calls"));
    }

    #[test]
    fn a_focus_shapes_the_summary_and_the_restart_says_it_is_done() {
        let asked = Covers::Asked {
            focus: Some("the API decisions"),
        };
        let asking = instruction(asked);
        assert!(asking.starts_with("The person asked for this conversation to be compacted"));
        assert!(asking.ends_with("whatever else is left out: the API decisions"));
        let text = restart(asked, "Chose REST.", None, None);
        assert!(
            text.contains("it is done") && text.contains("front and centre: the API decisions"),
            "{text}"
        );
        assert!(!instruction(Covers::Asked { focus: None }).contains("front and centre"));
    }

    #[test]
    fn the_older_part_is_asked_for_briefly_and_the_latest_follow() {
        let asking = instruction(Covers::Older { words: 300 });
        assert!(asking.contains("under 300 words") && !asking.contains("4. Next"));
        let text = restart(Covers::Older { words: 300 }, "Read a.", None, None);
        assert!(text.contains("the messages that follow, which are as they were"));
    }
}
