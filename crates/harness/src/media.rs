//! glave's media tools in the harness (`specs/gateway.md`, "Media tools"): `image_generate`
//! and `image_edit`, offered where the model provider serves them and run by the harness itself,
//! as a call of the provider's ([`ModelProvider::make`]): its arguments and the images it sends
//! read and checked before anyone is asked, then held and charged as the thread's work,
//! recorded as a model call is, and each image made kept whole as a sealed blob of the thread's
//! with a copy fit to send, so the model sees what it made and a client draws it. A call ends
//! with its provider's answer or its timeout; one that ends unanswered, or is stopped once sent,
//! may still be made and billed, and the model is told not to make it again unasked.

mod arguments;
mod inputs;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use glave_event::{BlobRef, Part};
use glave_ports::{
    BlobStore, Effect, EventLog, Message, ModelProvider, ModelRequest, Pieces, Purpose, ToolCall,
    ToolChoice, ToolOutput,
};
use serde_json::json;

use crate::approval::{ApprovalRequest, Approver, ask_of};
use crate::cancel::Cancel;
use crate::files::kind_of;
use crate::harness::{Harness, HarnessError, Thread};
use crate::hooks::ToolHook;
use crate::turn::{Live, Step};

use arguments::{Asked, asked, purpose};
pub(crate) use arguments::{IMAGE_EDIT, IMAGE_GENERATE, describe, spec};
use inputs::Inputs;

/// The largest image kept, in bytes.
const MOST_MADE: u64 = 64 * 1024 * 1024;

/// The longest stem of an image's name, in characters.
const LONGEST_STEM: usize = 40;

/// What the model is told of a call that may still be made and billed.
const MAY_BE_BILLED: &str = "It may still be made and billed: don't make it again unless the \
     person asks.";

