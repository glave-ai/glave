//! A person's computer in the cloud keeps its disk from filling (`specs/computers.md`, "A
//! person's computer in the cloud", decision 7). Its disk is its budget: the volume the
//! person's home is on. Once a minute, while no request runs and nothing runs in the
//! background, if more than [`FULL_PERCENT`] of it is used, glave frees what can be rebuilt,
//! the oldest first, until it is below again: build outputs git ignores at the root of the
//! worktrees glave made, then the caches commands fill, which a later command fetches again,
//! and last what the Trash holds. Only what has gone untouched for [`UNTOUCHED`], never a link
//! or what is on another disk, never the person's own files, a project folder or a worktree
//! itself. Each removal, and each time it could not keep the disk, is a line in glave's
//! folder ([`RECORD`]). A pass that ended still full waits [`AFTER_FULL`] for the next.

#[cfg(test)]
mod tests;

use std::convert::Infallible;
use std::ffi::OsString;
use std::os::unix::fs::MetadataExt as _;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use glave_ports::computer::ComputerError;
use glave_ports::{Clock, SystemClock};
use glave_process::Processes;
use tokio::io::AsyncWriteExt as _;

use crate::local::git::Git;
use crate::worktrees::{Trash, folders};

/// Above this share of the disk used, in percent, what can be rebuilt is freed.
const FULL_PERCENT: u64 = 80;

/// How often the disk is looked at.
const EVERY: Duration = Duration::from_mins(1);

/// How long after a pass that ended still full the next one waits: the person's own files
/// fill it, and freeing outputs and caches again each minute would only slow them.
const AFTER_FULL: Duration = Duration::from_hours(1);

/// How long something must have gone untouched to be freed, so what a thread uses now stays.
const UNTOUCHED: Duration = Duration::from_hours(1);

/// The file in glave's folder that says what was freed, a line each.
const RECORD: &str = "disk.log";

/// How large the record grows before it is moved aside (`disk.log.1`), the one before gone.
const RECORD_MOST: u64 = 1024 * 1024;

/// Build outputs a worktree may hold at its root, freed when git ignores them and tracks
/// nothing in them.
const BUILT: [&str; 5] = ["target", "node_modules", ".venv", "dist", "build"];

/// Caches in the person's home that commands fill and fetch again: npm's and cargo's
/// downloads; then each one in the cache home (pip's and uv's among them).
const CACHES: [&str; 2] = [".npm/_cacache", ".cargo/registry/cache"];

/// The most things one pass looks at, of each kind.
const MOST_LOOKED_AT: usize = 1_000;

/// How much a disk holds and how much of it is free, in bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Space {
    size: u64,
    available: u64,
}

impl Space {
    /// Whether more than [`FULL_PERCENT`] of it is used.
    fn full(self) -> bool {
        let used = self.size.saturating_sub(self.available);
        used.saturating_mul(100) > self.size.saturating_mul(FULL_PERCENT)
    }
}

/// The space on the disk `path` is on, as the system says.
fn space(path: &Path) -> std::io::Result<Space> {
    let stats = rustix::fs::statvfs(path)?;
    Ok(Space {
        size: stats.f_blocks.saturating_mul(stats.f_frsize),
        available: stats.f_bavail.saturating_mul(stats.f_frsize),
    })
}

/// What a thing freed was, as the record says it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Built,
    Cache,
    Trash,
}

impl Kind {
    fn word(self) -> &'static str {
        match self {
            Self::Built => "built",
            Self::Cache => "cache",
            Self::Trash => "trash",
        }
    }
}

/// One thing that may be freed: where it is, what it is, and the Trash's note on it.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Candidate {
    path: PathBuf,
    kind: Kind,
    note: Option<PathBuf>,
}

/// How a keeper reads the space on a disk.
type Measure = Box<dyn Fn(&Path) -> std::io::Result<Space> + Send + Sync>;

/// What keeps a person's computer's disk from filling, beside the computer it serves.
pub struct Keeper {
    /// The person's home, on the disk kept.
    home: PathBuf,
    /// glave's folder there, where the record is kept.
    glave: PathBuf,
    /// Where the worktrees glave made are, one folder per repository.
    worktrees: PathBuf,
    /// The caches' home: `$XDG_CACHE_HOME`, else `~/.cache`.
    caches: PathBuf,
    trash: Option<Trash>,
    processes: Processes,
    environment: Vec<(OsString, OsString)>,
    /// Whether anything runs there in the background, which nothing is freed under.
    busy: Box<dyn Fn() -> bool + Send + Sync>,
    clock: Box<dyn Clock>,
    measure: Measure,
}

