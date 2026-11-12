//! OpenRouter's image API, for glave's image tools (`specs/gateway.md`, "Media tools"): one
//! request (`POST {base}/images`) with the prompt, the images the call names as references,
//! the shape asked for and, at OpenRouter, the providers the person's filters leave; one JSON
//! answer with each image in base64 and what it cost. No standard covers making images, and
//! the image models glave defaults to are served by nothing else at OpenRouter.

use glave_event::{ModelRef, Part, StopReason};
use glave_ports::{
    Completion, Failure, InlineFile, Made, MadeFile, MediaAsk, Message, ModelRequest,
    ProviderError, Route,
};
use serde::{Deserialize, Serialize};
use serde_json::value::RawValue;

use crate::assemble::usage_and_cost;
use crate::client::OpenAiCompatible;
use crate::wire::sort_name;

/// The largest answer read: a few images of several megapixels, in base64.
const MOST_ANSWER: usize = 128 << 20;

/// The request, as OpenRouter's image API takes it: one image at a time.
#[derive(Debug, Serialize)]
pub(crate) struct ImagesRequest<'a> {
    model: &'a str,
    prompt: String,
    n: u8,
    #[serde(skip_serializing_if = "Option::is_none")]
    aspect_ratio: Option<&'a str>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    input_references: Vec<Reference>,
    #[serde(skip_serializing_if = "Option::is_none")]
    provider: Option<Providers<'a>>,
}

/// An image the call draws on, as a `data:` URL.
#[derive(Debug, Serialize)]
struct Reference {
    #[serde(rename = "type")]
    kind: &'static str,
    image_url: Url,
}

#[derive(Debug, Serialize)]
struct Url {
    url: String,
}

/// The providers it may go to: the filters the image API takes.
#[derive(Debug, Serialize)]
struct Providers<'a> {
    #[serde(skip_serializing_if = "Option::is_none")]
    only: Option<&'a [String]>,
    #[serde(skip_serializing_if = "<[String]>::is_empty")]
    ignore: &'a [String],
    #[serde(skip_serializing_if = "Option::is_none")]
    sort: Option<&'static str>,
}

/// The answer: each image made, and the call's usage with what it cost.
#[derive(Debug, Deserialize)]
struct ImagesAnswer {
    #[serde(default)]
    data: Vec<Image>,
    #[serde(default)]
    usage: Option<Box<RawValue>>,
}

#[derive(Debug, Deserialize)]
struct Image {
    b64_json: String,
    #[serde(default)]
    media_type: Option<String>,
}

impl OpenAiCompatible {
    /// Makes the image `request` asks, `ask` beside it, through the image API.
    pub(crate) async fn images(
        &self,
        request: &ModelRequest,
        ask: &MediaAsk,
    ) -> Result<Made, ProviderError> {
        let body = serde_json::to_vec(&images_request(request, ask)?)
            .map_err(|e| ProviderError::Protocol(format!("could not encode the request: {e}")))?;
        let url = format!("{}/images", self.base);
        let response = self.post(&url, body).await?;
        let answer = self.whole(response, &url, MOST_ANSWER).await?;
        let answer: ImagesAnswer = serde_json::from_slice(&answer)
            .map_err(|e| ProviderError::Protocol(format!("{url} did not answer with JSON: {e}")))?;
        if answer.data.is_empty() {
            return Err(ProviderError::NoOutput(
                "the answer held no image".to_owned(),
            ));
        }
        let (usage, cost) = usage_and_cost(answer.usage.as_deref(), false, &self.provider)?;
        let files = answer
            .data
            .into_iter()
            .map(|image| MadeFile {
                media_type: image.media_type,
                data: image.b64_json,
            })
            .collect();
        let completion = Completion {
            model: ModelRef {
                provider: self.provider.clone(),
                name: request.model.clone(),
            },
            response_id: None,
            parts: Vec::new(),
            tool_calls: Vec::new(),
            stop: StopReason::EndTurn,
            provider_stop: None,
            usage,
            cost,
            served: None,
            upstream: None,
        };
        Ok(Made { files, completion })
    }
}

/// `request` as the image API takes it: the text of its message as the prompt, and its images
/// as references, in order. Filters it can't be told (precision, price, keeping nothing) are
/// refused, so a call never goes out past them.
pub(crate) fn images_request<'a>(
    request: &'a ModelRequest,
    ask: &'a MediaAsk,
) -> Result<ImagesRequest<'a>, ProviderError> {
    let mut prompt = String::new();
    let mut input_references = Vec::new();
    for part in request.messages.iter().flat_map(parts) {
        match part {
            Part::Text { text } => prompt.push_str(text),
            Part::File { .. } => {
                if let Some(file) = request.inline(part) {
                    input_references.push(reference(file));
                }
            }
            Part::Reasoning { .. } => {}
        }
    }
    Ok(ImagesRequest {
        model: &request.model,
        prompt,
        n: 1,
        aspect_ratio: ask.aspect_ratio.as_deref(),
        input_references,
        provider: request.route.as_ref().map(providers).transpose()?,
    })
}

/// The parts of a person's message; none of anything else.
fn parts(message: &Message) -> &[Part] {
    match message {
        Message::User { parts } => parts,
        Message::Assistant { .. } | Message::Tool { .. } => &[],
    }
}

fn reference(file: &InlineFile) -> Reference {
    Reference {
        kind: "image_url",
        image_url: Url {
            url: format!("data:{};base64,{}", file.media_type, file.data),
        },
    }
}

/// What `route` asks of the providers, as the image API takes it.
fn providers(route: &Route) -> Result<Providers<'_>, ProviderError> {
    if route.zdr || !route.quantizations.is_empty() || route.max_price.is_some() {
        return Err(ProviderError::Unserved {
            failure: Failure::Rejected,
            reason: "OpenRouter's image models can't be held to filters on precision, price or \
                     what providers keep: loosen them to make images"
                .to_owned(),
        });
    }
    Ok(Providers {
        only: route.only.as_deref(),
        ignore: &route.ignore,
        sort: route.sort.map(sort_name),
    })
}

#[cfg(test)]
mod tests;
