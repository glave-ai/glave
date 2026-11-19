//! Files a turn made for the person (`specs/session.md`, "Files it made"). As a turn ends that
//! called a tool that can change files (glave's own `write`, `edit` or `bash`, or a worker on a
//! computer that never sleeps), the thread's folder is looked over once (`files/glob`): each
//! document, spreadsheet, deck, PDF and e-book in no hidden folder, changed in its window, counts
//! as made, whatever made it. The window runs from just after where the last look-over stopped
//! (its turn's start less a little, for a computer whose clock is behind, when that is later) up
//! to now; while a command the thread started in the background still runs, up to a little
//! before now, so nothing still being written is kept, and the look-over is owed the rest (a
//! mark): a `turn.made`, once the last such command ends while the thread is idle, or as the
//! next turn starts, else the next working turn's end. Each file is kept for the thread and read
//! apart from glave (`keep.rs`, [`glave_ports::ReadMade`]) within one deadline for the whole
//! look-over, or until the turn that ends with it is stopped. Where it stands is read again from
//! the log (`log.rs`). At most [`MOST_LOOKING`] look-overs run at once on a server, and
//! [`LOOKING_PER_ORG`] for one organization.

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
use crate::cancel::Cancel;
use crate::harness::{Harness, HarnessError, Thread};
use crate::shares::Shares;
use keep::Reading;

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

/// The most look-overs that run at once on a server: each walks a folder and reads up to
/// [`MOST`] files of 10 MB.
pub(crate) const MOST_LOOKING: usize = 4;

/// The most look-overs that run at once for one organization, so no one's turns ending at once
/// hold up everyone's.
pub(crate) const LOOKING_PER_ORG: usize = 2;

/// The longest a look-over waits for its place among the server's: past it, it is owed, as when
/// its folder can't be looked over.
const WAITS_TO_LOOK: Duration = Duration::from_secs(60);

/// The longest the files of one look-over are read for, in all: past it, the rest are kept
/// unread, so no turn or request waits on readers for minutes.
pub(crate) const READING_FOR: Duration = Duration::from_secs(30);

/// What a look-over found: the files kept, what they leave out, where it stopped, and whether
/// what changed after that is still owed a look (a command of the thread's still ran, or it
/// couldn't look).
#[derive(Debug, Default)]
pub(crate) struct Found {
    pub(crate) made: Vec<FileMade>,
    pub(crate) left: Option<MadeLeft>,
    pub(crate) until_ns: Option<u64>,
    pub(crate) waits: bool,
}

/// The places for look-overs on one server: [`MOST_LOOKING`] at once, [`LOOKING_PER_ORG`] for one
/// organization.
pub(crate) fn looking() -> Shares {
    Shares::new(MOST_LOOKING, LOOKING_PER_ORG)
}

impl Thread {
    /// Whether a look over what its last working turn made is owed and may be made now, while it
    /// is idle: the turn was stopped, or its folder couldn't be looked over, and no command of its
    /// runs in the background, whose end would make it; archived too, whose files are kept with
    /// it, but never once deleted, nor on its person's own computer, which looking would wake
    /// (`made.rs`).
    pub fn owes_look(&self) -> bool {
        self.owed_mark().is_some()
    }

    /// Its mark, when the look it owes may be made now ([`Thread::owes_look`]).
    pub(crate) fn owed_mark(&self) -> Option<Mark> {
        let looks = self.home.is_none()
            && self.standing != Standing::Deleted
            && !self.commands.any_running();
        self.made_mark.filter(|mark| mark.owed && looks)
    }
}

