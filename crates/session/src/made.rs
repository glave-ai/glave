//! Files a thread's turns made for the person (`specs/session.md`, "Files it made"), read from
//! its log: each `turn.ended` names the files its turn made (`glave_event::FileMade`), kept with
//! the thread or left on the computer, and a `turn.made` what more a turn's look-over found once
//! the thread's commands in the background had ended. `thread/files` gives them by turn, each
//! path once. Nothing here reads a file the agent made.

use glave_event::{AssetKind, Body, FileMade, Kept, MadeLeft, RawEvent, Seq, ThreadId, decode};
use glave_harness::MAX_FILE;
use glave_ports::{BlobStore, EventLog, ModelProvider};

use crate::protocol::{ApiError, MadeFile, Reply, Told, TurnFiles};
use crate::server::Server;
use crate::words::Sentence;

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

/// `file` as `thread/files` gives it: named by its file's name, and said where it is kept, or
/// why it isn't.
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
    let name = cleaned(&file_name);
    let sentence = Sentence::MadeLine {
        kind: kind_word(kind).to_owned(),
        name: name.clone(),
    };
    MadeFile {
        kind,
        name,
        file_name,
        media_type: media_type.to_owned(),
        bytes: file.bytes,
        sha256,
        kept: kept.into(),
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