impl Keeper {
    /// A keeper of the disk the person's home is on, in `environment` (`HOME` and the XDG
    /// folders read from it), recording in glave's folder `glave`, freeing build outputs in
    /// the worktrees under `worktrees`, running git as one of `processes`, never while
    /// `busy`.
    pub fn new(
        glave: PathBuf,
        worktrees: PathBuf,
        processes: Processes,
        environment: Vec<(OsString, OsString)>,
        busy: Box<dyn Fn() -> bool + Send + Sync>,
    ) -> Self {
        let read = |name: &str| {
            environment
                .iter()
                .find(|(key, _)| key == name)
                .map(|(_, value)| PathBuf::from(value))
        };
        let home = read("HOME").unwrap_or_else(|| glave.clone());
        let caches = read("XDG_CACHE_HOME").unwrap_or_else(|| home.join(".cache"));
        let trash = Trash::for_home(&home, read("XDG_DATA_HOME").as_deref(), "");
        Self {
            home,
            glave,
            worktrees,
            caches,
            trash,
            processes,
            environment,
            busy,
            clock: Box::new(SystemClock),
            measure: Box::new(space),
        }
    }

    /// Keeps the disk until dropped: every [`EVERY`], the first a minute after it starts,
    /// while `requests` says none runs and nothing runs in the background.
    #[expect(
        clippy::infinite_loop,
        reason = "it keeps the disk until the server beside it ends, which drops it"
    )]
    pub(crate) async fn keep_quietly(&self, requests: impl Fn() -> bool + Sync) -> Infallible {
        let quiet = || requests() && !(self.busy)();
        let mut wait = EVERY;
        loop {
            tokio::time::sleep(wait).await;
            wait = EVERY;
            if !quiet() {
                continue;
            }
            match self.keep(&quiet).await {
                Ok(true) => {}
                Ok(false) => wait = AFTER_FULL,
                Err(error) => {
                    self.note(&format!("could not keep the disk: {error}"))
                        .await;
                }
            }
        }
    }

    /// While the disk is full and `quiet` says nothing runs, frees what can be rebuilt, in
    /// order: whether there is room now, or it stopped for something that runs.
    async fn keep(&self, quiet: &(dyn Fn() -> bool + Sync)) -> Result<bool, ComputerError> {
        if !self.full()? {
            return Ok(true);
        }
        for candidate in self.candidates().await {
            if !quiet() {
                return Ok(true);
            }
            let Candidate { path, kind, note } = candidate;
            match remove(&path, note.as_deref()).await {
                Ok(()) => {
                    self.note(&format!("{} {}", kind.word(), path.display()))
                        .await;
                }
                Err(error) => {
                    let line = format!("could not free {}: {error}", path.display());
                    self.note(&line).await;
                    continue;
                }
            }
            if !self.full()? {
                return Ok(true);
            }
        }
        Ok(false)
    }

    /// Whether the disk is full now.
    fn full(&self) -> Result<bool, ComputerError> {
        (self.measure)(&self.home)
            .map(Space::full)
            .map_err(|error| {
                ComputerError::Failed(format!("could not read how full the disk is: {error}"))
            })
    }

    /// What may be freed, in order, each the oldest first: build outputs, caches, the Trash.
    async fn candidates(&self) -> Vec<Candidate> {
        let disk = tokio::fs::metadata(&self.home)
            .await
            .ok()
            .map(|found| found.dev());
        let now = SystemTime::from(self.clock.now());
        let mut built = Vec::new();
        for repository in folders(&self.worktrees).await.unwrap_or_default() {
            for worktree in folders(&repository).await.unwrap_or_default() {
                if built.len() >= MOST_LOOKED_AT {
                    break;
                }
                built.extend(self.outputs(&worktree).await);
            }
        }
        let mut caches: Vec<PathBuf> = CACHES.iter().map(|cache| self.home.join(cache)).collect();
        caches.extend(folders(&self.caches).await.unwrap_or_default());
        let trash = match &self.trash {
            Some(trash) => trash.held(MOST_LOOKED_AT).await,
            None => Vec::new(),
        };
        let mut found = Vec::new();
        for (paths, kind) in [(built, Kind::Built), (caches, Kind::Cache)] {
            let dated = paths.into_iter().map(|path| (path.clone(), path));
            for path in untouched(dated, disk, now).await {
                found.push(Candidate {
                    path,
                    kind,
                    note: None,
                });
            }
        }
        // The Trash by when things went into it, as its notes say.
        let dated = trash
            .into_iter()
            .map(|(path, note)| (note.clone().unwrap_or_else(|| path.clone()), (path, note)));
        for (path, note) in untouched(dated, disk, now).await {
            found.push(Candidate {
                path,
                kind: Kind::Trash,
                note,
            });
        }
        found
    }

    /// The build outputs at the root of `worktree`, a worktree git knows, that git ignores
    /// and tracks nothing in; none when git can't say.
    async fn outputs(&self, worktree: &Path) -> Vec<PathBuf> {
        if tokio::fs::symlink_metadata(worktree.join(".git"))
            .await
            .is_err()
        {
            return Vec::new();
        }
        let git = Git::at(&self.processes, &self.environment, worktree);
        let mut asked = vec!["check-ignore", "--"];
        asked.extend(BUILT);
        let Ok(ignored) = git.run(&asked).await else {
            return Vec::new();
        };
        let mut found = Vec::new();
        for name in ignored
            .lines()
            .map(str::trim)
            .filter(|name| BUILT.contains(name))
        {
            let tracked = git.run(&["ls-files", "--", name]).await;
            if matches!(tracked, Ok(listed) if listed.trim().is_empty()) {
                found.push(worktree.join(name));
            }
        }
        found
    }

    /// Writes `line` to the record, at the time the clock says, moving a full one aside
    /// first; a record that could not be written is no reason to keep what was freed.
    async fn note(&self, line: &str) {
        let record = self.glave.join(RECORD);
        if tokio::fs::metadata(&record)
            .await
            .is_ok_and(|found| found.len() >= RECORD_MOST)
        {
            let aside = self.glave.join(format!("{RECORD}.1"));
            let _moved = tokio::fs::rename(&record, aside).await;
        }
        let opened = tokio::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&record)
            .await;
        if let Ok(mut file) = opened {
            let line = format!("{} {line}\n", self.clock.now());
            // Flushed before the file goes: tokio writes in the background, and the next
            // line's own handle would otherwise land first.
            let _written = file.write_all(line.as_bytes()).await;
            let _flushed = file.flush().await;
        }
    }
}

