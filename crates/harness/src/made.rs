//! Files a turn made for the person (`specs/session.md`, "Files it made"). As a turn ends that
//! called a tool that can change files (glave's own `write`, `edit` or `bash`, or a worker on a
//! computer that never sleeps), the thread's folder is looked over once (`files/glob`): each
//! document, spreadsheet, deck, PDF and e-book in no hidden folder, changed since the turn
//! started, counts as made, whatever made it. While a command the thread started in the
//! background still runs, nothing is looked over: once the last ends, the folder is, since the
//! first turn that waited, and a `turn.made` names what it found for that turn. Each file is
//! read through the computer port, looked through for the person's secrets (`scan.rs`), kept for
//! the thread, sealed, and read apart from glave for its title, its parts and its preview
//! ([`ReadMade`]), one at a time; the model never sees it. Where the turn stands is read again
//! from the log whenever the thread is: nothing here lasts only in memory.

mod scan;

use std::time::Duration;

use glave_event::{
    AssetKind, At, Body, Event, FileMade, FileRead, Kept, MadeLeft, MadePreview, RawEvent, Seq,
    TurnMade,
};
use glave_ports::computer::{EntryKind, Glob};
use glave_ports::{BlobStore, EventLog, MadeRead, ModelProvider, Pieces, digest};

use crate::approval::Approver;
use crate::files::MAX_FILE;
use crate::harness::{HarnessError, Thread};
use crate::tools::Tool;
use scan::Scanned;

/// The most files one turn keeps: past them, the rest stay on the computer, counted.
pub(crate) const MOST: usize = 8;

/// The most files a look-over finds.
const MOST_FOUND: usize = 1_000;

/// The most entries a look-over looks at, folders too: past them, or a few seconds, it names
/// nothing.
const MOST_LOOKED_AT: usize = 50_000;

/// How long before its turn started a file may have changed and count, for a computer whose
/// clock is a little behind the server's.
const MARGIN: Duration = Duration::from_secs(2);

/// What a preview is kept as.
const MARKDOWN: &str = "text/markdown";

/// The turn open on a thread, as what it made is told: where its `turn.started` is, and whether
/// it has called a tool that can change files.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct TurnOpen {
    pub(crate) turn: Seq,
    pub(crate) at: At,
    pub(crate) works: bool,
}

/// The first turn whose look-over waits for the thread's commands in the background to end.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Waiting {
    pub(crate) turn: Seq,
    pub(crate) at: At,
}

/// The files a look-over kept, and what they leave out.
type Found = (Vec<FileMade>, Option<MadeLeft>);

