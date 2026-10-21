//! Files a person adds to a message (`specs/session.md`, "Attachments"): `file/upload` keeps
//! one, unreferenced, in a project's blobs; `turn/start` names it, and the thread refers to it
//! from then on. Images are sent to the model inline, so a message with one is refused,
//! plainly and before anything is sent, when the thread's model does not read images.

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use glave_event::{Part, ThreadId};
use glave_harness::{MAX_IMAGE, is_image};
use glave_ports::{
    BlobError, BlobStore, Caller, EventLog, ModelProvider, Pieces, ProjectId, Scope,
};

use crate::protocol::{ApiError, AttachedFile, FileInfo, Reply, code};
use crate::server::Server;
use crate::words::Sentence;

/// The largest file that is not an image, in bytes: in base64, it fits the 16 MiB a message
/// may be on every transport.
pub(crate) const MAX_FILE: u64 = 10 * 1024 * 1024;

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
    /// message to name, refusing a video, and an image or a file over its size.
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
        let limit = if is_image(&media_type) {
            MAX_IMAGE
        } else {
            MAX_FILE
        };
        let bytes = STANDARD
            .decode(data)
            .map_err(|_| ApiError::new(code::INVALID_PARAMS, Sentence::FileUnreadable))?;
        let put = self
            .inner
            .harness
            .blobs()
            .put(
                &scope,
                None,
                &media_type,
                limit,
                Box::new(Pieces::from(bytes)),
            )
            .await;
        let kept = put.map_err(|error| match error {
            BlobError::TooLarge(_) => too_large(&media_type),
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

    /// The `file` parts `files` make in `thread`'s message, once the thread refers to each:
    /// refused when one is not kept in its project (never uploaded there, or swept), and when
    /// one is an image and `model` does not read images.
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
        let blobs = self.inner.harness.blobs();
        let mut parts = Vec::with_capacity(files.len());
        for file in files {
            let found = blobs.get(scope, &file.sha256).await;
            let found = found.map_err(|error| ApiError::internal(&error))?;
            let Some(found) = found else {
                return Err(gone(&file.name));
            };
            parts.push(Part::File {
                blob: found.reference,
                name: Some(file.name.trim().to_owned()),
            });
        }
        let images = parts
            .iter()
            .any(|part| matches!(part, Part::File { blob, .. } if is_image(&blob.media_type)));
        if images && !self.reads_images(model) {
            return Err(ApiError::new(code::NO_IMAGES, Sentence::NoImages));
        }
        for (file, part) in files.iter().zip(&parts) {
            let Part::File { blob, .. } = part else {
                continue;
            };
            let referred = blobs.refer(scope, &blob.sha256, thread).await;
            if !referred.map_err(|error| ApiError::internal(&error))? {
                return Err(gone(&file.name));
            }
        }
        Ok(parts)
    }
}

/// A file a message names that is not kept: never uploaded here, or swept a day after.
fn gone(name: &str) -> ApiError {
    ApiError::new(
        code::NO_FILE,
        Sentence::FileGone {
            name: name.to_owned(),
        },
    )
}

/// An image or a file over its size.
fn too_large(media_type: &str) -> ApiError {
    let sentence = if is_image(media_type) {
        Sentence::ImageTooLarge {
            megabytes: MAX_IMAGE / (1024 * 1024),
        }
    } else {
        Sentence::FileTooLarge {
            megabytes: MAX_FILE / (1024 * 1024),
        }
    };
    ApiError::new(code::LIMIT, sentence)
}
