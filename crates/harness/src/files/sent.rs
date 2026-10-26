//! The files in a thread made ready for each model call (`specs/session.md`, "Attachments"):
//! an image or a PDF sent inline, where the provider puts it beside the text, the newest first
//! and within what one request holds; a text file as its text, cut at [`MAX_TEXT`]; anything
//! else, and what is past the limits, by its name. A model that reads no images is told each
//! is not shown. The bytes are read from the blob store at each call, so nothing is held
//! between them.

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use glave_event::{BlobRef, Part};
use glave_ports::{BlobStore, EventLog, InlineFile, Message, ModelProvider, PDF, Scope, is_inline};

use super::fit::MAX_IMAGE;
use super::{MAX_FILE, is_text};
use crate::approval::Approver;
use crate::harness::Harness;

/// The most of a text file the model reads, in bytes; the rest is said to be cut.
const MAX_TEXT: u64 = 256 * 1024;

/// The most files one request sends inline; older ones go by their names. A bound on the
/// request rather than a provider's rule: Claude takes 100 or more, each within 2,000 px.
const MAX_INLINE: usize = 20;

/// The most bytes they hold together: in base64, with the conversation, within Anthropic's
/// 32 MB request.
const MAX_INLINE_BYTES: u64 = 20 * 1024 * 1024;

impl<P, L, B, A> Harness<P, L, B, A>
where
    P: ModelProvider,
    L: EventLog,
    B: BlobStore,
    A: Approver,
{
    /// `messages` made ready for `model`: text files as their text, and the images and PDFs
    /// to send inline beside them, for a model that reads them. A file that cannot be read is
    /// sent by its name.
    pub(crate) async fn with_files(
        &self,
        scope: &Scope,
        model: &str,
        mut messages: Vec<Message>,
    ) -> (Vec<Message>, Vec<InlineFile>) {
        let sees = self.reads_images(model);
        let mut files = Vec::new();
        if sees {
            for blob in inlined(&messages) {
                files.extend(self.inline(scope, &blob).await);
            }
        }
        for message in &mut messages {
            let (Message::User { parts } | Message::Tool { output: parts, .. }) = message else {
                continue;
            };
            for part in parts.iter_mut() {
                let Some(sent) = part.sent().cloned() else {
                    continue;
                };
                let Part::File { name, .. } = part else {
                    continue;
                };
                let name = name.clone().unwrap_or_else(|| "a file".to_owned());
                if is_inline(&sent.media_type) {
                    if !sees {
                        let text = format!("[{name} is not shown: this model reads no images]");
                        *part = Part::Text { text };
                    }
                } else if is_text(&sent.media_type)
                    && let Some(text) = self.text(scope, &sent, &name).await
                {
                    *part = Part::Text { text };
                }
            }
        }
        (messages, files)
    }

    /// The image or PDF `blob` holds, in base64; `None` when it cannot be read.
    async fn inline(&self, scope: &Scope, blob: &BlobRef) -> Option<InlineFile> {
        let bytes = self.read(scope, blob, blob.bytes).await?;
        Some(InlineFile {
            sha256: blob.sha256.clone(),
            media_type: blob.media_type.clone(),
            data: STANDARD.encode(bytes),
        })
    }

    /// The text file `blob` holds, named `name`, as the model reads it: its text in a `file`
    /// element, cut at [`MAX_TEXT`] with a note saying so. `None` when it is not UTF-8.
    async fn text(&self, scope: &Scope, blob: &BlobRef, name: &str) -> Option<String> {
        let bytes = self.read(scope, blob, MAX_TEXT).await?;
        let cut = blob.bytes > MAX_TEXT;
        let text = match String::from_utf8(bytes) {
            Ok(text) => text,
            // Cut inside a character: what came before it.
            Err(error) if cut => {
                let valid = error.utf8_error().valid_up_to();
                let mut bytes = error.into_bytes();
                bytes.truncate(valid);
                String::from_utf8(bytes).ok()?
            }
            Err(_) => return None,
        };
        let note = if cut {
            format!(
                "\n[cut here: the file has {} bytes, and only the first {MAX_TEXT} are shown]",
                blob.bytes
            )
        } else {
            String::new()
        };
        Some(format!("<file name=\"{name}\">\n{text}{note}\n</file>"))
    }

    /// Up to `most` bytes of `blob`; `None` when it is gone or cannot be read.
    async fn read(&self, scope: &Scope, blob: &BlobRef, most: u64) -> Option<Vec<u8>> {
        let found = self.blobs.get(scope, &blob.sha256).await;
        let mut found = match found {
            Ok(Some(found)) => found,
            Ok(None) => return None,
            Err(error) => {
                tracing::warn!(%error, "a file could not be read for the model");
                return None;
            }
        };
        let mut bytes = Vec::new();
        while let Some(chunk) = found.bytes.next_chunk().await.ok()? {
            bytes.extend_from_slice(&chunk);
            if bytes.len() as u64 >= most {
                bytes.truncate(usize::try_from(most).unwrap_or(usize::MAX));
                break;
            }
        }
        Some(bytes)
    }
}