/// Removes `path`, a folder or a file itself, never what a link leads to; with `note`, the
/// Trash's note on it.
async fn remove(path: &Path, note: Option<&Path>) -> std::io::Result<()> {
    if tokio::fs::symlink_metadata(path).await?.is_dir() {
        tokio::fs::remove_dir_all(path).await?;
    } else {
        tokio::fs::remove_file(path).await?;
    }
    if let Some(note) = note {
        let _gone = tokio::fs::remove_file(note).await;
    }
    Ok(())
}

/// Of `found`, each dated by its path and kept as its value, those that are no link, are on
/// the disk `disk` and went untouched for [`UNTOUCHED`] before `now`: the oldest first, in
/// the order given where two were touched at once.
async fn untouched<T>(
    found: impl Iterator<Item = (PathBuf, T)>,
    disk: Option<u64>,
    now: SystemTime,
) -> Vec<T> {
    let mut dated = Vec::new();
    for (dated_by, kept) in found.take(MOST_LOOKED_AT) {
        let Ok(found) = tokio::fs::symlink_metadata(&dated_by).await else {
            continue;
        };
        let touched = found.modified().unwrap_or(SystemTime::UNIX_EPOCH);
        let old = now
            .duration_since(touched)
            .is_ok_and(|age| age >= UNTOUCHED);
        if old && !found.is_symlink() && Some(found.dev()) == disk {
            dated.push((touched, kept));
        }
    }
    dated.sort_by_key(|(touched, _)| *touched);
    dated.into_iter().map(|(_, kept)| kept).collect()
}
