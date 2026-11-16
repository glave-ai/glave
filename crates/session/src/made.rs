//! Files a thread's turns made for the person (`specs/session.md`, "Files it made"), read from
//! its log: each `turn.ended` names the files its turn made (`glave_event::FileMade`), kept with
//! the thread or left on the computer. `thread/files` gives them by turn, each described from its
//! content by glave-office (its title, its pages, sheets or slides, whether it previews), the
//! descriptions kept for the files read most lately; `file/preview` gives one as Markdown. Both
//! read at most [`AT_ONCE`] files at a time, each within [`WAIT`].

use std::collections::VecDeque;
use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use glave_event::{AssetKind, Body, FileMade, Kept, RawEvent, Seq, Sha256, ThreadId, decode};
use glave_harness::MAX_FILE;
use glave_office::{Described, Parts};
use glave_ports::{BlobStore, EventLog, ModelProvider, Scope, read_all, spoken};
use tokio::sync::Semaphore;

use crate::files::not_here;
use crate::protocol::{ApiError, FilePreview, MadeFile, Reply, Told, TurnFiles, code};
use crate::server::Server;
use crate::words::Sentence;

/// How many made files are described or previewed at once: each reads a file of up to 10 MB.
const AT_ONCE: usize = 2;

/// The longest a file is read for its description or its preview.
const WAIT: Duration = Duration::from_secs(20);

/// The most descriptions kept, by the file's hash.
const KEPT: usize = 256;

/// What reading made files shares: the turn to read one, and the descriptions read most lately.
pub(crate) struct Reading {
    permits: Semaphore,
    described: Mutex<VecDeque<(Sha256, Described)>>,
}

impl Default for Reading {
    fn default() -> Self {
        Self {
            permits: Semaphore::new(AT_ONCE),
            described: Mutex::new(VecDeque::with_capacity(KEPT)),
        }
    }
}