impl<P, L, B, A> Harness<P, L, B, A>
where
    P: ModelProvider,
    L: EventLog,
    B: BlobStore,
    A: Approver,
{
    /// The files `thread`'s turn made, as it ends: none unless it called a tool that can change
    /// files there. Where it stops, and whether the rest is owed, becomes the thread's mark. Read
    /// apart from glave until the look-over's deadline; once `cancel` stops the turn, the look is
    /// owed, made once the turn has ended (`Thread::owes_look`) or at the next turn's start.
    pub(crate) async fn made_by_turn(&self, thread: &mut Thread, cancel: &Cancel) -> Found {
        let Some(turn) = thread.turn_open.take().filter(|turn| turn.works) else {
            return Found::default();
        };
        // A worker's files are its parent's, which looks over the same folder.
        if thread.delegated() {
            return Found::default();
        }
        let started = u64::try_from(turn.at.timestamp().as_nanosecond()).unwrap_or(0);
        let since = match thread.made_mark {
            Some(mark) if mark.owed => mark.since_ns,
            looked => started
                .saturating_sub(nanos(MARGIN))
                .max(looked.map_or(0, |mark| mark.since_ns)),
        };
        let reading = self.reading(cancel.clone());
        let found = self
            .look_over(thread, since, &reading)
            .await
            .unwrap_or(Found {
                until_ns: Some(since),
                waits: true,
                ..Found::default()
            });
        thread.made_mark = found.until_ns.map(|since_ns| Mark {
            turn: turn.turn,
            since_ns,
            owed: found.waits,
        });
        found
    }

    /// Stages in `events` what the look-over a turn's mark owes found (`turn.made`), once no
    /// command of the thread's runs in the background: from the mark up to now, for that turn.
    /// Nothing when the folder can't be looked over, or `cancel` stops it, the mark staying
    /// owed. Never on a person's
    /// own computer, which looking would wake: the next turn that works there looks.
    pub(crate) async fn stage_marked(
        &self,
        thread: &mut Thread,
        events: &mut Vec<RawEvent>,
        cancel: &Cancel,
    ) -> Result<(), HarnessError> {
        let Some(mark) = thread.owed_mark() else {
            return Ok(());
        };
        let reading = self.reading(cancel.clone());
        let Some(found) = self.look_over(thread, mark.since_ns, &reading).await else {
            return Ok(());
        };
        let made = TurnMade {
            turn: mark.turn,
            made_until_ns: found.until_ns,
            made: found.made,
            made_left: found.left,
        };
        self.stage(thread, events, &made)?;
        thread.made_mark = found.until_ns.map(|since_ns| Mark {
            turn: mark.turn,
            since_ns,
            owed: found.waits,
        });
        Ok(())
    }

    /// The files in `thread`'s folder changed after `since_ns`, by the computer's clock, each
    /// kept or said why not, in path order: at most [`MOST`], the rest counted; and where it
    /// stopped. While a command of the thread's still runs, only those settled for [`MARGIN`],
    /// the rest owed. None, said so, when the folder holds too many files to look over in time;
    /// `None` when it can't be looked over, its place among the server's look-overs doesn't come
    /// in time, or `reading`'s stop comes first: the look is owed then.
    async fn look_over(&self, thread: &Thread, since_ns: u64, reading: &Reading) -> Option<Found> {
        let stop = &reading.cancel;
        let place = self.looking.share(thread.scope.org());
        let _place = stop
            .run(tokio::time::timeout(WAITS_TO_LOOK, place))
            .await?
            .ok()??;
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
        let found = stop
            .run(self.workspace_for(thread).glob(&glob))
            .await?
            .ok()?;
        let until_ns = found.cut_off_ns;
        if found.stopped {
            return Some(Found {
                left: Some(MadeLeft::TooMany),
                until_ns,
                waits: running,
                ..Found::default()
            });
        }
        let listed = u64::try_from(found.files.len()).unwrap_or(u64::MAX);
        let mut more = found.total.saturating_sub(listed);
        // Walked in no order, so a large folder is never read whole first: named in path order.
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
            } else if let Some(file) = stop
                .run(self.keep_made_file(thread, path, Some(reading)))
                .await?
            {
                made.push(file);
            }
        }
        Some(Found {
            made,
            left: (more > 0).then_some(MadeLeft::More { files: more }),
            until_ns,
            waits: running,
        })
    }
}

impl<P, L, B, A> Harness<P, L, B, A>
where
    P: ModelProvider,
    L: EventLog,
    B: BlobStore,
    A: Approver,
{
    /// How long a look-over starting now reads its files: until its deadline, or until `cancel`
    /// stops what it ends with.
    fn reading(&self, cancel: Cancel) -> Reading {
        Reading {
            until: Instant::now() + self.reading_for,
            cancel,
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
