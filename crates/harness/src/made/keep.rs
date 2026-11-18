//! A file a look-over found, kept for the thread: asked for again, read through the computer
//! port, hashed and looked through for the person's secrets off the async threads, and kept
//! sealed when its digest held through the read and it holds none.

use glave_event::{AssetKind, BlobRef, FileMade, Kept};
use glave_ports::computer::EntryKind;
use glave_ports::{BlobStore, EventLog, ModelProvider, Pieces, digest};

use super::scan::{self, Scanned};
use crate::approval::Approver;
use crate::files::MAX_FILE;
use crate::harness::{Harness, Thread};

impl<P, L, B, A> Harness<P, L, B, A>
where
    P: ModelProvider,
    L: EventLog,
    B: BlobStore,
    A: Approver,
{
    /// The file at `path` in `thread`'s folder, kept for the thread when it is no larger than a
    /// thread keeps and holds none of the person's secrets. `None` when it isn't a plain file
    /// there now, or it changed while it was read, as one still being written does.
    pub(super) async fn keep_made_file(&self, thread: &Thread, path: &str) -> Option<FileMade> {
        let workspace = self.workspace_for(thread);
        let stat = workspace.stat(path).await.ok()?;
        let media_type = AssetKind::made_as(&stat.path)?;
        if stat.kind != Some(EntryKind::File) {
            return None;
        }
        let made = |kept| FileMade {
            path: stat.path.clone(),
            bytes: stat.size,
            kept,
        };
        if stat.size > MAX_FILE {
            return Some(made(Kept::TooLarge));
        }
        let bytes = workspace.read(&stat.path).await.ok()?.into_vec();
        let values = self.secret_values();
        // Hashing and looking through up to 10 MB, and what its parts inflate to, takes a while.
        let looked = tokio::task::spawn_blocking(move || {
            let (sha256, scanned) = (digest(&bytes), scan::scanned(&bytes, &values));
            (bytes, sha256, scanned)
        });
        let (bytes, sha256, scanned) = looked.await.ok()?;
        if Some(sha256) != stat.digest {
            return None;
        }
        match scanned {
            Scanned::Holds => return Some(made(Kept::Secret)),
            Scanned::Unchecked => return Some(made(Kept::Unchecked)),
            Scanned::Clean => {}
        }
        let blob = self.keep_bytes(thread, media_type, bytes).await?;
        Some(made(Kept::Blob(blob)))
    }

    /// The values of the person's secrets, as a file a turn made must not hold them.
    pub(super) fn secret_values(&self) -> Vec<Vec<u8>> {
        self.workspace
            .hidden()
            .variables()
            .values()
            .map(|value| value.expose().as_bytes().to_vec())
            .collect()
    }

    /// `bytes`, kept for `thread`, sealed, as `media_type`; `None`, logged, when they can't be.
    pub(super) async fn keep_bytes(
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
