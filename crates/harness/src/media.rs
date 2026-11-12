//! glave's media tools in the harness (`specs/gateway.md`, "Media tools"): `image_generate`
//! and `image_edit`, offered where the model provider serves them and run by the harness itself,
//! as a call of the provider's ([`ModelProvider::make`]): held and charged as the thread's work,
//! recorded as a model call is, and each image made kept whole as a sealed blob of the thread's
//! with a copy fit to send, so the model sees what it made and a client draws it.

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

use crate::approval::{ApprovalRequest, Approver};
use crate::calls::{NOT_RUN, Needs};
use crate::cancel::Cancel;
use crate::files::kind_of;
use crate::harness::{Harness, HarnessError, Thread};
use crate::hooks::ToolHook;
use crate::turn::{Live, Step};

use arguments::{Asked, asked, purpose};
pub(crate) use arguments::{IMAGE_EDIT, IMAGE_GENERATE, describe, spec};

/// The tokens one image is held at where its price is per token: a four-megapixel image's, one
/// per 16×16 pixels.
const TOKENS_PER_IMAGE: u32 = 16_384;

/// The largest image kept, in bytes.
const MOST_MADE: u64 = 64 * 1024 * 1024;

impl<P, L, B, A> Harness<P, L, B, A>
where
    P: ModelProvider,
    L: EventLog,
    B: BlobStore,
    A: Approver,
{
    /// Runs a call of a media tool: asked about as `web_fetch` is, since it sends words and
    /// images beyond the computer and spends money, then made through the provider.
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
                secret: false,
                caution: None,
                outside_sandbox: None,
            };
            let needs = Needs {
                request,
                firm: false,
            };
            let turn = &mut step.summary;
            self.approve(thread, &needs, turn, cancel, live).await?
        };
        if let Some(refused) = refused {
            return Ok(refused);
        }
        self.record_started(thread, &call.call_id).await?;
        live(Live::ToolStarted {
            summary: summary.clone(),
        });
        let output = match asked(purpose, &call.arguments) {
            Ok(asked) => self.made(thread, purpose, asked, step, cancel).await?,
            Err(wrong) => ToolOutput::text(true, wrong),
        };
        live(Live::ToolFinished {
            summary,
            is_error: output.is_error,
        });
        self.after_tool(thread, &hooked, output, cancel).await
    }

    /// What the provider made for `asked`, kept for `thread`, as the model reads it, the call
    /// recorded as a model call of the turn's.
    async fn made(
        &self,
        thread: &mut Thread,
        purpose: Purpose,
        asked: Asked,
        step: &mut Step,
        cancel: &Cancel,
    ) -> Result<ToolOutput, HarnessError> {
        let inputs = match self.inputs(thread, asked.images.iter()).await {
            Ok(inputs) => inputs,
            Err(wrong) => return Ok(ToolOutput::text(true, wrong)),
        };
        let mut parts = vec![Part::Text { text: asked.prompt }];
        parts.extend(inputs.parts);
        let request = ModelRequest {
            model: String::new(),
            system: String::new(),
            messages: vec![Message::User { parts }],
            tools: Vec::new(),
            tool_choice: ToolChoice::None,
            max_output_tokens: Some(TOKENS_PER_IMAGE),
            purpose,
            effort: None,
            files: inputs.files,
            payer: Some(thread.payer()),
            route: None,
        };
        let ask = asked.ask;
        let Some(made) = cancel.run(self.provider.make(&request, &ask)).await else {
            return Ok(ToolOutput::text(true, NOT_RUN));
        };
        let made = match made {
            Ok(made) => made,
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
            match self
                .keep_made(thread, file.media_type.as_deref(), &file.data)
                .await
            {
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
                ..ToolOutput::text(true, "No image was kept.")
            });
        }
        let mut output = ToolOutput::text(false, json!({ "files": files }).to_string());
        output.parts.extend(shown);
        Ok(output)
    }

    /// The image the provider made, `data` in base64 of what it said is `said`, kept whole for
    /// `thread` with a copy fit to send beside it: its `file` part, or why it was not kept.
    async fn keep_made(
        &self,
        thread: &Thread,
        said: Option<&str>,
        data: &str,
    ) -> Result<Part, String> {
        let bytes = STANDARD
            .decode(data.trim())
            .map_err(|_| "(an image came back unreadable)".to_owned())?;
        let media_type = kind_of(said.unwrap_or_default(), &bytes)
            .filter(|kind| glave_ports::is_image(kind))
            .ok_or_else(|| "(what came back was not an image glave reads)".to_owned())?;
        let kept = self.blobs.put(
            &thread.scope,
            Some(thread.id),
            &media_type,
            MOST_MADE,
            Box::new(Pieces::from(bytes)),
        );
        let blob = kept
            .await
            .map_err(|error| format!("(the image could not be kept: {error})"))?;
        let resized = self.fit_copy(thread, &blob).await;
        Ok(Part::File {
            name: Some(format!("image.{}", extension(&media_type))),
            blob,
            resized,
        })
    }

    /// The copy of the image `blob` the model is sent, kept for `thread`: none when it is sent
    /// as it is, or when no copy could be made, and then the image is sent by its name alone.
    async fn fit_copy(&self, thread: &Thread, blob: &BlobRef) -> Option<BlobRef> {
        let copy = self.resized(&thread.scope, blob).await.ok()??;
        let referred = self.blobs.refer(&thread.scope, &copy.sha256, thread.id);
        referred.await.ok()?.then_some(copy)
    }
}

/// The file extension for an image of `media_type`: `png` for `image/png`.
fn extension(media_type: &str) -> &str {
    media_type.rsplit('/').next().unwrap_or("png")
}
