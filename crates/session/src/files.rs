//! Files a person adds to a message (`specs/session.md`, "Attachments"): `file/upload` keeps
//! one, unreferenced and untouched, in a project's blobs; `turn/start` names it, and the thread
//! refers to it from then on, an image with the copy the model is sent beside it. Images and
//! PDFs are sent to the model inline, so a message with one is refused, plainly and before
//! anything is sent, when the thread's model does not read them.

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use glave_event::{Part, ThreadId};
use glave_harness::{FileError, MAX_FILE, is_image, is_visual, kind_of};
use glave_ports::{
    BlobError, BlobStore, Caller, EventLog, ModelProvider, Pieces, ProjectId, Scope,
};

use crate::protocol::{ApiError, AttachedFile, FileInfo, Reply, code};
use crate::server::Server;
use crate::words::Sentence;

/// The most files one message holds.
pub(crate) const MAX_FILES: usize = 20;

/// The longest name a file keeps, in characters.
const MAX_NAME: usize = 255;

impl<P, L, B> Server<P, L, B>
where
    P: ModelProvider + 'static,
    L: EventLog + 'static,
    B: BlobStore + 'static,
{
    /// Answers `file/upload`: keeps `data` in `project` (the caller's Chats when absent) for a
    /// message to name, as what it holds says it is (`media_type` decides only text's kind),
    /// refusing a video, a file over its size, and any file but an image, a PDF or text. Every
    /// file fits in base64 within the 16 MiB a message may be on every transport.
    pub(crate) async fn upload(
        &self,
        caller: &Caller,
        project: Option<ProjectId>,
        name: &str,
        media_type: &str,
        data: &str,
    ) -> Result<Reply, ApiError> {
        let (_, scope) = self.project_for(caller, project).await?;
        let name = name.trim();
        if name.is_empty() || name.chars().count() > MAX_NAME {
            return Err(ApiError::new(
                code::INVALID_PARAMS,
                Sentence::FileName { most: MAX_NAME },
            ));
        }
        let media_type = media_type.trim().to_ascii_lowercase();
        if media_type.starts_with("video/") {
            return Err(ApiError::new(code::INVALID_PARAMS, Sentence::NoVideos));
        }
        let bytes = STANDARD
            .decode(data)
            .map_err(|_| ApiError::new(code::INVALID_PARAMS, Sentence::FileUnreadable))?;
        let Some(kind) = kind_of(&media_type, &bytes) else {
            return Err(ApiError::new(code::INVALID_PARAMS, Sentence::FileKind));
        };
        let put = self
            .inner
            .harness
            .blobs()
            .put(&scope, None, &kind, MAX_FILE, Box::new(Pieces::from(bytes)))
            .await;
        let kept = put.map_err(|error| match error {
            BlobError::TooLarge(_) => ApiError::new(
                code::LIMIT,
                Sentence::FileTooLarge {
                    megabytes: MAX_FILE / (1024 * 1024),
                },
            ),
            BlobError::Storage(_) => ApiError::internal(&error),
        })?;
        Ok(Reply::File {
            file: FileInfo {
                sha256: kept.sha256,
                name: name.to_owned(),
                media_type: kept.media_type,
                bytes: kept.bytes,
            },
        })
    }

    /// The `file` parts `files` make in `thread`'s message, once the thread refers to each and
    /// each image has the copy the model is sent: refused when one is not kept in its project
    /// (never uploaded there, or swept), when an image cannot be opened, and when one is an
    /// image or a PDF and `model` does not read them.
    pub(crate) async fn attach(
        &self,
        scope: &Scope,
        thread: ThreadId,
        files: &[AttachedFile],
        model: &str,
    ) -> Result<Vec<Part>, ApiError> {
        if files.len() > MAX_FILES {
            return Err(ApiError::new(
                code::LIMIT,
                Sentence::TooManyFiles { most: MAX_FILES },
            ));
        }
        let harness = &self.inner.harness;
        let blobs = harness.blobs();
        let mut found = Vec::with_capacity(files.len());
        for file in files {
            let blob = blobs.get(scope, &file.sha256).await;
            let blob = blob.map_err(|error| ApiError::internal(&error))?;
            let blob = blob.ok_or_else(|| gone(&file.name))?.reference;
            found.push((file.name.trim(), blob));
        }
        let shown = found.iter().any(|(_, blob)| is_visual(&blob.media_type));
        if shown && !harness.reads_images(model) {
            return Err(ApiError::new(code::NO_IMAGES, Sentence::NoImages));
        }
        let mut parts = Vec::with_capacity(found.len());
        for (name, blob) in found {
            let referred = blobs.refer(scope, &blob.sha256, thread).await;
            if !referred.map_err(|error| ApiError::internal(&error))? {
                return Err(gone(name));
            }
            let resized = if is_image(&blob.media_type) {
                let resized = harness.resized(scope, thread, &blob).await;
                resized.map_err(|error| unfit(name, &error))?
            } else {
                None
            };
            parts.push(Part::File {
                blob,
                name: Some(name.to_owned()),
                resized,
            });
        }
        Ok(parts)
    }
}

/// A file a message names that is not kept: never uploaded here, or swept a day after.
fn gone(name: &str) -> ApiError {
    ApiError::new(
        code::NO_FILE,
        Sentence::FileGone {
            name: name.escape_debug().to_string(),
        },
    )
}

/// Why the image `name` could not be made fit to send.
fn unfit(name: &str, error: &FileError) -> ApiError {
    match error {
        FileError::NotAnImage => ApiError::new(
            code::INVALID_PARAMS,
            Sentence::ImageUnreadable {
                name: name.escape_debug().to_string(),
            },
        ),
        FileError::Gone => gone(name),
        FileError::Blob(error) => ApiError::internal(error),
    }
}
