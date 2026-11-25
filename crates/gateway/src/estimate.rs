//! What a request needs, read before it is sent: its size in tokens, whether it uses tools or
//! images, and whose opaque reasoning it holds.

use glave_event::{Arguments, Opaque, Part};
use glave_ports::{InlineFile, Message, ModelRequest};

/// Bytes per token when judging whether a request fits a target's window: generous, so a
/// request that may fit is sent, and the provider has the last word.
pub(crate) const FIT_BYTES_PER_TOKEN: u64 = 4;
/// Bytes per token when holding a call's worst case: cautious, since code and other languages run
/// closer to 3, and settling corrects it.
const RESERVE_BYTES_PER_TOKEN: u64 = 3;
/// The most one image sent in to an image model reads, in tokens, as a media call's worst case
/// counts it: GPT Image reads every image in at high fidelity, 6,240 tokens for a 1536x1024 one
/// beside its tiles' own (OpenAI's image guide, 2026-11-12), Nano Banana Pro 560.
const IMAGE_IN_TOKENS: u64 = 8_192;
/// What one image a chat model reads comes to, in tokens: about a megapixel, as Claude reads
/// its largest at about 1,600.
const IMAGE_READ_TOKENS: u64 = 1_600;
/// What one page of a PDF reads, in tokens: its text and an image of it, as Anthropic's PDF
/// guide puts a page at 1,500 to 3,000.
const PDF_PAGE_TOKENS: u64 = 1_500;
/// The pages a PDF is counted at, at most. Its pages can't be counted without reading the whole
/// file, so its size counts, as if it were text, up to this many pages: a scanned PDF of a few
/// pages is megabytes, and never holds what its bytes would say.
const PDF_MOST_PAGES: u64 = 20;

/// What a request needs from the target that serves it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Needs<'a> {
    /// Bytes of everything the model reads but its files: system prompt, conversation and tools.
    pub(crate) bytes: u64,
    /// Images sent inline. Not part of whether a request fits, which a provider judges better,
    /// only of what it may cost.
    inline_images: u64,
    /// Tokens of the PDFs sent inline, as [`pdf_tokens`] counts them.
    pdfs: u64,
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
            inline_images: 0,
            pdfs: 0,
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
                    needs.add_parts(request, parts);
                }
                Message::Assistant { parts, tool_calls } => {
                    needs.add_parts(request, parts);
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

    fn add_parts(&mut self, request: &ModelRequest, parts: &'a [Part]) {
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
                Part::File { .. } => {
                    match request.inline(part) {
                        Some(file) if file.is_pdf() => {
                            self.pdfs = self.pdfs.saturating_add(pdf_tokens(file));
                        }
                        Some(_) => self.inline_images = self.inline_images.saturating_add(1),
                        // Sent by its name alone, which is little.
                        None => {}
                    }
                    0
                }
            };
            self.bytes = self.bytes.saturating_add(bytes);
        }
    }

    /// The request's input, estimated generously: whether it fits a window.
    pub(crate) fn fit_tokens(&self) -> u64 {
        self.bytes.div_ceil(FIT_BYTES_PER_TOKEN)
    }

    /// The request's input but its files, estimated cautiously: how much room its answer has.
    pub(crate) fn reserve_tokens(&self) -> u64 {
        self.bytes.div_ceil(RESERVE_BYTES_PER_TOKEN)
    }

    /// The request's input, its files' too, estimated cautiously: what a call's worst case
    /// holds, each image at the most an image model reads of one when `media`.
    pub(crate) fn held_tokens(&self, media: bool) -> u64 {
        let image = if media {
            IMAGE_IN_TOKENS
        } else {
            IMAGE_READ_TOKENS
        };
        self.reserve_tokens()
            .saturating_add(self.inline_images.saturating_mul(image))
            .saturating_add(self.pdfs)
    }

    /// The request's input, its files' too, estimated generously: what a call is charged for
    /// reading when its provider says nothing of what it read.
    pub(crate) fn read_tokens(&self) -> u64 {
        self.fit_tokens()
            .saturating_add(self.inline_images.saturating_mul(IMAGE_READ_TOKENS))
            .saturating_add(self.pdfs)
    }
}

/// What a PDF sent inline reads: its bytes as text, but no more than [`PDF_MOST_PAGES`] pages.
fn pdf_tokens(file: &InlineFile) -> u64 {
    // Base64 holds three bytes in every four characters.
    let bytes = len(&file.data).saturating_mul(3) / 4;
    bytes
        .div_ceil(RESERVE_BYTES_PER_TOKEN)
        .min(PDF_MOST_PAGES * PDF_PAGE_TOKENS)
}

fn len(text: &str) -> u64 {
    u64::try_from(text.len()).unwrap_or(u64::MAX)
}
