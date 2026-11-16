//! A file a thread holds, read back through the server (`file/read`): a piece at a time, so a
//! client draws an image the agent made, or saves any file, over every transport. Only a file
//! one of the thread's own messages or tool results names is given: never what glave keeps
//! beside them (a turn's configuration, what a service keeps while it asks), nor another
//! thread's.

use std::collections::HashSet;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use glave_event::{
    Body, CallId, FileMade, MessageAdded, Part, Payload as _, RawEvent, Sha256, ThreadId,
    ToolCallCompleted, ToolCallRequested, decode,
};
use glave_ports::stream::Reserved;
use glave_ports::{Blob, BlobStore, EventLog, ModelProvider, Scope};

use crate::protocol::{ApiError, FilePiece, Reply, code};
use crate::server::{MAX_PAGE, Server, no_thread};
use crate::words::Sentence;

/// The most of a file one answer holds, in bytes: in base64, well within the 16 MiB a message
/// may be on every transport.
const PIECE: usize = 8 * 1024 * 1024;

/// The events that name the files a thread holds: its messages and its tools' results, and the
/// calls that say which tool a result is of, since only glave's own make files.
const NAMING: [&str; 3] = [
    MessageAdded::TYPE,
    ToolCallCompleted::TYPE,
    ToolCallRequested::TYPE,
];

impl<P, L, B> Server<P, L, B>
where
    P: ModelProvider + 'static,
    L: EventLog + 'static,
    B: BlobStore + 'static,
{
    /// Answers `file/read`: the piece of the file `sha256` names in `thread` that starts at
    /// `offset`, after the request was let through for the thread's project. A file no `file`
    /// part of the thread names is "no longer here", as one never kept.
    pub(crate) async fn read_file(
        &self,
        thread: ThreadId,
        sha256: &Sha256,
        offset: u64,
    ) -> Result<Reply, ApiError> {
        let mut found = self.held_file(thread, sha256).await?;
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

    /// The file `sha256` that `thread` holds, to read, after the request was let through for the
    /// thread's project: one its messages or tool results name, or one of its calls made. Any
    /// other is "no longer here", as one never kept.
    pub(crate) async fn held_file(
        &self,
        thread: ThreadId,
        sha256: &Sha256,
    ) -> Result<Blob, ApiError> {
        let scope = self.scope_of(thread)?;
        let not_here = || ApiError::new(code::NO_FILE, Sentence::FileNotHere);
        if !self.names_file(&scope, thread, sha256).await? {
            return Err(not_here());
        }
        let found = self.inner.harness.blobs().get(&scope, sha256).await;
        let found = found.map_err(|error| ApiError::internal(&error))?;
        found.ok_or_else(not_here)
    }

    /// Whether a `file` part of `thread`'s messages or tool results names `sha256`, as the file
    /// or the copy of an image the model is sent, or a call of glave's own `write` or `bash`
    /// made it for the person (`glave_event::FileMade`): read back from the thread's end, where
    /// the files a client draws are, a page at a time until it is found. A deleted thread is no
    /// such thread.
    async fn names_file(
        &self,
        scope: &Scope,
        thread: ThreadId,
        sha256: &Sha256,
    ) -> Result<bool, ApiError> {
        if Reserved::of(thread).is_some() {
            return Err(no_thread(thread));
        }
        self.ensure_not_deleted(thread).await?;
        let log = self.inner.harness.log();
        let mut before = None;
        // The calls whose results say they made it, read before the calls themselves.
        let mut made = HashSet::new();
        loop {
            let events = log
                .read_back(scope, thread, Some(&NAMING), before, MAX_PAGE)
                .await?;
            let more = events.len() == MAX_PAGE;
            before = events.last().map(|event| event.seq);
            if events
                .into_iter()
                .any(|event| names(event, sha256, &mut made))
            {
                return Ok(true);
            }
            if !more || before.is_none() {
                return Ok(false);
            }
        }
    }
}

/// Whether `event`, a message or a tool's result, has a `file` part naming `sha256`, or is the
/// call of glave's own `write` or `bash` whose result, among `made`, says it made it.
fn names(event: RawEvent, sha256: &Sha256, made: &mut HashSet<CallId>) -> bool {
    let parts = match decode(event).map(|event| event.body) {
        Ok(Body::MessageAdded(message)) => message.parts,
        Ok(Body::ToolCallCompleted(done)) => {
            let structured = done.structured.as_ref().map(FileMade::read);
            if structured.is_some_and(|files| files.iter().any(|file| file.blob.sha256 == *sha256))
            {
                made.insert(done.call_id);
            }
            done.output
        }
        Ok(Body::ToolCallRequested(requested)) => {
            return made.contains(&requested.call_id)
                && FileMade::TOOLS.contains(&requested.tool.as_str());
        }
        _ => return false,
    };
    parts.iter().any(|part| match part {
        Part::File { blob, resized, .. } => {
            blob.sha256 == *sha256 || resized.as_ref().is_some_and(|copy| copy.sha256 == *sha256)
        }
        Part::Text { .. } | Part::Reasoning { .. } => false,
    })
}
