//! The model provider port: send a conversation, stream the answer live, get the finished
//! answer back. Providers translate their own wire formats at this edge.

use std::future::Future;

use glave_event::{
    Arguments, CallId, Cost, ModelRef, ModelResponded, Money, Part, PrincipalId, Sha256,
    StopReason, Usage,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;

use crate::failure::Failure;
use crate::scope::OrgId;

/// A model that can answer a conversation.
pub trait ModelProvider: Send + Sync {
    /// Sends one request and streams the answer. Live fragments go to `on_delta` as they
    /// arrive and are never stored; the finished answer is returned. An error once any output
    /// came (text, reasoning or a tool call) is [`ProviderError::AfterOutput`], so nobody
    /// asks again and shows a second answer after the first one's start.
    fn complete(
        &self,
        request: &ModelRequest,
        on_delta: &mut (dyn FnMut(Delta) + Send),
    ) -> impl Future<Output = Result<Completion, ProviderError>> + Send;
}

/// One request: the system prompt, the conversation so far, and the tools on offer.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModelRequest {
    pub model: String,
    pub system: String,
    pub messages: Vec<Message>,
    pub tools: Vec<ToolSpec>,
    /// Whether the model may call the tools on offer. They stay listed either way, since some
    /// providers refuse a conversation that has tool calls but no tools.
    #[serde(default, skip_serializing_if = "ToolChoice::is_auto")]
    pub tool_choice: ToolChoice,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_output_tokens: Option<u32>,
    /// What the call is for, so a gateway can choose the model for it. Providers ignore it.
    #[serde(default, skip_serializing_if = "Purpose::is_main")]
    pub purpose: Purpose,
    /// How much the model is asked to reason: a level its provider names, such as `high` or
    /// `none`, one the model offers. None leaves it to the model. A gateway sends each target
    /// only a level it offers, and one asked for none, or for a level it does not offer, the
    /// level its model uses unasked when its catalog says; a provider that has no way to send
    /// it leaves it out.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effort: Option<String>,
    /// The content of the images and PDFs the messages and tool results hold, for the provider
    /// to send inline where a [`Part::File`] names them; a file not here is sent by its name
    /// alone.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub files: Vec<InlineFile>,
    /// Whose work the call is, so a gateway that keeps organizations' credits charges it
    /// there. None is glave's own, charged to no one. Providers ignore it, and it is never
    /// sent.
    #[serde(skip)]
    pub payer: Option<Payer>,
}

/// Whose work a model call is: the organization whose credits pay for it, and the person it
/// works for, when it works for anyone.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Payer {
    pub org: OrgId,
    pub person: Option<PrincipalId>,
}

/// The image types every provider glave speaks to reads inline.
const IMAGE_TYPES: [&str; 4] = ["image/png", "image/jpeg", "image/gif", "image/webp"];

/// A PDF's media type: providers read one as its text and an image of each page.
pub const PDF: &str = "application/pdf";

/// Whether a file of `media_type` is an image every provider reads inline.
pub fn is_image(media_type: &str) -> bool {
    IMAGE_TYPES.contains(&media_type)
}

/// Whether a file of `media_type` is sent to a model inline, which only a model that reads
/// images reads: an image, or a PDF.
pub fn is_inline(media_type: &str) -> bool {
    is_image(media_type) || media_type == PDF
}

/// An image or a PDF a message or a tool result holds, its content ready to send inline.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InlineFile {
    /// The blob it is, as [`Part::sent`] names it.
    pub sha256: Sha256,
    /// Its media type, one [`is_inline`] takes.
    pub media_type: String,
    /// Its bytes, in base64.
    pub data: String,
}

impl InlineFile {
    /// Whether it is a PDF, which providers take as a document rather than an image.
    pub fn is_pdf(&self) -> bool {
        self.media_type == PDF
    }
}

impl ModelRequest {
    /// The content of the file `part` names, when it is sent inline.
    pub fn inline(&self, part: &Part) -> Option<&InlineFile> {
        let sent = part.sent()?;
        self.files.iter().find(|file| file.sha256 == sent.sha256)
    }
}

