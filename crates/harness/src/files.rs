//! The files in a thread made ready for the model (`specs/session.md`, "Attachments"): the
//! person's and those the tools hand it. An image or a PDF is sent inline, where the provider
//! puts it beside the text; a text file as its text, cut at [`MAX_TEXT`]; anything else by its
//! name. An image comes into the thread fit to send ([`fit`]), with a copy beside it when it
//! needs one. The bytes are read from the blob store at each model call, so nothing is held
//! between them. A model that reads no images is sent every file by its name.

mod fit;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use glave_event::{BlobRef, Part, ThreadId};
use glave_ports::computer::looks_binary;
use glave_ports::{
    BlobError, BlobStore, EventLog, InlineFile, Message, ModelProvider, Pieces, Scope, ToolOutput,
    read_all,
};
use thiserror::Error;

use crate::approval::Approver;
use crate::harness::{Harness, Thread};

pub(crate) use fit::{Fitted, MAX_IMAGE};

/// The image types every provider glave speaks to reads inline.
pub(crate) const IMAGE_TYPES: [&str; 4] = ["image/png", "image/jpeg", "image/gif", "image/webp"];

/// A PDF's media type: providers read one as its text and an image of each page.
pub(crate) const PDF: &str = "application/pdf";

/// The largest file a thread takes, in bytes: a PDF this size, in base64, leaves room in
/// Anthropic's 32 MB request; OpenAI takes 50 MB.
pub const MAX_FILE: u64 = 10 * 1024 * 1024;

/// The most of a text file the model reads, in bytes; the rest is said to be cut.
pub(crate) const MAX_TEXT: u64 = 256 * 1024;

/// Whether a file of `media_type` is an image the model reads inline.
pub fn is_image(media_type: &str) -> bool {
    IMAGE_TYPES.contains(&media_type)
}

/// Whether a file of `media_type` is one a model reads by sight, inline: an image, or a PDF,
/// which models read as its text and an image of each page.
pub fn is_visual(media_type: &str) -> bool {
    is_image(media_type) || media_type == PDF
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

/// What a file is kept as, told by what it holds rather than its name: an image (PNG, JPEG,
/// GIF, WebP) or a PDF by its first bytes; text as it was `said` to be, or `text/plain`.
/// `None` for anything else, which glave does not read.
pub fn kind_of(said: &str, bytes: &[u8]) -> Option<String> {
    if let Some(sniffed) = sniffed(bytes) {
        return Some(sniffed.to_owned());
    }
    (!looks_binary(bytes)).then(|| if is_text(said) { said } else { "text/plain" }.to_owned())
}

/// The media type of an image glave reads (PNG, JPEG, GIF, WebP) or a PDF, by its first bytes.
pub(crate) fn sniffed(bytes: &[u8]) -> Option<&'static str> {
    let image = image::guess_format(bytes)
        .ok()
        .map(|format| format.to_mime_type());
    if let Some(image) = image.filter(|image| is_image(image)) {
        return Some(image);
    }
    bytes.starts_with(b"%PDF-").then_some(PDF)
}

/// [`fit::fit`], off the async threads, since decoding and scaling a photo takes a tenth of
/// a second: `bytes` back, and the image they are made fit to send, when they are one.
pub(crate) async fn fitted(bytes: Vec<u8>) -> (Vec<u8>, Option<Fitted>) {
    let fitting = tokio::task::spawn_blocking(move || {
        let fitted = fit::fit(&bytes);
        (bytes, fitted)
    });
    fitting.await.unwrap_or_default()
}

