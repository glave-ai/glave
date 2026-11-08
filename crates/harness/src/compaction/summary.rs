//! Asking for a handoff summary: sent as the thread's own calls are, so the provider's cache
//! still holds the conversation, with the instruction last; an answer that cannot stand in for
//! the conversation asked for once more, adjusted for why; and every answer counted, failed
//! ones too.

use glave_event::{Part, StopReason, TurnResult};
use glave_ports::{
    BlobStore, Completion, EventLog, Failure, Message, ModelProvider, ModelRequest, ProviderError,
    Purpose, ToolChoice,
};

use crate::approval::Approver;
use crate::cancel::Cancel;
use crate::harness::{Harness, HarnessError, Thread};
use crate::room::trim_oldest;
use crate::toolset::ToolSet;

/// The least reasoning asked for when a summary's reasoning took its room: a level every
/// provider glave reaches names so (OpenAI's, Anthropic's and OpenRouter's); a gateway sends a
/// model that offers no such level its own.
const LEAST_EFFORT: &str = "low";

const INSTRUCTION: &str = "\
This conversation is about to outgrow the space available. Write a handoff summary that the \
work can continue from alone: the conversation above will be replaced by it, and whoever \
continues will know only what it says. Use these headings:

1. Request and rules: what the person wants, and every rule and constraint they gave, word for \
word where the wording matters. They hold to the end.
2. Done: each piece finished, with the files changed and how it was checked. It is not to be \
done again.
3. In progress: exactly where the current piece stands, what is changed and what is not yet.
4. Next: the steps left, in order, the very next one first: one small enough to finish before \
the space runs out again.
5. Failed: approaches that did not work and why, so they are not tried again.
6. Files read: each file and what was learned from it that the work still needs (names, \
signatures, values, where things are), so it needn't be read again unless it must be changed.

Keep the exact file paths, names, numbers and error messages that still matter. Leave out \
anything not needed to continue.";

/// Said when memory opens the conversation: it opens the restarted one too.
const MEMORY: &str = "Leave out the memories the <MEMORY> block holds: they open the \
conversation again after the summary.";

/// What the summary call is asked, its cap the one the request is sent with: about half as many
/// words as its tokens, which leaves room for reasoning and for text denser than prose.
fn instruction(words: u32, memory: bool) -> String {
    let memory = if memory {
        format!("{MEMORY} ")
    } else {
        String::new()
    };
    format!(
        "{INSTRUCTION} {memory}Keep it under {words} words: it is cut off past that. Do not call \
         any tools: reply with the summary only."
    )
}

/// Why a summary's answer cannot stand in for the conversation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Unusable {
    /// It stopped at its cap, cut short; `reasoned` when it reasoned first, which may have
    /// taken the room.
    Cut { reasoned: bool },
    /// It called a tool instead.
    CalledTool,
    /// It reasoned and wrote nothing.
    ReasoningOnly,
    /// It wrote nothing at all.
    Empty,
}

impl Unusable {
    /// Why `completion`, whose text is `text`, cannot be used, if it cannot.
    fn of(completion: &Completion, text: &str) -> Option<Self> {
        let reasoned = (completion.parts.iter()).any(|part| matches!(part, Part::Reasoning { .. }));
        if completion.stop == StopReason::MaxTokens {
            Some(Self::Cut { reasoned })
        } else if !completion.tool_calls.is_empty() {
            Some(Self::CalledTool)
        } else if !text.trim().is_empty() {
            None
        } else if reasoned {
            Some(Self::ReasoningOnly)
        } else {
            Some(Self::Empty)
        }
    }

    /// `request` changed for the one more try: less reasoning when reasoning took the room,
    /// no tools when it called one, half the words when it wrote past the cap.
    fn adjust(self, request: &mut ModelRequest, words: u32, memory: bool) {
        match self {
            Self::Cut { reasoned: true } | Self::ReasoningOnly => {
                request.effort = Some(LEAST_EFFORT.to_owned());
            }
            Self::CalledTool => request.tools.clear(),
            Self::Cut { reasoned: false } => {
                if let Some(Message::User { parts }) = request.messages.last_mut() {
                    *parts = vec![Part::Text {
                        text: instruction(words / 2, memory),
                    }];
                }
            }
            Self::Empty => {}
        }
    }

    /// Why it failed, in a few plain words.
    fn said(self) -> &'static str {
        match self {
            Self::Cut { .. } => "it was cut off at its limit",
            Self::CalledTool => "the model called a tool instead",
            Self::ReasoningOnly => "the model only reasoned",
            Self::Empty => "the model wrote nothing",
        }
    }
}

/// The summary's text: its text parts joined.
fn text_of(completion: &Completion) -> String {
    let texts: Vec<&str> = (completion.parts.iter())
        .filter_map(|part| match part {
            Part::Text { text } => Some(text.as_str()),
            _ => None,
        })
        .collect();
    texts.join("\n")
}

