//! Files coming into a thread (`specs/session.md`, "Attachments"): the person's, and those the
//! tools hand the model. Each is told by what it holds rather than its name, and an image is
//! made fit to send once, as it comes in ([`fit`]), with a copy beside it when it needs one.
//! What each model call is sent is [`sent`]'s.

mod fit;
mod sent;

use glave_event::{BlobRef, Part};
use glave_ports::computer::looks_binary;
use glave_ports::{
    BlobError, BlobStore, EventLog, ModelProvider, PDF, Pieces, ProviderError, Scope, ToolFile,
    ToolOutput, is_image, read_all,
};
use thiserror::Error;

use crate::approval::Approver;
use crate::harness::{Harness, Thread};

pub use fit::MAX_SIDE;

/// The largest file a thread takes, in bytes: a PDF this size, in base64, leaves room in
/// Anthropic's 32 MB request; OpenAI takes 50 MB.
pub const MAX_FILE: u64 = 10 * 1024 * 1024;

/// How many images are made fit to send at once: decoding one can take a few hundred
/// megabytes, so a server reading many at once stays within its memory.
pub(crate) const FITTING_AT_ONCE: usize = 2;

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

/// Whether `error` is a provider refusing what a request sent inline: a request it cannot
/// take (400, 422), a model that takes none of it (404, as OpenRouter answers), or too much
/// of it (413, 415). Asked again with every file by its name, the thread goes on.
pub(crate) fn refuses_files(error: &ProviderError) -> bool {
    matches!(
        error,
        ProviderError::Http {
            status: 400 | 404 | 413 | 415 | 422,
            ..
        }
    )
}

/// Why a file could not come into a thread fit to send.
#[derive(Debug, Error)]
pub enum FileError {
    #[error("it is not a whole image glave reads: a PNG, JPEG, GIF or WebP")]
    Unreadable,
    #[error("it is too large to open: at most {MAX_SIDE} pixels on a side")]
    TooLarge,
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
    /// Makes the image `blob` fit to send, once, as it comes into a thread: its copy, kept
    /// beside it for the thread to refer to, or `None` when the image is sent as it is.
    pub async fn resized(
        &self,
        scope: &Scope,
        blob: &BlobRef,
    ) -> Result<Option<BlobRef>, FileError> {
        let found = self.blobs.get(scope, &blob.sha256).await?;
        let mut found = found.ok_or(FileError::Gone)?;
        let bytes = read_all(found.bytes.as_mut(), MAX_FILE).await?;
        let (_, copy) = self.fit(bytes).await;
        let Some(copy) = copy? else {
            return Ok(None);
        };
        let kept = self.blobs.put(
            scope,
            None,
            "image/jpeg",
            fit::MAX_IMAGE,
            Box::new(Pieces::from(copy)),
        );
        Ok(Some(kept.await?))
    }

    /// `output` with the files it hands the model kept for `thread`, an image fit to send,
    /// each a `file` part after its parts; one that cannot be kept is said so instead.
    pub(crate) async fn keep_files(&self, thread: &Thread, mut output: ToolOutput) -> ToolOutput {
        for file in std::mem::take(&mut output.files) {
            let name = file.name.clone();
            output.parts.push(match self.kept(thread, file).await {
                Ok(part) => part,
                Err(error) => Part::Text {
                    text: format!("({name} could not be shown: {error})"),
                },
            });
        }
        output
    }

    /// `file`, kept for `thread` as the model is sent it.
    async fn kept(&self, thread: &Thread, file: ToolFile) -> Result<Part, FileError> {
        let (media_type, bytes) = if is_image(&file.media_type) {
            match self.fit(file.bytes).await {
                (_, Ok(Some(copy))) => ("image/jpeg".to_owned(), copy),
                (bytes, Ok(None)) => (file.media_type, bytes),
                (_, Err(error)) => return Err(error),
            }
        } else {
            (file.media_type, file.bytes)
        };
        let put = self.blobs.put(
            &thread.scope,
            Some(thread.id),
            &media_type,
            MAX_FILE,
            Box::new(Pieces::from(bytes)),
        );
        Ok(Part::File {
            blob: put.await?,
            name: Some(file.name),
            resized: None,
        })
    }

    /// `bytes` back, and [`fit::fit`] of them: off the async threads, since decoding and
    /// scaling a photo takes a fifth of a second, and at most [`FITTING_AT_ONCE`] at a time.
    async fn fit(&self, bytes: Vec<u8>) -> (Vec<u8>, Result<Option<Vec<u8>>, FileError>) {
        let _permit = self.fitting.acquire().await;
        let fitting = tokio::task::spawn_blocking(move || {
            let copy = fit::fit(&bytes);
            (bytes, copy)
        });
        fitting
            .await
            .unwrap_or_else(|_| (Vec::new(), Err(FileError::Unreadable)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_is_told_by_media_type() {
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

    #[test]
    fn a_refusal_of_what_was_sent_is_told_from_one_that_may_pass() {
        let http = |status| ProviderError::Http {
            status,
            body: String::new(),
        };
        assert!(refuses_files(&http(400)) && refuses_files(&http(404)));
        assert!(refuses_files(&http(413)));
        assert!(!refuses_files(&http(401)) && !refuses_files(&http(429)));
        assert!(!refuses_files(&http(503)));
        assert!(!refuses_files(&ProviderError::Transport("reset".into())));
    }
}