impl<P, L, B, A> crate::harness::Harness<P, L, B, A>
where
    P: ModelProvider,
    L: EventLog,
    B: BlobStore,
    A: Approver,
{
    /// The files `thread`'s turn made, as it ends, with what they leave out: none unless it
    /// called a tool that can change files, and none yet while a command of the thread's still
    /// runs in the background. Staged in `events` first, what the first turn that waited made,
    /// once none runs: what this turn made is that turn's too. A person's own computer, which
    /// may sleep, is looked at only after a call of the turn's worked there.
    pub(crate) async fn made_by_turn(
        &self,
        thread: &mut Thread,
        events: &mut Vec<RawEvent>,
    ) -> Result<Found, HarnessError> {
        let turn = thread.turn_open.take().filter(|turn| turn.works);
        let awake = turn.is_some() || thread.home.is_none();
        // A worker's files are its parent's, which looks over the same folder.
        if thread.delegated() || !awake || (turn.is_none() && thread.made_waits.is_none()) {
            return Ok((Vec::new(), None));
        }
        if thread.commands.any_running() {
            thread.commands.check().await;
        }
        if thread.commands.any_running() {
            let Some(turn) = turn else {
                return Ok((Vec::new(), None));
            };
            thread.made_waits.get_or_insert(Waiting {
                turn: turn.turn,
                at: turn.at,
            });
            return Ok((Vec::new(), Some(MadeLeft::Waits)));
        }
        if self.stage_waited(thread, events, awake).await? {
            return Ok((Vec::new(), None));
        }
        match turn {
            Some(turn) => Ok(self.look_over(thread, turn.at).await),
            None => Ok((Vec::new(), None)),
        }
    }

    /// Stages in `events` what the first turn whose look-over waited made (`turn.made`), once
    /// none of the thread's commands runs in the background: true when one waited. Not on a
    /// person's own computer unless it is `awake`, as a call of a turn's left it: looking would
    /// wake it, so the next turn that works there looks.
    pub(crate) async fn stage_waited(
        &self,
        thread: &mut Thread,
        events: &mut Vec<RawEvent>,
        awake: bool,
    ) -> Result<bool, HarnessError> {
        let looks = !thread.commands.any_running() && (awake || thread.home.is_none());
        let Some(waiting) = thread.made_waits.filter(|_| looks) else {
            return Ok(false);
        };
        let (made, made_left) = self.look_over(thread, waiting.at).await;
        let made = TurnMade {
            turn: waiting.turn,
            made,
            made_left,
        };
        self.stage(thread, events, &made)?;
        thread.made_waits = None;
        Ok(true)
    }

    /// The files in `thread`'s folder changed since `since`, each kept or said why not, in path
    /// order: at most [`MOST`], the rest counted. None when the folder can't be looked over, and
    /// none, said so, when it holds too many files to look over in time.
    async fn look_over(&self, thread: &Thread, since: At) -> Found {
        let margin = i128::try_from(MARGIN.as_nanos()).unwrap_or(0);
        let since = since.timestamp().as_nanosecond().saturating_sub(margin);
        let glob = Glob {
            pattern: AssetKind::made_pattern(),
            path: ".".to_owned(),
            limit: MOST_FOUND,
            changed_since_ns: Some(u64::try_from(since).unwrap_or(0)),
            visible: true,
            most_looked_at: Some(MOST_LOOKED_AT),
        };
        let Ok(found) = self.workspace_for(thread).glob(&glob).await else {
            return (Vec::new(), None);
        };
        if found.stopped {
            return (Vec::new(), Some(MadeLeft::TooMany));
        }
        let listed = u64::try_from(found.files.len()).unwrap_or(u64::MAX);
        let mut more = found.total.saturating_sub(listed);
        let mut paths: Vec<String> = found
            .files
            .into_iter()
            .filter(|path| !locked(path))
            .collect();
        paths.sort();
        let mut made = Vec::new();
        for path in &paths {
            if made.len() >= MOST {
                more = more.saturating_add(1);
            } else if let Some(file) = self.keep_made_file(thread, path).await {
                made.push(file);
            }
        }
        (made, (more > 0).then_some(MadeLeft::More { files: more }))
    }

    /// The file at `path` in `thread`'s folder, kept for the thread when it is no larger than a
    /// thread keeps and holds none of the person's secrets, then read apart from glave. `None`
    /// when it isn't a plain file there now, or it changed while it was read, as one still
    /// being written does.
    async fn keep_made_file(&self, thread: &Thread, path: &str) -> Option<FileMade> {
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
        let values: Vec<Vec<u8>> = self
            .workspace
            .hidden()
            .variables()
            .values()
            .map(|value| value.expose().as_bytes().to_vec())
            .collect();
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
            Scanned::Holds => return Some(made(Kept::Secret, None)),
            Scanned::Unchecked => return Some(made(Kept::Unchecked, None)),
            Scanned::Clean => {}
        }
        let read = match &self.reader {
            Some(reader) => reader(bytes.clone(), media_type.to_owned()).await,
            None => None,
        };
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

    /// `bytes`, kept for `thread`, sealed, as `media_type`; `None`, logged, when they can't be.
    async fn keep_bytes(
        &self,
        thread: &Thread,
        media_type: &str,
        bytes: Vec<u8>,
    ) -> Option<glave_event::BlobRef> {
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

/// Whether a call of the tool `name` can change files in the thread's folder: glave's own
/// `write`, `edit` or `bash`, or a worker unless the thread works on its person's own computer
/// (`at_home`), which may sleep, and which the worker wakes with its own tools.
pub(crate) fn changes_files(name: &str, at_home: bool) -> bool {
    matches!(
        Tool::named(name),
        Some(Tool::Write | Tool::Edit | Tool::Bash)
    ) || (!at_home && name.trim().eq_ignore_ascii_case(crate::subagent::AGENT))
}

/// The turn `events` open, its `turn.started` the last of them: nothing called yet.
pub(crate) fn opened_by(events: &[RawEvent]) -> Option<TurnOpen> {
    events.last().map(|event| TurnOpen {
        turn: event.seq,
        at: event.at,
        works: false,
    })
}

/// Where a thread read from `events` stands, as what its turns made is told: the turn open
/// now, and the first turn whose look-over waits for the thread's commands in the background.
pub(crate) fn from_log(events: &[Event], at_home: bool) -> (Option<TurnOpen>, Option<Waiting>) {
    let (mut open, mut started, mut waiting) = (None, None, None);
    for event in events {
        let (turn, at) = (event.header.seq, event.header.at);
        match &event.body {
            Body::TurnStarted(_) => {
                started = Some(Waiting { turn, at });
                open = Some(TurnOpen {
                    turn,
                    at,
                    works: false,
                });
            }
            Body::ToolCallRequested(call) => {
                let works = changes_files(&call.tool, at_home);
                if let Some(open) = open.as_mut() {
                    open.works |= works;
                }
            }
            Body::TurnEnded(ended) => {
                if ended.made_left == Some(MadeLeft::Waits) && waiting.is_none() {
                    waiting = started;
                }
                open = None;
            }
            Body::TurnMade(made)
                if waiting.is_some_and(|waits: Waiting| waits.turn == made.turn) =>
            {
                waiting = None;
            }
            _ => {}
        }
    }
    (open, waiting)
}

/// Whether `path` names an Office lock file (`~$report.docx`), which Office keeps beside a file
/// it has open: never something made for the person.
fn locked(path: &str) -> bool {
    path.rsplit('/')
        .next()
        .is_some_and(|name| name.starts_with("~$"))
}

#[cfg(test)]
mod tests;