impl<P, L, B, A> Harness<P, L, B, A>
where
    P: ModelProvider,
    L: EventLog,
    B: BlobStore,
    A: Approver,
{
    /// The request for `thread`'s summary: its prompt, tools, messages (memory and files as its
    /// calls send them) and reasoning, as its own calls are sent, so the provider's cache holds
    /// all they share; then the instruction. The tools stay listed, since some providers refuse
    /// a conversation with tool calls but no tools; the instruction asks for text only.
    async fn summary_request(&self, thread: &Thread, offer: &ToolSet) -> (ModelRequest, u32) {
        let model = self.model_for(thread);
        let cap = self.summary_tokens(&model);
        let words = cap / 2;
        let memory = self.memory_for(thread).is_some();
        let (mut messages, files) =
            (self.with_files(&thread.scope, &model, self.sent(thread))).await;
        messages.push(Message::User {
            parts: vec![Part::Text {
                text: instruction(words, memory),
            }],
        });
        let request = ModelRequest {
            max_output_tokens: Some(cap),
            model,
            system: offer.system.clone(),
            messages,
            tools: offer.specs.clone(),
            tool_choice: ToolChoice::None,
            purpose: Purpose::Summary,
            files,
            effort: thread.effort.clone(),
            payer: Some(thread.payer()),
            route: None,
        };
        (request, words)
    }

    /// `thread`'s summary and its text. An answer that cannot stand in for the conversation is
    /// recorded at once, its reason logged, and asked for once more as [`Unusable`] says; one
    /// that still cannot fails. Every call is counted in `spent`, what it used too.
    pub(super) async fn summary(
        &self,
        thread: &mut Thread,
        offer: &ToolSet,
        spent: &mut TurnResult,
        cancel: &Cancel,
    ) -> Result<(Completion, String), HarnessError> {
        let (mut request, words) = self.summary_request(thread, offer).await;
        let memory = self.memory_for(thread).is_some();
        let mut tried = false;
        loop {
            let completion = self.summarize(&mut request, spent, cancel).await?;
            spent.take_in(&completion.responded());
            let text = text_of(&completion);
            let Some(unusable) = Unusable::of(&completion, &text) else {
                return Ok((completion, text));
            };
            tracing::warn!(
                reason = ?unusable,
                stop = ?completion.stop,
                again = !tried,
                "a summary came back that cannot stand in for the conversation"
            );
            let mut events = Vec::new();
            self.stage(thread, &mut events, &completion.responded())?;
            self.commit(thread, events).await?;
            if tried {
                let failed = format!("the model returned no summary: {}", unusable.said());
                return Err(ProviderError::Protocol(failed).into());
            }
            tried = true;
            unusable.adjust(&mut request, words, memory);
        }
    }

    /// Asks for the summary, counting each call in `spent`. A request refused as too long loses
    /// its oldest tool outputs and is asked again, until it fits or there are none left to
    /// lose. Only the model call can be dropped by a cancel: what follows it writes to the log.
    async fn summarize(
        &self,
        request: &mut ModelRequest,
        spent: &mut TurnResult,
        cancel: &Cancel,
    ) -> Result<Completion, HarnessError> {
        loop {
            let asked = cancel
                .run(self.provider.complete(request, &mut |_| {}))
                .await
                .ok_or(HarnessError::Cancelled)?;
            spent.model_calls = spent.model_calls.saturating_add(1);
            match asked {
                Err(error) if error.failure() == Failure::TooLong => {
                    self.learn_window(&request.model, &error);
                    if !trim_oldest(&mut request.messages) {
                        return Err(error.into());
                    }
                }
                other => return Ok(other?),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use glave_event::{Arguments, ModelRef, TokenCounts, Usage};
    use glave_ports::RequestedCall;

    fn completion(parts: Vec<Part>, calls: usize, stop: StopReason) -> Completion {
        let call = RequestedCall {
            provider_call_id: None,
            tool: "read".into(),
            arguments: Arguments::Malformed("{".into()),
        };
        Completion {
            model: ModelRef {
                provider: "p".into(),
                name: "m".into(),
            },
            response_id: None,
            parts,
            tool_calls: vec![call; calls],
            stop,
            provider_stop: None,
            usage: Usage::new(TokenCounts::default(), false, None).unwrap(),
            cost: None,
            served: None,
            upstream: None,
        }
    }

    fn unusable(parts: Vec<Part>, calls: usize, stop: StopReason) -> Option<Unusable> {
        let completion = completion(parts, calls, stop);
        Unusable::of(&completion, &text_of(&completion))
    }

    #[test]
    fn only_a_whole_summary_in_text_stands_in() {
        let text = |text: &str| Part::Text { text: text.into() };
        let thought = Part::Reasoning {
            text: Some("thinking".into()),
            opaque: None,
        };
        let end = StopReason::EndTurn;
        assert_eq!(unusable(vec![text("Did A.")], 0, end), None);
        let cut = unusable(vec![thought.clone(), text("Did")], 0, StopReason::MaxTokens);
        assert_eq!(cut, Some(Unusable::Cut { reasoned: true }));
        let cut = unusable(vec![text("Did")], 0, StopReason::MaxTokens);
        assert_eq!(cut, Some(Unusable::Cut { reasoned: false }));
        let called = unusable(vec![text("Reading.")], 1, StopReason::ToolUse);
        assert_eq!(called, Some(Unusable::CalledTool));
        assert_eq!(
            unusable(vec![thought], 0, end),
            Some(Unusable::ReasoningOnly)
        );
        assert_eq!(unusable(vec![text(" \n")], 0, end), Some(Unusable::Empty));
    }

    #[test]
    fn the_instruction_states_the_cap_it_is_sent_with() {
        let asked = instruction(2_000, false);
        assert!(asked.contains("Keep it under 2000 words"), "{asked}");
        assert!(!asked.contains("<MEMORY>"));
        assert!(instruction(2_000, true).contains("Leave out the memories the <MEMORY> block"));
    }
}
