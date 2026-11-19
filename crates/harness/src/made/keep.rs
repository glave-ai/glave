//! A file a look-over found, kept for the thread: asked for again, read through the computer
//! port, hashed and looked through for the person's secrets off the async threads, then read
//! apart from glave, by the look-over's deadline, for its title,
//! its parts and its preview. What the reader found is looked through for the person's secrets
//! too, since it is the file's text joined up (a value Word splits across runs, or one written
//! as character references, shows whole there). A file holding none is kept sealed, its digest
//! holding through the read, its preview beside it.

use glave_event::{AssetKind, BlobRef, FileMade, FileRead, Kept, MadePreview};
use glave_ports::computer::EntryKind;
use glave_ports::{BlobStore, EventLog, MadeRead, ModelProvider, Pieces, digest};
use tokio::time::Instant;

use super::scan::{self, Scanned};
use crate::approval::Approver;
use crate::cancel::Cancel;
use crate::files::MAX_FILE;
use crate::harness::{Harness, Thread};

/// What a preview is kept as.
const MARKDOWN: &str = "text/markdown";

/// How long the files of a look-over may be read for, until its deadline, and what stops the
/// look-over: the turn it ends with being stopped, which owes the look.
pub(crate) struct Reading {
    pub(crate) until: Instant,
    pub(crate) cancel: Cancel,
}

impl<P, L, B, A> Harness<P, L, B, A>
where
    P: ModelProvider,
    L: EventLog,
    B: BlobStore,
    A: Approver,
{
    /// The file at `path` in `thread`'s folder, kept for the thread when it is no larger than a
    /// thread keeps and it, and what was read of it, hold none of the person's secrets: read
    /// apart from glave within `reading`, when it is given. `None` when it isn't a plain file
    /// there now, or it changed while it was read, as one still being written does.
    pub(super) async fn keep_made_file(
        &self,
        thread: &Thread,
        path: &str,
        reading: Option<&Reading>,
    ) -> Option<FileMade> {
        let workspace = self.workspace_for(thread);
        let stat = workspace.stat(path).await.ok()?;
        let media_type = AssetKind::made_as(&stat.path)?;
        if stat.kind != Some(EntryKind::File) {
            return None;
        }
        let made = |kept, read| FileMade {
            path: stat.path.clone(),
            bytes: stat.size,
            kept,
            read,
        };
        if stat.size > MAX_FILE {
            return Some(made(Kept::TooLarge, None));
        }
        let bytes = workspace.read(&stat.path).await.ok()?.into_vec();
        let values = self.secret_values();
        let looking = values.clone();
        // Hashing and looking through up to 10 MB, and what its parts inflate to, takes a while.
        let looked = tokio::task::spawn_blocking(move || {
            let (sha256, scanned) = (digest(&bytes), scan::scanned(&bytes, &looking));
            (bytes, sha256, scanned)
        });
        let (bytes, sha256, scanned) = looked.await.ok()?;
        if Some(sha256) != stat.digest {
            return None;
        }
        match scanned {
            Scanned::Holds => return Some(made(Kept::Secret, None)),
            Scanned::Unchecked => return Some(made(Kept::Unchecked, None)),
            Scanned::Clean => {}
        }
        let read = match (&self.reader, reading) {
            (Some(reader), Some(reading)) => {
                let read = reader(bytes.clone(), media_type.to_owned());
                tokio::time::timeout_at(reading.until, read)
                    .await
                    .ok()
                    .flatten()
            }
            _ => None,
        };
        if read
            .as_ref()
            .is_some_and(|read| shows_secret(read, &values))
        {
            return Some(made(Kept::Secret, None));
        }
        let blob = self.keep_bytes(thread, media_type, bytes).await?;
        let read = match read {
            Some(read) => Some(self.kept_read(thread, read).await),
            None => None,
        };
        Some(made(Kept::Blob(blob), read))
    }

    /// What reading a file found, its preview kept for `thread`; none when it can't be.
    async fn kept_read(&self, thread: &Thread, read: MadeRead) -> FileRead {
        let preview = match read.markdown {
            Some(markdown) => self
                .keep_bytes(thread, MARKDOWN, markdown.into_bytes())
                .await
                .map(|blob| MadePreview {
                    blob,
                    cut: read.cut,
                }),
            None => None,
        };
        FileRead {
            title: read.title,
            parts: read.parts,
            preview,
        }
    }

    /// The values of the person's secrets, as a file a turn made must not hold them.
    fn secret_values(&self) -> Vec<Vec<u8>> {
        self.workspace
            .hidden()
            .variables()
            .values()
            .map(|value| value.expose().as_bytes().to_vec())
            .collect()
    }

    /// `bytes`, kept for `thread`, sealed, as `media_type`; `None`, logged, when they can't be.
    async fn keep_bytes(
        &self,
        thread: &Thread,
        media_type: &str,
        bytes: Vec<u8>,
    ) -> Option<BlobRef> {
        let pieces = Box::new(Pieces::from(bytes));
        let put = self
            .blobs
            .put(&thread.scope, Some(thread.id), media_type, MAX_FILE, pieces);
        match put.await {
            Ok(blob) => Some(blob),
            Err(error) => {
                tracing::warn!(%error, "a file the turn made could not be kept");
                None
            }
        }
    }
}

/// Whether what was read of a file, its title or its preview, shows one of `values`.
fn shows_secret(read: &MadeRead, values: &[Vec<u8>]) -> bool {
    [read.title.as_deref(), read.markdown.as_deref()]
        .into_iter()
        .flatten()
        .any(|text| scan::scanned(text.as_bytes(), values) == Scanned::Holds)
}