impl<P, L, B> Server<P, L, B>
where
    P: ModelProvider + 'static,
    L: EventLog + 'static,
    B: BlobStore + 'static,
{
    /// The files `thread`'s turns made, read from its log, each described.
    pub(crate) async fn made_files(&self, thread: ThreadId) -> Result<Reply, ApiError> {
        let scope = self.scope_of(thread)?;
        let mut turns: Vec<(Seq, Vec<FileMade>)> = Vec::new();
        self.read_all(thread, |event| read(&mut turns, event))
            .await?;
        let mut made = Vec::with_capacity(turns.len());
        for (turn, files) in turns.into_iter().filter(|(_, files)| !files.is_empty()) {
            let mut described = Vec::with_capacity(files.len());
            for file in files {
                described.push(self.made_file(&scope, file).await);
            }
            made.push(TurnFiles {
                turn,
                files: described,
            });
        }
        Ok(Reply::Files { made })
    }

    /// Answers `file/preview`: the file `sha256` that `thread` holds, as Markdown, when it is a
    /// spreadsheet, a CSV file, a document or a deck glave reads; refused otherwise.
    pub(crate) async fn preview_file(
        &self,
        thread: ThreadId,
        sha256: &Sha256,
    ) -> Result<Reply, ApiError> {
        let scope = self.holding(thread, sha256).await?;
        let previewed = self
            .read_made(&scope, sha256, |bytes, media_type| {
                glave_office::preview(&bytes, &media_type).ok()
            })
            .await?;
        let preview =
            previewed.ok_or_else(|| ApiError::new(code::INVALID_PARAMS, Sentence::NoPreview))?;
        Ok(Reply::FilePreview {
            preview: FilePreview {
                markdown: preview.markdown,
                cut: preview.cut.then(|| Sentence::PreviewCut.into()),
            },
        })
    }

    /// `file` as `thread/files` gives it, described from its content when it was kept.
    async fn made_file(&self, scope: &Scope, file: FileMade) -> MadeFile {
        let file_name = file
            .path
            .rsplit('/')
            .next()
            .unwrap_or(&file.path)
            .to_owned();
        let media_type = AssetKind::made_as(&file.path).unwrap_or_default();
        let (sha256, kept) = match &file.kept {
            Kept::Blob(blob) => (Some(blob.sha256.clone()), Sentence::KeptWithChat),
            Kept::TooLarge => (
                None,
                Sentence::LeftTooLarge {
                    megabytes: MAX_FILE / (1024 * 1024),
                },
            ),
            Kept::Secret => (None, Sentence::LeftSecret),
        };
        let described = match &sha256 {
            Some(sha256) => self.made_described(scope, sha256).await,
            None => Described::default(),
        };
        let kind = AssetKind::of(media_type);
        let name = described.title.clone().unwrap_or_else(|| stem(&file_name));
        let (unit, count) = match described.parts {
            Some(Parts::Pages(count)) => ("pages", count),
            Some(Parts::Sheets(count)) => ("sheets", count),
            Some(Parts::Slides(count)) => ("slides", count),
            None => ("none", 0),
        };
        let sentence = Sentence::MadeLine {
            kind: kind_word(kind).to_owned(),
            name: name.clone(),
            unit: unit.to_owned(),
            count,
        };
        MadeFile {
            kind,
            name,
            file_name,
            media_type: media_type.to_owned(),
            bytes: file.bytes,
            previews: sha256.is_some() && described.previews,
            sha256,
            parts: described.parts.map(|_| {
                Told::from(Sentence::FileParts {
                    unit: unit.to_owned(),
                    count,
                })
            }),
            kept: kept.into(),
            sentence: sentence.into(),
        }
    }

    /// What the file `sha256` in `scope` is, described from its content: read once, then kept
    /// among the [`KEPT`] read most lately. Nothing known when it can't be read.
    async fn made_described(&self, scope: &Scope, sha256: &Sha256) -> Described {
        let reading = &self.inner.reading;
        let cached = {
            let kept = reading
                .described
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            let found = kept.iter().find(|(kept, _)| kept == sha256);
            found.map(|(_, described)| described.clone())
        };
        if let Some(described) = cached {
            return described;
        }
        let read = self.read_made(scope, sha256, |bytes, media_type| {
            Some(glave_office::describe(&bytes, &media_type))
        });
        let described = read.await.ok().flatten().unwrap_or_default();
        let mut kept = reading
            .described
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if kept.len() >= KEPT {
            kept.pop_front();
        }
        kept.push_back((sha256.clone(), described.clone()));
        described
    }

    /// `reads` of the file `sha256` in `scope`, its bytes and media type, off the async
    /// threads, one of at most [`AT_ONCE`] at a time and within [`WAIT`]. "No longer here" when
    /// it is not kept; `None` when `reads` reads nothing.
    async fn read_made<T: Send + 'static>(
        &self,
        scope: &Scope,
        sha256: &Sha256,
        reads: impl FnOnce(Vec<u8>, String) -> Option<T> + Send + 'static,
    ) -> Result<Option<T>, ApiError> {
        let too_slow = || {
            let waited = spoken(WAIT);
            ApiError::new(code::LIMIT, Sentence::PreviewTooSlow { waited })
        };
        let permits = &self.inner.reading.permits;
        let _permit = tokio::time::timeout(WAIT, permits.acquire())
            .await
            .map_err(|_| too_slow())?;
        let found = self.inner.harness.blobs().get(scope, sha256).await;
        let found = found.map_err(|error| ApiError::internal(&error))?;
        let mut found = found.ok_or_else(not_here)?;
        let bytes = read_all(found.bytes.as_mut(), MAX_FILE).await;
        let bytes = bytes.map_err(|error| ApiError::internal(&error))?;
        let media_type = found.reference.media_type;
        // Reading a workbook or a document's XML takes a while for a large one.
        let reading = tokio::task::spawn_blocking(move || reads(bytes, media_type));
        match tokio::time::timeout(WAIT, reading).await {
            Ok(Ok(read)) => Ok(read),
            Ok(Err(_)) => Ok(None),
            Err(_) => Err(too_slow()),
        }
    }
}

/// Reads the next event of a thread's log into `turns`: each turn's start, and the files its
/// end says it made.
fn read(turns: &mut Vec<(Seq, Vec<FileMade>)>, event: RawEvent) {
    let seq = event.seq;
    if !matches!(event.kind.as_str(), "turn.started" | "turn.ended") {
        return;
    }
    match decode(event).map(|event| event.body) {
        Ok(Body::TurnStarted(_)) => turns.push((seq, Vec::new())),
        Ok(Body::TurnEnded(ended)) => {
            if let Some((_, files)) = turns.last_mut() {
                *files = ended.made;
            }
        }
        _ => {}
    }
}

/// `file_name` without the end that says its kind: `Trip budget.xlsx` is `Trip budget`.
fn stem(file_name: &str) -> String {
    file_name
        .rsplit_once('.')
        .map_or(file_name, |(stem, _)| stem)
        .to_owned()
}

/// The word a sentence chooses its kind of file by.
fn kind_word(kind: AssetKind) -> &'static str {
    match kind {
        AssetKind::Document => "document",
        AssetKind::Spreadsheet => "spreadsheet",
        AssetKind::Deck => "deck",
        AssetKind::Pdf => "pdf",
        AssetKind::Picture | AssetKind::Other => "other",
    }
}

#[cfg(test)]
mod tests;
