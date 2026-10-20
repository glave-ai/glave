//! The files a person's messages hold, made ready for the model (`specs/session.md`,
//! "Attachments"): an image sent inline, where the provider puts it beside the text; a text
//! file as its text, cut at [`MAX_TEXT`]; anything else by its name. The bytes are read from
//! the blob store at each model call, so nothing is held between them.

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use glave_event::{BlobRef, Part};
use glave_ports::{BlobStore, EventLog, InlineFile, Message, ModelProvider, Scope};

use crate::approval::Approver;
use crate::harness::Harness;

/// The image types every provider glave speaks to reads inline.
pub(crate) const IMAGE_TYPES: [&str; 4] = ["image/png", "image/jpeg", "image/gif", "image/webp"];

/// The largest image sent inline, in bytes: what the providers take.
pub const MAX_IMAGE: u64 = 5 * 1024 * 1024;

/// The most of a text file the model reads, in bytes; the rest is said to be cut.
pub(crate) const MAX_TEXT: u64 = 256 * 1024;

/// Whether a file of `media_type` is an image the model reads inline.
pub fn is_image(media_type: &str) -> bool {
    IMAGE_TYPES.contains(&media_type)
}

/// Whether a file of `media_type` is text the model reads as text.
pub(crate) fn is_text(media_type: &str) -> bool {
    let media_type = media_type.split(';').next().unwrap_or_default().trim();
    media_type.starts_with("text/")
        || matches!(
            media_type,
            "application/json"
                | "application/xml"
                | "application/yaml"
                | "application/x-yaml"
                | "application/toml"
                | "application/javascript"
                | "application/x-sh"
                | "application/sql"
        )
        || media_type.ends_with("+json")
        || media_type.ends_with("+xml")
}

impl<P, L, B, A> Harness<P, L, B, A>
where
    P: ModelProvider,
    L: EventLog,
    B: BlobStore,
    A: Approver,
{
    /// `messages` with the files of the person's messages made ready: text files as their
    /// text, and the images to send inline beside them. A file that cannot be read is sent
    /// by its name.
    pub(crate) async fn with_files(
        &self,
        scope: &Scope,
        mut messages: Vec<Message>,
    ) -> (Vec<Message>, Vec<InlineFile>) {
        let mut files: Vec<InlineFile> = Vec::new();
        for message in &mut messages {
            let Message::User { parts } = message else {
                continue;
            };
            for part in parts.iter_mut() {
                let Part::File { blob, name } = part else {
                    continue;
                };
                if is_image(&blob.media_type) {
                    if files.iter().all(|file| file.sha256 != blob.sha256)
                        && let Some(file) = self.image(scope, blob).await
                    {
                        files.push(file);
                    }
                } else if is_text(&blob.media_type)
                    && let Some(text) = self.text(scope, blob, name.as_deref()).await
                {
                    *part = Part::Text { text };
                }
            }
        }
        (messages, files)
    }

    /// The image `blob` holds, in base64, when it can be read and is no larger than
    /// [`MAX_IMAGE`].
    async fn image(&self, scope: &Scope, blob: &BlobRef) -> Option<InlineFile> {
        if blob.bytes > MAX_IMAGE {
            return None;
        }
        let bytes = self.read(scope, blob, MAX_IMAGE).await?;
        Some(InlineFile {
            sha256: blob.sha256.clone(),
            media_type: blob.media_type.clone(),
            data: STANDARD.encode(bytes),
        })
    }

    /// The text file `blob` holds, named `name`, as the model reads it: its text in a `file`
    /// element, cut at [`MAX_TEXT`] with a note saying so. `None` when it is not UTF-8.
    async fn text(&self, scope: &Scope, blob: &BlobRef, name: Option<&str>) -> Option<String> {
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
        let name = name.unwrap_or("file");
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn images_and_text_are_told_apart_by_media_type() {
        assert!(is_image("image/png") && is_image("image/webp"));
        assert!(!is_image("image/svg+xml") && !is_image("image/heic"));
        assert!(is_text("text/plain; charset=utf-8") && is_text("text/markdown"));
        assert!(is_text("application/json") && is_text("application/ld+json"));
        assert!(
            is_text("image/svg+xml"),
            "an SVG is text the model can read"
        );
        assert!(!is_text("application/pdf") && !is_text("application/zip"));
    }
}
