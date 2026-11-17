//! Files a thread's turns made for the person (`specs/session.md`, "Files it made"), read from
//! its log: each `turn.ended` names the files its turn made (`glave_event::FileMade`), kept with
//! the thread or left on the computer, with what glave read of each in a process of its own,
//! and a `turn.made` what more a turn's look-over found once the thread's commands in the
//! background had ended. `thread/files` gives them by turn, each path once, `file/preview` a
//! file's preview as its turn kept it. Nothing here reads a file the agent made.

use glave_event::{
    AssetKind, Body, FileMade, Kept, MadeLeft, MadePreview, Parts, RawEvent, Seq, Sha256, ThreadId,
    decode,
};
use glave_harness::MAX_FILE;
use glave_ports::{BlobStore, EventLog, ModelProvider, read_all};

use crate::files::not_here;
use crate::protocol::{ApiError, FilePreview, MadeFile, Reply, Told, TurnFiles, code};
use crate::server::Server;
use crate::words::Sentence;

/// The most of a preview read back, in bytes: more than any preview holds (256 KB).
const MOST_PREVIEW: u64 = 1024 * 1024;

/// What a turn made, as the thread's log names it.
struct Turn {
    /// Its `turn.started`.
    started: Seq,
    files: Vec<FileMade>,
    left: Option<MadeLeft>,
}

impl<P, L, B> Server<P, L, B>
where
    P: ModelProvider + 'static,
    L: EventLog + 'static,
    B: BlobStore + 'static,
{
    /// The files `thread`'s turns made, as its log names them.
    pub(crate) async fn made_files(&self, thread: ThreadId) -> Result<Reply, ApiError> {
        let made = self
            .made_by_turns(thread)
            .await?
            .into_iter()
            .filter(|turn| !turn.files.is_empty() || turn.left.is_some())
            .map(|turn| TurnFiles {
                turn: turn.started,
                files: turn.files.iter().map(made_file).collect(),
                note: turn.left.map(|left| Told::from(left_out(left))),
            })
            .collect();
        Ok(Reply::Files { made })
    }

    /// Answers `file/preview`: the preview `thread`'s turn kept of the file `sha256` it made;
    /// refused for a file it keeps none of.
    pub(crate) async fn preview_file(
        &self,
        thread: ThreadId,
        sha256: &Sha256,
    ) -> Result<Reply, ApiError> {
        let scope = self.holding(thread, sha256).await?;
        let no_preview = || ApiError::new(code::INVALID_PARAMS, Sentence::NoPreview);
        let MadePreview { blob, cut } = self
            .made_by_turns(thread)
            .await?
            .into_iter()
            .flat_map(|turn| turn.files)
            .find_map(|file| match (file.kept, file.read) {
                (Kept::Blob(kept), Some(read)) if kept.sha256 == *sha256 => read.preview,
                _ => None,
            })
            .ok_or_else(no_preview)?;
        let found = self.inner.harness.blobs().get(&scope, &blob.sha256).await;
        let mut found = found
            .map_err(|error| ApiError::internal(&error))?
            .ok_or_else(not_here)?;
        let bytes = read_all(found.bytes.as_mut(), MOST_PREVIEW).await;
        let bytes = bytes.map_err(|error| ApiError::internal(&error))?;
        let markdown = String::from_utf8(bytes).map_err(|_| no_preview())?;
        Ok(Reply::FilePreview {
            preview: FilePreview {
                markdown,
                cut: cut.then(|| Sentence::PreviewCut.into()),
            },
        })
    }

    /// What each of `thread`'s turns made, as its log names it, in order.
    async fn made_by_turns(&self, thread: ThreadId) -> Result<Vec<Turn>, ApiError> {
        let mut turns = Vec::new();
        self.read_all(thread, |event| read(&mut turns, event))
            .await?;
        Ok(turns)
    }
}

/// Reads the next event of a thread's log into `turns`: each turn's start, the files its end
/// says it made, and what a turn whose look-over waited made, once looked over, with it: a file
/// found again in place of where it was first named.
fn read(turns: &mut Vec<Turn>, event: RawEvent) {
    let started = event.seq;
    if !matches!(
        event.kind.as_str(),
        "turn.started" | "turn.ended" | "turn.made"
    ) {
        return;
    }
    match decode(event).map(|event| event.body) {
        Ok(Body::TurnStarted(_)) => turns.push(Turn {
            started,
            files: Vec::new(),
            left: None,
        }),
        Ok(Body::TurnEnded(ended)) => {
            if let Some(last) = turns.last_mut() {
                last.files = ended.made;
                last.left = ended.made_left;
            }
        }
        Ok(Body::TurnMade(made)) => {
            if let Some(looked) = turns.iter_mut().find(|turn| turn.started == made.turn) {
                for file in made.made {
                    match looked
                        .files
                        .iter_mut()
                        .find(|named| named.path == file.path)
                    {
                        Some(named) => *named = file,
                        None => looked.files.push(file),
                    }
                }
                looked.left = made.made_left.or(looked.left);
            }
        }
        _ => {}
    }
}

/// `file` as `thread/files` gives it: named by its title, else its file's name, and said where
/// it is kept, or why it isn't, and why it can't be shown when glave couldn't read it.
fn made_file(file: &FileMade) -> MadeFile {
    let file_name = file
        .path
        .rsplit('/')
        .next()
        .unwrap_or(&file.path)
        .to_owned();
    let media_type = AssetKind::made_as(&file.path).unwrap_or_default();
    let kind = AssetKind::of(media_type);
    let (sha256, kept) = match &file.kept {
        Kept::Blob(blob) => (Some(blob.sha256.clone()), Sentence::KeptWithChat),
        Kept::TooLarge => (
            None,
            Sentence::LeftTooLarge {
                megabytes: MAX_FILE / (1024 * 1024),
            },
        ),
        Kept::Secret => (None, Sentence::LeftSecret),
        Kept::Unchecked => (None, Sentence::LeftUnchecked),
    };
    let unread = sha256.is_some() && file.read.is_none();
    let read = file.read.clone().unwrap_or_default();
    let name = read.title.unwrap_or_else(|| cleaned(&file_name));
    let (unit, count) = match read.parts {
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
    // A PDF is drawn by the browser; any other file opens as the preview its turn kept.
    let shown = read.preview.is_some() || kind == AssetKind::Pdf;
    MadeFile {
        kind,
        name,
        file_name,
        media_type: media_type.to_owned(),
        bytes: file.bytes,
        previews: sha256.is_some() && !unread && shown,
        sha256,
        parts: read.parts.map(|_| {
            Told::from(Sentence::FileParts {
                unit: unit.to_owned(),
                count,
            })
        }),
        kept: kept.into(),
        unread: unread.then(|| Sentence::MadeUnread.into()),
        sentence: sentence.into(),
    }
}

/// What the files a turn made leave out, in a sentence.
fn left_out(left: MadeLeft) -> Sentence {
    match left {
        MadeLeft::More { files } => Sentence::MadeMore { files },
        MadeLeft::TooMany => Sentence::MadeTooMany,
    }
}

/// `file_name` as a name to call a file by: without the end that says its kind, its
/// underscores spaces, each space single (`Trip_budget  2026.xlsx` is `Trip budget 2026`).
fn cleaned(file_name: &str) -> String {
    let stem = file_name
        .rsplit_once('.')
        .map_or(file_name, |(stem, _)| stem)
        .replace('_', " ");
    let cleaned = stem.split_whitespace().collect::<Vec<_>>().join(" ");
    if cleaned.is_empty() {
        file_name.to_owned()
    } else {
        cleaned
    }
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
