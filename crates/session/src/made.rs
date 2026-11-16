//! Files a thread's turns made for the person (`specs/session.md`, "Files it made"), read from
//! its log: glave's own `write` and `bash` record each file a call made in its
//! `tool.call.completed` (`glave_event::FileMade`), kept with the thread. `thread/files` gives
//! them by turn, with their kind, title and size; `file/preview` gives one as Markdown.

use std::collections::HashSet;

use glave_event::{Body, CallId, FileKind, FileMade, RawEvent, Seq, Sha256, ThreadId, decode};
use glave_harness::MAX_FILE;
use glave_ports::{BlobStore, EventLog, ModelProvider, read_all};

use crate::protocol::{ApiError, MadeFile, Reply, TurnFiles, code};
use crate::server::Server;
use crate::words::Sentence;

impl<P, L, B> Server<P, L, B>
where
    P: ModelProvider + 'static,
    L: EventLog + 'static,
    B: BlobStore + 'static,
{
    /// The files `thread`'s turns made, read from its log.
    pub(crate) async fn made_files(&self, thread: ThreadId) -> Result<Reply, ApiError> {
        let mut reading = Reading::default();
        self.read_all(thread, |event| reading.read(event)).await?;
        Ok(Reply::Files {
            made: reading.turns,
        })
    }

    /// Answers `file/preview`: the file `sha256` that `thread` made, as Markdown, when it is a
    /// spreadsheet, a CSV file, a document or a deck glave reads; refused otherwise.
    pub(crate) async fn preview_file(
        &self,
        thread: ThreadId,
        sha256: &Sha256,
    ) -> Result<Reply, ApiError> {
        let scope = self.scope_of(thread)?;
        let not_here = || ApiError::new(code::NO_FILE, Sentence::FileNotHere);
        if !self.names_file(&scope, thread, sha256).await? {
            return Err(not_here());
        }
        let found = self.inner.harness.blobs().get(&scope, sha256).await;
        let found = found.map_err(|error| ApiError::internal(&error))?;
        let mut found = found.ok_or_else(not_here)?;
        let bytes = read_all(found.bytes.as_mut(), MAX_FILE).await;
        let bytes = bytes.map_err(|error| ApiError::internal(&error))?;
        let media_type = found.reference.media_type;
        // Reading a workbook or a document's XML takes a while for a large one.
        let previewed =
            tokio::task::spawn_blocking(move || glave_office::preview(&bytes, &media_type)).await;
        match previewed {
            Ok(Ok(markdown)) => Ok(Reply::FilePreview { markdown }),
            _ => Err(ApiError::new(code::INVALID_PARAMS, Sentence::NoPreview)),
        }
    }
}

/// A thread's log read so far, in order: the files each turn made, and the calls of glave's own
/// tools that make them.
#[derive(Default)]
struct Reading {
    turns: Vec<TurnFiles>,
    /// Where each file of the last turn was made, in its order.
    paths: Vec<String>,
    makers: HashSet<CallId>,
}

impl Reading {
    /// Reads the next event of the log.
    fn read(&mut self, event: RawEvent) {
        let seq = event.seq;
        let Ok(event) = decode(event) else {
            return;
        };
        match event.body {
            Body::TurnStarted(_) => {
                self.paths.clear();
                self.turns.push(TurnFiles {
                    turn: seq,
                    files: Vec::new(),
                });
            }
            Body::ToolCallRequested(requested)
                if FileMade::TOOLS.contains(&requested.tool.as_str()) =>
            {
                self.makers.insert(requested.call_id);
            }
            Body::ToolCallCompleted(done) if self.makers.contains(&done.call_id) => {
                let made = done.structured.as_ref().map(FileMade::read);
                if let Some(turn) = self.turns.last_mut() {
                    for file in made.unwrap_or_default() {
                        kept(&mut turn.files, &mut self.paths, seq, file);
                    }
                }
            }
            _ => {}
        }
    }
}

/// Keeps `file`, made at `seq`, among `files`, whose paths are `paths`: in place of what was
/// made before at its path, else after them.
fn kept(files: &mut Vec<MadeFile>, paths: &mut Vec<String>, seq: Seq, file: FileMade) {
    let name = file
        .path
        .rsplit('/')
        .next()
        .unwrap_or(&file.path)
        .to_owned();
    let title = name
        .rsplit_once('.')
        .map_or(name.as_str(), |(stem, _)| stem)
        .to_owned();
    let made = MadeFile {
        seq,
        sha256: file.blob.sha256,
        kind: FileKind::of(&file.blob.media_type),
        media_type: file.blob.media_type,
        bytes: file.blob.bytes,
        name,
        title,
        kept: Sentence::KeptWithChat.into(),
    };
    let before = paths.iter().position(|path| *path == file.path);
    match before.and_then(|at| files.get_mut(at)) {
        Some(before) => *before = made,
        None => {
            paths.push(file.path);
            files.push(made);
        }
    }
}

#[cfg(test)]
mod tests;