/// The images and PDFs `messages` send inline, each once: the newest first, at most
/// [`MAX_INLINE`] of them and [`MAX_INLINE_BYTES`] together, each within what a provider takes
/// as one.
fn inlined(messages: &[Message]) -> Vec<BlobRef> {
    let parts = messages.iter().rev().flat_map(|message| {
        let parts = match message {
            Message::User { parts } | Message::Tool { output: parts, .. } => parts.as_slice(),
            Message::Assistant { .. } => &[],
        };
        parts.iter().rev()
    });
    let mut chosen: Vec<BlobRef> = Vec::new();
    let mut bytes: u64 = 0;
    for sent in parts.filter_map(Part::sent) {
        let most = if sent.media_type == PDF {
            MAX_FILE
        } else {
            MAX_IMAGE
        };
        let fits = sent.bytes <= most && bytes.saturating_add(sent.bytes) <= MAX_INLINE_BYTES;
        let again = chosen.iter().any(|blob| blob.sha256 == sent.sha256);
        if is_inline(&sent.media_type) && fits && !again {
            bytes = bytes.saturating_add(sent.bytes);
            chosen.push(sent.clone());
            if chosen.len() == MAX_INLINE {
                break;
            }
        }
    }
    chosen
}

#[cfg(test)]
mod tests {
    use glave_event::Sha256;

    use super::*;

    fn file(n: u8, media_type: &str, bytes: u64) -> Part {
        Part::File {
            blob: BlobRef {
                sha256: Sha256::from_digest([n; 32]),
                bytes,
                media_type: media_type.into(),
            },
            name: None,
            resized: None,
        }
    }

    fn shas(chosen: &[BlobRef]) -> Vec<u8> {
        chosen
            .iter()
            .map(|blob| u8::from_str_radix(blob.sha256.as_str().get(..2).unwrap(), 16).unwrap())
            .collect()
    }

    #[test]
    fn the_newest_files_go_inline_within_the_limits_and_the_rest_by_name() {
        let mut messages: Vec<Message> = (0..30)
            .map(|n| Message::User {
                parts: vec![file(n, "image/jpeg", 200_000)],
            })
            .collect();
        let chosen = inlined(&messages);
        assert_eq!(chosen.len(), MAX_INLINE);
        assert_eq!(shas(&chosen).first(), Some(&29), "the newest first");
        assert_eq!(shas(&chosen).last(), Some(&10));

        messages.push(Message::Tool {
            call_id: glave_event::CallId::new("call_1").unwrap(),
            is_error: false,
            output: vec![
                file(40, PDF, 9_000_000),
                file(41, PDF, 9_000_000),
                file(42, PDF, 9_000_000),
                file(43, "text/plain", 10),
                file(44, "image/png", MAX_IMAGE + 1),
            ],
        });
        messages.push(Message::User {
            parts: vec![file(29, "image/jpeg", 200_000)],
        });
        let chosen = inlined(&messages);
        assert_eq!(
            shas(&chosen).get(..3),
            Some([29, 42, 41].as_slice()),
            "each once; text, and an image too heavy alone, never; PDFs until 20 MB"
        );
        assert!(
            chosen.iter().map(|blob| blob.bytes).sum::<u64>() <= MAX_INLINE_BYTES,
            "within the bytes"
        );
    }
}