/// Why an image could not be made fit to send.
#[derive(Debug, Error)]
pub enum FileError {
    #[error("it is not an image glave can read")]
    NotAnImage,
    #[error("it is no longer kept")]
    Gone,
    #[error(transparent)]
    Blob(#[from] BlobError),
}

impl<P, L, B, A> Harness<P, L, B, A>
where
    P: ModelProvider,
    L: EventLog,
    B: BlobStore,
    A: Approver,
{
    /// Makes the image `blob` fit to send, once, as it comes into `thread`: its copy, kept
    /// for the thread beside it, or `None` when the image is sent as it is.
    pub async fn resized(
        &self,
        scope: &Scope,
        thread: ThreadId,
        blob: &BlobRef,
    ) -> Result<Option<BlobRef>, FileError> {
        let found = self.blobs.get(scope, &blob.sha256).await?;
        let mut found = found.ok_or(FileError::Gone)?;
        let bytes = read_all(found.bytes.as_mut(), MAX_FILE).await?;
        let (_, fitted) = fitted(bytes).await;
        let Some(copy) = fitted.ok_or(FileError::NotAnImage)?.copy else {
            return Ok(None);
        };
        let kept = self.blobs.put(
            scope,
            Some(thread),
            "image/jpeg",
            MAX_IMAGE,
            Box::new(Pieces::from(copy.bytes)),
        );
        Ok(Some(kept.await?))
    }

    /// `output` with the files it hands the model kept for `thread`, each a `file` part after
    /// its parts; one that cannot be kept is said so instead.
    pub(crate) async fn keep_files(&self, thread: &Thread, mut output: ToolOutput) -> ToolOutput {
        for file in std::mem::take(&mut output.files) {
            let bytes = Box::new(Pieces::from(file.bytes));
            let put = self.blobs.put(
                &thread.scope,
                Some(thread.id),
                &file.media_type,
                MAX_FILE,
                bytes,
            );
            output.parts.push(match put.await {
                Ok(blob) => Part::File {
                    blob,
                    name: Some(file.name),
                    resized: None,
                },
                Err(error) => Part::Text {
                    text: format!("({} could not be shown: {error})", file.name),
                },
            });
        }
        output
    }

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
        let mut files: Vec<InlineFile> = Vec::new();
        for message in &mut messages {
            let (Message::User { parts } | Message::Tool { output: parts, .. }) = message else {
                continue;
            };
            for part in parts.iter_mut() {
                let Some(sent) = part.sent().cloned() else {
                    continue;
                };
                if is_visual(&sent.media_type) {
                    if sees
                        && files.iter().all(|file| file.sha256 != sent.sha256)
                        && let Some(file) = self.inline(scope, &sent).await
                    {
                        files.push(file);
                    }
                } else if let Part::File { name, .. } = part
                    && is_text(&sent.media_type)
                    && let Some(text) = self.text(scope, &sent, name.as_deref()).await
                {
                    *part = Part::Text { text };
                }
            }
        }
        (messages, files)
    }

    /// The image or PDF `blob` holds, in base64, when it can be read and is no larger than
    /// the providers take.
    async fn inline(&self, scope: &Scope, blob: &BlobRef) -> Option<InlineFile> {
        let most = if blob.media_type == PDF {
            MAX_FILE
        } else {
            MAX_IMAGE
        };
        if blob.bytes > most {
            return None;
        }
        let bytes = self.read(scope, blob, most).await?;
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

    #[test]
    fn a_file_is_kept_as_what_it_holds_not_what_it_is_called() {
        let png = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR";
        assert_eq!(kind_of("text/plain", png).as_deref(), Some("image/png"));
        assert_eq!(
            kind_of("image/png", b"\xff\xd8\xff\xe0").as_deref(),
            Some("image/jpeg")
        );
        assert_eq!(kind_of("", b"%PDF-1.7\n").as_deref(), Some(PDF));
        assert_eq!(
            kind_of("text/markdown", b"# Notes").as_deref(),
            Some("text/markdown")
        );
        assert_eq!(
            kind_of("application/octet-stream", b"fn main() {}").as_deref(),
            Some("text/plain"),
            "source code a browser could not name is text"
        );
        assert_eq!(kind_of("image/heic", b"\0\0\0\x18ftypheic"), None);
        assert_eq!(kind_of("application/zip", b"PK\x03\x04\0\0"), None);
    }
}
