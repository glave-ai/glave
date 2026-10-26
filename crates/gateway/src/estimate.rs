//! What a request needs, read before it is sent: its size in tokens, whether it uses tools or
//! images, and whose opaque reasoning it holds.

use glave_event::{Arguments, Opaque, Part};
use glave_ports::{Message, ModelRequest};

/// Bytes per token when judging whether a request fits a target's window: generous, so a
/// request that may fit is sent, and the provider has the last word.
const FIT_BYTES_PER_TOKEN: u64 = 4;
/// Bytes per token when reserving a budget: cautious, since code and other languages run
/// closer to 3, and settling corrects it.
const RESERVE_BYTES_PER_TOKEN: u64 = 3;

/// What a request needs from the target that serves it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Needs<'a> {
    /// Bytes of everything the model reads: system prompt, conversation and tools.
    pub(crate) bytes: u64,
    pub(crate) tools: bool,
    /// Whether it sends an image or a PDF inline, which only a model that reads images reads.
    pub(crate) images: bool,
    /// The provider and model of the latest opaque reasoning in the conversation, if any.
    pub(crate) reasoning: Option<&'a Opaque>,
}

impl<'a> Needs<'a> {
    pub(crate) fn of(request: &'a ModelRequest) -> Self {
        let mut needs = Self {
            bytes: len(&request.system),
            tools: !request.tools.is_empty(),
            images: !request.files.is_empty(),
            reasoning: None,
        };
        for tool in &request.tools {
            needs.bytes = needs
                .bytes
                .saturating_add(len(&tool.name))
                .saturating_add(len(&tool.description))
                .saturating_add(len(&tool.parameters.to_string()));
        }
        for message in &request.messages {
            match message {
                Message::User { parts } | Message::Tool { output: parts, .. } => {
                    needs.add_parts(parts);
                }
                Message::Assistant { parts, tool_calls } => {
                    needs.add_parts(parts);
                    for call in tool_calls {
                        let arguments = match &call.arguments {
                            Arguments::Json(map) => {
                                serde_json::to_string(map).map_or(0, |j| len(&j))
                            }
                            Arguments::Malformed(raw) => len(raw),
                        };
                        needs.bytes = needs
                            .bytes
                            .saturating_add(len(&call.tool))
                            .saturating_add(arguments);
                    }
                }
            }
        }
        needs
    }

    fn add_parts(&mut self, parts: &'a [Part]) {
        for part in parts {
            let bytes = match part {
                Part::Text { text } => len(text),
                Part::Reasoning { text, opaque } => {
                    if opaque.is_some() {
                        self.reasoning = opaque.as_ref();
                    }
                    text.as_deref()
                        .map_or(0, len)
                        .saturating_add(opaque.as_ref().map_or(0, |o| len(&o.data)))
                }
                Part::File { .. } => 0,
            };
            self.bytes = self.bytes.saturating_add(bytes);
        }
    }

    /// The request's input, estimated generously: whether it fits a window.
    pub(crate) fn fit_tokens(&self) -> u64 {
        self.bytes.div_ceil(FIT_BYTES_PER_TOKEN)
    }

    /// The request's input, estimated cautiously: what a budget reserves.
    pub(crate) fn reserve_tokens(&self) -> u64 {
        self.bytes.div_ceil(RESERVE_BYTES_PER_TOKEN)
    }
}

fn len(text: &str) -> u64 {
    u64::try_from(text.len()).unwrap_or(u64::MAX)
}