impl<P, L, B, A> Harness<P, L, B, A>
where
    P: ModelProvider,
    L: EventLog,
    B: BlobStore,
    A: Approver,
{
    /// Runs a call of a media tool: its arguments and the images it sends checked first, then
    /// asked about as `web_fetch` is, since it sends words and images beyond the computer and
    /// spends money, its line naming each file it sends, then made through the provider.
    pub(crate) async fn call_media(
        &self,
        thread: &mut Thread,
        call: &ToolCall,
        step: &mut Step,
        cancel: &Cancel,
        live: &mut (dyn FnMut(Live) + Send),
    ) -> Result<ToolOutput, HarnessError> {
        let Some(purpose) = purpose(&call.tool) else {
            return Ok(ToolOutput::text(true, format!("{:?} is gone", call.tool)));
        };
        let checked = match asked(purpose, &call.arguments) {
            Ok(asked) => self
                .inputs(thread, asked.images.iter())
                .await
                .map(|inputs| (asked, inputs)),
            Err(wrong) => Err(wrong),
        };
        let (asked, inputs) = match checked {
            Ok(checked) => checked,
            Err(wrong) => return Ok(ToolOutput::text(true, wrong)),
        };
        let name = purpose.id();
        let summary = describe(name, &call.arguments);
        let hooked = ToolHook::of(call, name);
        let refused = if let Some(answered) = step.waited(&call.call_id) {
            // Its hooks ran and it asked before the turn stopped: its answer decides.
            self.answered(thread, answered, cancel, live).await?
        } else if let Some(stopped) =
            (self.stopped_by_hook(thread, &hooked, &summary, cancel, live)).await?
        {
            Some(stopped)
        } else {
            let request = ApprovalRequest {
                thread: thread.id,
                call_id: call.call_id.clone(),
                tool: name.to_owned(),
                effect: Effect::Runs,
                summary: summary.clone(),
                ask: ask_of(name),
                secret: false,
                caution: None,
                outside_sandbox: None,
            };
            let turn = &mut step.summary;
            self.approve(thread, &request, turn, cancel, live).await?
        };
        if let Some(refused) = refused {
            return Ok(refused);
        }
        self.record_started(thread, &call.call_id).await?;
        live(Live::ToolStarted {
            summary: summary.clone(),
        });
        let made = (purpose, asked, inputs);
        let output = self.made(thread, made, step, cancel).await?;
        live(Live::ToolFinished {
            summary,
            is_error: output.is_error,
        });
        self.after_tool(thread, &hooked, output, cancel).await
    }

    /// What the provider made for `asked`, with `inputs`, kept for `thread`, as the model reads
    /// it, the call recorded as a model call of the turn's.
    async fn made(
        &self,
        thread: &mut Thread,
        (purpose, asked, inputs): (Purpose, Asked, Inputs),
        step: &mut Step,
        cancel: &Cancel,
    ) -> Result<ToolOutput, HarnessError> {
        let named = stem(&asked.prompt);
        let mut parts = vec![Part::Text { text: asked.prompt }];
        parts.extend(inputs.parts);
        let request = ModelRequest {
            model: String::new(),
            system: String::new(),
            messages: vec![Message::User { parts }],
            tools: Vec::new(),
            tool_choice: ToolChoice::None,
            max_output_tokens: None,
            purpose,
            effort: None,
            files: inputs.files,
            payer: Some(thread.payer()),
            route: None,
        };
        let Some(made) = cancel.run(self.provider.make(&request, &asked.ask)).await else {
            let said = format!("Stopped once it may have been sent. {MAY_BE_BILLED}");
            return Ok(ToolOutput::text(true, said));
        };
        let made = match made {
            Ok(made) => made,
            Err(error) if error.unanswered() => {
                let said = format!("No image came back: {error}. {MAY_BE_BILLED}");
                return Ok(ToolOutput::text(true, said));
            }
            Err(error) => {
                let said = format!("No image was made: {error}");
                return Ok(ToolOutput::text(true, said));
            }
        };
        let responded = made.completion.responded();
        step.summary.model_calls = step.summary.model_calls.saturating_add(1);
        step.summary.take_in(&responded);
        self.record(thread, &responded).await?;
        let mut files = Vec::new();
        let mut shown = Vec::new();
        for file in made.files {
            let said = file.media_type.as_deref();
            match self.keep_made(thread, said, &file.data, &named).await {
                Ok(part) => {
                    if let Part::File { blob, .. } = &part {
                        files.push(json!({
                            "file": format!("blob:{}", blob.sha256.as_str()),
                            "media_type": blob.media_type,
                        }));
                    }
                    shown.push(part);
                }
                Err(wrong) => shown.push(Part::Text { text: wrong }),
            }
        }
        if files.is_empty() {
            return Ok(ToolOutput {
                parts: shown,
                ..ToolOutput::text(true, "No image came back, and the call was billed as made.")
            });
        }
        let mut output = ToolOutput::text(false, json!({ "files": files }).to_string());
        output.parts.extend(shown);
        Ok(output)
    }

    /// The image the provider made, `data` in base64 of what it said is `said`, kept whole for
    /// `thread` as `stem` with a copy fit to send beside it: its `file` part, or why it was not
    /// kept.
    async fn keep_made(
        &self,
        thread: &Thread,
        said: Option<&str>,
        data: &str,
        stem: &str,
    ) -> Result<Part, String> {
        let bytes = STANDARD
            .decode(data.trim())
            .map_err(|_| "(an image came back unreadable)".to_owned())?;
        let media_type = kind_of(said.unwrap_or_default(), &bytes)
            .filter(|kind| glave_ports::is_image(kind))
            .ok_or_else(|| "(what came back was not an image glave reads)".to_owned())?;
        // The copy the model sees, made from what is here, however large the image.
        let (bytes, copy) = self.fit(bytes).await;
        let blob = self
            .keep_image(thread, &media_type, bytes)
            .await
            .map_err(|error| format!("(the image could not be kept: {error})"))?;
        let resized = match copy {
            Ok(Some(copy)) => self.keep_image(thread, "image/jpeg", copy).await.ok(),
            Ok(None) | Err(_) => None,
        };
        Ok(Part::File {
            name: Some(format!("{stem}.{}", extension(&media_type))),
            blob,
            resized,
        })
    }

    /// `bytes` of `media_type`, kept for `thread`.
    async fn keep_image(
        &self,
        thread: &Thread,
        media_type: &str,
        bytes: Vec<u8>,
    ) -> Result<BlobRef, glave_ports::BlobError> {
        let pieces = Box::new(Pieces::from(bytes));
        let put = self.blobs.put(
            &thread.scope,
            Some(thread.id),
            media_type,
            MOST_MADE,
            pieces,
        );
        put.await
    }
}

/// The file extension for an image of `media_type`: `png` for `image/png`.
fn extension(media_type: &str) -> &str {
    media_type.rsplit('/').next().unwrap_or("png")
}

/// The stem an image made for `prompt` is saved as: its first words, in plain letters and
/// digits joined by hyphens, `image` when it has none.
fn stem(prompt: &str) -> String {
    let mut stem = String::new();
    for word in prompt.split_whitespace() {
        let word: String = word
            .chars()
            .filter(char::is_ascii_alphanumeric)
            .map(|c| c.to_ascii_lowercase())
            .collect();
        if word.is_empty() {
            continue;
        }
        if stem.len() + word.len() + 1 > LONGEST_STEM {
            break;
        }
        if !stem.is_empty() {
            stem.push('-');
        }
        stem.push_str(&word);
    }
    if stem.is_empty() {
        "image".to_owned()
    } else {
        stem
    }
}
