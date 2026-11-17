//! Files a turn made for the person (`specs/session.md`, "Files it made"). As a turn ends that
//! called a tool that can change files (glave's own `write`, `edit` or `bash`, or a worker on a
//! computer that never sleeps), the thread's folder is looked over once (`files/glob`): each
//! document, spreadsheet, deck, PDF and e-book in no hidden folder, changed in its window, counts
//! as made, whatever made it. The window runs from where the last look-over stopped, else the
//! turn's start, up to now; while a command the thread started in the background still runs, up
//! to a little before now, so nothing still being written is kept, and where it stopped is
//! recorded (`made_until_ns`) for the next to start from: a `turn.made` once the last such
//! command ends while the thread is idle, or the next working turn's end. Each file is kept for
//! the thread (`keep.rs`) and read apart from glave ([`glave_ports::ReadMade`]) within one
//! deadline for the whole look-over. Where it stands is read again from the log (`log.rs`).

mod keep;
mod log;
mod scan;

use std::time::Duration;

use glave_event::AssetKind;
use glave_event::{FileMade, MadeLeft, RawEvent, TurnMade};
use glave_ports::computer::Glob;
use glave_ports::{BlobStore, EventLog, ModelProvider, Standing};
use tokio::time::Instant;

use crate::approval::Approver;
use crate::harness::{Harness, HarnessError, Thread};

pub(crate) use log::{Mark, TurnOpen, changes_files, from_log, opened_by};

/// The most files one turn keeps: past them, the rest stay on the computer, counted.
pub(crate) const MOST: usize = 8;

/// The most files a look-over finds.
const MOST_FOUND: usize = 1_000;

/// The most entries a look-over looks at, folders too: past them, or a few seconds, it names
/// nothing.
const MOST_LOOKED_AT: usize = 50_000;

/// How far before its turn started a look-over reaches, for a computer whose clock is a little
/// behind the server's; and how far before now it stops while a command still runs, so a file
/// still being written is left for the next.
const MARGIN: Duration = Duration::from_secs(2);

/// The longest the files of one look-over are read for, in all: past it, the rest are kept
/// unread, so no turn or request waits on readers for minutes.
pub(crate) const READING_FOR: Duration = Duration::from_secs(30);

/// What a look-over found: the files kept, what they leave out, and where it stopped while a
/// command of the thread's still ran.
#[derive(Debug, Default)]
pub(crate) struct Found {
    pub(crate) made: Vec<FileMade>,
    pub(crate) left: Option<MadeLeft>,
    pub(crate) until_ns: Option<u64>,
}

impl<P, L, B, A> Harness<P, L, B, A>
where
    P: ModelProvider,
    L: EventLog,
    B: BlobStore,
    A: Approver,
{
    /// The files `thread`'s turn made, as it ends: none unless it called a tool that can change
    /// files there. Read apart from glave unless the turn was stopped (`read` false), when they
    /// are kept unread.
    pub(crate) async fn made_by_turn(&self, thread: &mut Thread, read: bool) -> Found {
        let Some(turn) = thread.turn_open.take().filter(|turn| turn.works) else {
            return Found::default();
        };
        // A worker's files are its parent's, which looks over the same folder.
        if thread.delegated() {
            return Found::default();
        }
        let started = u64::try_from(turn.at.timestamp().as_nanosecond()).unwrap_or(0);
        let since = thread.made_mark.map_or_else(
            || started.saturating_sub(nanos(MARGIN)),
            |mark| mark.since_ns,
        );
        let found = self.look_over(thread, since, read).await;
        thread.made_mark = found.until_ns.map(|since_ns| Mark {
            turn: turn.turn,
            since_ns,
        });
        found
    }

    /// Stages in `events` what the look-over a turn's mark left made (`turn.made`), once no
    /// command of the thread's runs in the background and the thread is idle: from the mark up
    /// to now, for that turn. Never on a person's own computer, which looking would wake: the
    /// next turn that works there looks.
    pub(crate) async fn stage_marked(
        &self,
        thread: &mut Thread,
        events: &mut Vec<RawEvent>,
    ) -> Result<(), HarnessError> {
        let looks = thread.home.is_none()
            && thread.standing == Standing::Active
            && !thread.commands.any_running();
        let Some(mark) = thread.made_mark.filter(|_| looks) else {
            return Ok(());
        };
        let found = self.look_over(thread, mark.since_ns, true).await;
        let made = TurnMade {
            turn: mark.turn,
            made: found.made,
            made_left: found.left,
            made_until_ns: found.until_ns,
        };
        self.stage(thread, events, &made)?;
        thread.made_mark = found.until_ns.map(|since_ns| Mark {
            turn: mark.turn,
            since_ns,
        });
        Ok(())
    }

    /// The files in `thread`'s folder changed since `since_ns`, by the computer's clock, each
    /// kept or said why not, in path order: at most [`MOST`], the rest counted. While a command
    /// of the thread's still runs, only those settled for [`MARGIN`], and where that stopped.
    /// None when the folder can't be looked over, the mark staying where it was while a command
    /// runs; none, said so, when it holds too many files to look over in time.
    async fn look_over(&self, thread: &Thread, since_ns: u64, read: bool) -> Found {
        if thread.commands.any_running() {
            thread.commands.check().await;
        }
        let running = thread.commands.any_running();
        let glob = Glob {
            pattern: AssetKind::made_pattern(),
            path: ".".to_owned(),
            limit: MOST_FOUND,
            changed_since_ns: Some(since_ns),
            visible: true,
            most_looked_at: Some(MOST_LOOKED_AT),
            settled_for_ns: running.then(|| nanos(MARGIN)),
        };
        let Ok(found) = self.workspace_for(thread).glob(&glob).await else {
            return Found {
                until_ns: running.then_some(since_ns),
                ..Found::default()
            };
        };
        let until_ns = found.cut_off_ns.filter(|_| running);
        if found.stopped {
            let left = Some(MadeLeft::TooMany);
            return Found {
                made: Vec::new(),
                left,
                until_ns,
            };
        }
        let listed = u64::try_from(found.files.len()).unwrap_or(u64::MAX);
        let mut more = found.total.saturating_sub(listed);
        let mut paths: Vec<String> = found
            .files
            .into_iter()
            .filter(|path| !locked(path))
            .collect();
        paths.sort();
        let reading = read.then(|| Instant::now() + self.reading_for);
        let mut made = Vec::new();
        for path in &paths {
            if made.len() >= MOST {
                more = more.saturating_add(1);
            } else if let Some(file) = self.keep_made_file(thread, path, reading).await {
                made.push(file);
            }
        }
        let left = (more > 0).then_some(MadeLeft::More { files: more });
        Found {
            made,
            left,
            until_ns,
        }
    }
}

/// `duration` in nanoseconds.
fn nanos(duration: Duration) -> u64 {
    u64::try_from(duration.as_nanos()).unwrap_or(u64::MAX)
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