/// What a model call is for. A stable public vocabulary: gateway routes are named by it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Purpose {
    /// The agent's own turn.
    #[default]
    Main,
    /// Deciding whether a flagged action the person asked for may run (auto mode).
    Judge,
    /// A compaction's handoff summary.
    Summary,
    /// A subagent's turn.
    Subagent,
    /// A thread's short title, after its first exchange.
    Title,
}

impl Purpose {
    /// The purpose's stable id: `main`, `judge`, `summary`, `subagent` or `title`.
    pub fn id(self) -> &'static str {
        match self {
            Self::Main => "main",
            Self::Judge => "judge",
            Self::Summary => "summary",
            Self::Subagent => "subagent",
            Self::Title => "title",
        }
    }

    #[expect(
        clippy::trivially_copy_pass_by_ref,
        reason = "serde's `skip_serializing_if` passes the field by reference"
    )]
    pub(crate) fn is_main(&self) -> bool {
        *self == Self::Main
    }
}

/// Whether a response may call tools.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolChoice {
    /// The model decides.
    #[default]
    Auto,
    /// Text only: no tool calls.
    None,
}

impl ToolChoice {
    #[expect(
        clippy::trivially_copy_pass_by_ref,
        reason = "serde's `skip_serializing_if` passes the field by reference"
    )]
    pub(crate) fn is_auto(&self) -> bool {
        *self == Self::Auto
    }
}

/// One message of the conversation, in provider-neutral form.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "role", rename_all = "snake_case")]
pub enum Message {
    User {
        parts: Vec<Part>,
    },
    Assistant {
        parts: Vec<Part>,
        tool_calls: Vec<ToolCall>,
    },
    Tool {
        call_id: CallId,
        is_error: bool,
        output: Vec<Part>,
    },
}

/// A tool call already in the conversation, under its glave call id.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolCall {
    pub call_id: CallId,
    pub tool: String,
    pub arguments: Arguments,
}

/// A tool offered to the model: its name, what it does, and a JSON Schema for its arguments.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolSpec {
    pub name: String,
    pub description: String,
    pub parameters: Value,
}

/// A live fragment of an answer. Shown as it arrives, never stored.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Delta {
    Text(String),
    Reasoning(String),
}

/// A finished answer.
#[derive(Debug, Clone, PartialEq)]
pub struct Completion {
    pub model: ModelRef,
    pub response_id: Option<String>,
    /// Text and reasoning, in the order the model produced them.
    pub parts: Vec<Part>,
    pub tool_calls: Vec<RequestedCall>,
    pub stop: StopReason,
    pub provider_stop: Option<String>,
    pub usage: Usage,
    /// The cost the provider itself reported, when it reports one (OpenRouter does), or a
    /// gateway computed from a configured price.
    pub cost: Option<Cost>,
    /// How a gateway served the call; `None` when no gateway was in between.
    pub served: Option<Served>,
    /// The provider a service that routes calls (OpenRouter) passed this one on to, as it
    /// names it, such as `Anthropic`; `None` when it says none.
    pub upstream: Option<String>,
}

impl Completion {
    /// The content-free record of the call, `model.responded`: with the gateway route,
    /// target and service that served it when a gateway did, and where a service that routes
    /// calls passed it on to.
    pub fn responded(&self) -> ModelResponded {
        let served = self.served.as_ref();
        ModelResponded {
            model: self.model.clone(),
            response_id: self.response_id.clone(),
            stop: self.stop,
            provider_stop: self.provider_stop.clone(),
            usage: self.usage.clone(),
            cost: self.cost.clone(),
            route: served.map(|served| served.route.clone()),
            target: served.map(|served| served.target.clone()),
            service: served.and_then(|served| served.service.clone()),
            upstream: self.upstream.clone(),
            effort: served.and_then(|served| served.effort.clone()),
            reasoning_stripped: served.is_some_and(|served| served.reasoning_stripped),
        }
    }
}

