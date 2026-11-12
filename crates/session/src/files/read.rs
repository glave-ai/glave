//! A file a thread holds, read back through the server (`file/read`): a piece at a time, so a
//! client draws an image the agent made, or saves any file, over every transport.

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use glave_event::{Sha256, ThreadId};
use glave_ports::{BlobStore, EventLog, ModelProvider};

use crate::protocol::{ApiError, FilePiece, Reply, code};
use crate::server::Server;
use crate::words::Sentence;

/// The most of a file one answer holds, in bytes: in base64, well within the 16 MiB a message
/// may be on every transport.
const PIECE: usize = 8 * 1024 * 1024;

impl<P, L, B> Server<P, L, B>
where
    P: ModelProvider + 'static,
    L: EventLog + 'static,
    B: BlobStore + 'static,
{
    /// Answers `file/read`: the piece of the file `sha256` names in `thread`'s project that
    /// starts at `offset`, after the request was let through for the thread's project.
    pub(crate) async fn read_file(
        &self,
        thread: ThreadId,
        sha256: &Sha256,
        offset: u64,
    ) -> Result<Reply, ApiError> {
        let scope = self.scope_of(thread)?;
        let found = self.inner.harness.blobs().get(&scope, sha256).await;
        let found = found.map_err(|error| ApiError::internal(&error))?;
        let Some(mut found) = found else {
            return Err(ApiError::new(code::NO_FILE, Sentence::FileNotHere));
        };
        let total = found.reference.bytes;
        let mut skip = offset;
        let mut piece = Vec::new();
        while piece.len() < PIECE {
            let next = found.bytes.next_chunk().await;
            let Some(chunk) = next.map_err(|error| ApiError::internal(&error))? else {
                break;
            };
            // What comes before `offset` is passed over, a chunk at a time.
            let length = chunk.len() as u64;
            let from = usize::try_from(skip.min(length)).unwrap_or(chunk.len());
            skip = skip.saturating_sub(length);
            let rest = chunk.get(from..).unwrap_or_default();
            let room = PIECE.saturating_sub(piece.len());
            piece.extend_from_slice(rest.get(..room).unwrap_or(rest));
        }
        let end = offset.saturating_add(piece.len() as u64);
        Ok(Reply::FilePiece {
            piece: FilePiece {
                media_type: found.reference.media_type,
                bytes: total,
                offset,
                data: STANDARD.encode(&piece),
                next: (end < total).then_some(end),
            },
        })
    }
}