/// How a gateway served a call: the route it was sent by and the target that answered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Served {
    /// The route's id: the call's purpose, such as `main` or `judge`.
    pub route: String,
    /// The name of the target that answered.
    pub target: String,
    /// The service the target is at, in a person's words, such as `OpenRouter`, when the
    /// gateway knows it.
    pub service: Option<String>,
    /// The reasoning level the target was sent, if any: the one asked for, or its model's
    /// default.
    pub effort: Option<String>,
    /// True when opaque reasoning was left out of the request because the target that
    /// answered could not accept it.
    pub reasoning_stripped: bool,
}

/// A tool call the model just asked for. The harness gives it a glave call id when it
/// records it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RequestedCall {
    pub provider_call_id: Option<String>,
    pub tool: String,
    pub arguments: Arguments,
}

/// Why a model request failed.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ProviderError {
    /// The provider answered with an error status.
    #[error("the model provider returned HTTP {status}: {body}")]
    Http { status: u16, body: String },
    /// Credentials for the provider could not be had: a token file missing, or an identity
    /// provider that refused. Never holds a secret.
    #[error("could not get credentials for the model provider: {0}")]
    Credentials(String),
    /// The provider could not be reached, or the connection broke.
    #[error("could not reach the model provider: {0}")]
    Transport(String),
    /// The provider's response did not follow its own protocol.
    #[error("the model provider sent a malformed response: {0}")]
    Protocol(String),
    /// The response ended in an error before any text, reasoning or tool call: nothing was
    /// shown or run, so asking again is safe.
    #[error("the model provider ended the response in an error before any output: {0}")]
    NoOutput(String),
    /// The response broke off, for the reason inside, after text, reasoning or a tool call had
    /// come: never transient, since asking again would show that output twice.
    #[error("the model provider's answer broke off partway: {0}")]
    AfterOutput(Box<ProviderError>),
    /// The provider sent nothing for this long, before or during its answer.
    #[error("the model provider sent nothing for {seconds} s")]
    Timeout { seconds: u64 },
    /// A gateway sent nothing: no target could serve the request. `failure` says why, in the
    /// terms a provider's own refusal would have used.
    #[error("no model could serve the request: {reason}")]
    Unserved { failure: Failure, reason: String },
    /// A gateway sent nothing: the call could exceed this budget.
    #[error("the {budget} budget would be exceeded: {detail}")]
    OverBudget { budget: String, detail: String },
    /// A gateway sent nothing: the organization's credits could not pay for the call.
    #[error(transparent)]
    Unpaid(Unpaid),
}

/// Why an organization's credits could not pay for a call (`specs/gateway.md`, "Credits"),
/// in the words its people are told.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum Unpaid {
    /// Nothing is left: what was added, less what was charged and what calls running hold.
    #[error("you're out of credits: add credits to keep going")]
    Spent,
    /// Something is left, but less than the call could cost.
    #[error(
        "this could cost up to {needs:.2}, more than the {left:.2} of credits left: add \
         credits, or choose a cheaper model"
    )]
    Short { left: Money, needs: Money },
    /// Nothing is left of the most it may spend this month.
    #[error(
        "this month's spending limit of {limit:.2} is reached: the account's owner can raise it"
    )]
    MonthLimit { limit: Money },
    /// Something is left of this month's limit, but less than the call could cost.
    #[error(
        "this could cost up to {needs:.2}, more than the {left:.2} left of this month's \
         {limit:.2} limit: the account's owner can raise it, or choose a cheaper model"
    )]
    MonthShort {
        limit: Money,
        left: Money,
        needs: Money,
    },
    /// No catalog says what the model costs, so no one can say what the call could.
    #[error("{model} has no price here, so credits can't pay for it: choose another model")]
    Unpriced { model: String },
}

/// An event stream that cannot be read is a response that does not follow its protocol.
impl From<crate::SseError> for ProviderError {
    fn from(error: crate::SseError) -> Self {
        Self::Protocol(error.to_string())
    }
}
