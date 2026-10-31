//! A person's computer in the cloud keeps its disk from filling (`specs/computers.md`, "A
//! person's computer in the cloud", decision 7). Its disk is its budget: the volume the
//! person's home is on. Once a minute, while no request runs and nothing runs in the
//! background, if more than 80% of it is used, glave frees what can be rebuilt,
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
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant, SystemTime};

use glave_ports::computer::{ComputerError, DiskSpace, DiskUse};
use glave_ports::{Clock, SystemClock};
use glave_process::Processes;
use tokio::io::AsyncWriteExt as _;

use crate::local::git::Git;
use crate::worktrees::{Trash, folders};

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
pub(crate) const BUILT: [&str; 5] = ["target", "node_modules", ".venv", "dist", "build"];

/// Caches in the person's home that commands fill and fetch again: npm's and cargo's
/// downloads; then each one in the cache home (pip's and uv's among them), and each in glave's
/// sandbox cache folder, where sandboxed commands keep theirs.
const CACHES: [&str; 2] = [".npm/_cacache", ".cargo/registry/cache"];

/// Folders of the sandbox's cache that hold more than what is fetched again (settings, installed
/// programs, modules that can't be removed), never freed whole.
const HOMES_IN_THE_SANDBOX: [&str; 3] = ["cargo", "rustup", "go"];

/// What in those is fetched again: rustup installs a toolchain it finds gone as it's used.
const FETCHED_IN_THE_SANDBOX: [&str; 2] = ["cargo/registry", "rustup/toolchains"];

/// The most things one pass looks at, of each kind.
const MOST_LOOKED_AT: usize = 1_000;

/// How many of what uses the disk most the person is told of.
const LARGEST: usize = 10;

/// The most files and folders one look at what uses the disk reads, and how long it takes at
/// most: past either it says what it saw, since an answer that's nearly right is what's wanted.
const MOST_WALKED: u64 = 2_000_000;
const WALKS_FOR: Duration = Duration::from_mins(1);

/// How deep it looks, where a chain of folders could otherwise hold a file open at each.
const DEEPEST: usize = 64;

/// A folder's share of all that's looked at, one part in this, below which it isn't named: so what
/// fills a folder as many small files names the folder rather than a few of them.
const WORTH_NAMING: u64 = 20;

/// What uses a computer's disk most, as its [`Keeper`] last looked, for the computer to say: empty
/// until it finds it has nothing more it can free with more than 90% of the disk used.
#[derive(Debug, Clone, Default)]
pub struct Largest(Arc<Mutex<Vec<DiskUse>>>);

impl Largest {
    /// Says what uses it, as the test of a server that can't fill a disk does.
    pub fn set(&self, largest: Vec<DiskUse>) {
        *self.0.lock().unwrap_or_else(PoisonError::into_inner) = largest;
    }

    pub(crate) fn get(&self) -> Vec<DiskUse> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

/// How far a look has got.
struct Walk {
    dev: u64,
    walked: u64,
    until: Instant,
}

impl Walk {
    fn over(&self) -> bool {
        self.walked > MOST_WALKED || Instant::now() > self.until
    }

    /// What `path` holds on the disk it looks at; a link or another disk counts nothing.
    fn size_of(&mut self, path: &Path, depth: usize) -> u64 {
        let Ok(metadata) = std::fs::symlink_metadata(path) else {
            return 0;
        };
        self.walked = self.walked.saturating_add(1);
        if metadata.dev() != self.dev || metadata.file_type().is_symlink() {
            return 0;
        }
        let mut bytes = on_disk(&metadata);
        if metadata.is_dir() && depth < DEEPEST {
            let Ok(entries) = std::fs::read_dir(path) else {
                return bytes;
            };
            for entry in entries.flatten() {
                if self.over() {
                    break;
                }
                bytes = bytes.saturating_add(self.size_of(&entry.path(), depth + 1));
            }
        }
        bytes
    }
}

/// The `kept` largest of `found`, largest first, the same path order each time.
fn keep_largest(found: &mut Vec<DiskUse>, kept: usize) {
    found.sort_by(|a, b| b.bytes.cmp(&a.bytes).then_with(|| a.path.cmp(&b.path)));
    found.truncate(kept);
}

/// What uses the disk under `home` most, the largest first: its files and each folder one level
/// down that's worth naming, and of a folder what those leave, when that's worth naming too. A
/// link or another disk counts nothing. Blocking, for a minute at most.
pub(crate) fn largest(home: &Path) -> Vec<DiskUse> {
    let Ok(here) = std::fs::symlink_metadata(home) else {
        return Vec::new();
    };
    let Ok(tops) = std::fs::read_dir(home) else {
        return Vec::new();
    };
    let mut walk = Walk {
        dev: here.dev(),
        walked: 0,
        until: Instant::now() + WALKS_FOR,
    };
    let relative = |path: &Path| {
        path.strip_prefix(home)
            .unwrap_or(path)
            .display()
            .to_string()
    };
    let mut total: u64 = 0;
    // Each folder at the top: its path, all it holds, and its largest entries.
    let mut folders: Vec<(String, u64, Vec<DiskUse>)> = Vec::new();
    let mut files: Vec<DiskUse> = Vec::new();
    for top in tops.flatten() {
        if walk.over() {
            break;
        }
        let path = top.path();
        let Ok(metadata) = std::fs::symlink_metadata(&path) else {
            continue;
        };
        if !metadata.is_dir() {
            let bytes = walk.size_of(&path, 0);
            total = total.saturating_add(bytes);
            files.push(DiskUse {
                path: relative(&path),
                bytes,
            });
            keep_largest(&mut files, LARGEST);
            continue;
        }
        if metadata.dev() != here.dev() {
            continue;
        }
        let (mut holds, mut inside) = (on_disk(&metadata), Vec::new());
        for entry in std::fs::read_dir(&path).into_iter().flatten().flatten() {
            if walk.over() {
                break;
            }
            let bytes = walk.size_of(&entry.path(), 1);
            holds = holds.saturating_add(bytes);
            inside.push(DiskUse {
                path: relative(&entry.path()),
                bytes,
            });
            keep_largest(&mut inside, LARGEST);
        }
        total = total.saturating_add(holds);
        folders.push((relative(&path), holds, inside));
    }
    named(total, folders, files)
}

/// Of what a look found, `total` in all, what's worth naming.
fn named(
    total: u64,
    folders: Vec<(String, u64, Vec<DiskUse>)>,
    files: Vec<DiskUse>,
) -> Vec<DiskUse> {
    let worth = |bytes: u64| bytes > 0 && bytes >= total / WORTH_NAMING;
    let mut found: Vec<DiskUse> = files.into_iter().filter(|file| worth(file.bytes)).collect();
    for (path, holds, inside) in folders {
        let listed: Vec<DiskUse> = inside
            .into_iter()
            .filter(|entry| worth(entry.bytes))
            .collect();
        let rest = holds.saturating_sub(listed.iter().map(|entry| entry.bytes).sum());
        found.extend(listed);
        if worth(rest) {
            found.push(DiskUse { path, bytes: rest });
        }
    }
    keep_largest(&mut found, LARGEST);
    found
}

/// What a file takes on the disk: its blocks, not its length, which a sparse file overstates.
fn on_disk(metadata: &std::fs::Metadata) -> u64 {
    metadata.blocks().saturating_mul(512)
}

/// The space on the disk `path` is on, as the system says.
pub(crate) fn space(path: &Path) -> std::io::Result<DiskSpace> {
    let stats = rustix::fs::statvfs(path)?;
    Ok(DiskSpace {
        size: stats.f_blocks.saturating_mul(stats.f_frsize),
        free: stats.f_bavail.saturating_mul(stats.f_frsize),
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
type Measure = Box<dyn Fn(&Path) -> std::io::Result<DiskSpace> + Send + Sync>;

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
    /// What it found uses the disk, which its computer says.
    largest: Largest,
}

impl Keeper {
    /// A keeper of the disk the person's home is on, in `environment` (`HOME` and the XDG
    /// folders read from it), recording in glave's folder `glave`, freeing build outputs in
    /// the worktrees under `worktrees`, running git as one of `processes`, never while
    /// `busy`, and saying in `largest` what uses the disk when nothing more can go.
    pub fn new(
        glave: PathBuf,
        worktrees: PathBuf,
        processes: Processes,
        environment: Vec<(OsString, OsString)>,
        busy: Box<dyn Fn() -> bool + Send + Sync>,
        largest: Largest,
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
            largest,
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
                Ok(true) => self.largest.set(Vec::new()),
                Ok(false) => {
                    self.look_at_what_is_left().await;
                    wait = AFTER_FULL;
                }
                Err(error) => {
                    self.note(&format!("could not keep the disk: {error}"))
                        .await;
                }
            }
        }
    }

    /// With nothing more to free, notes what uses the disk when more than 90% of it is used; a
    /// walk of the home, so it runs here, between requests, and never as one.
    async fn look_at_what_is_left(&self) {
        let short = (self.measure)(&self.home).is_ok_and(DiskSpace::short);
        let home = self.home.clone();
        let found = if short {
            tokio::task::spawn_blocking(move || largest(&home))
                .await
                .unwrap_or_default()
        } else {
            Vec::new()
        };
        self.largest.set(found);
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
            .map(DiskSpace::full)
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
        let in_the_sandbox = self.glave.join(crate::local::CACHE_FOLDER);
        let kept = in_the_sandbox.clone();
        caches.extend(
            folders(&in_the_sandbox)
                .await
                .unwrap_or_default()
                .into_iter()
                .filter(|folder| {
                    !HOMES_IN_THE_SANDBOX
                        .iter()
                        .any(|home| *folder == kept.join(home))
                }),
        );
        caches.extend(FETCHED_IN_THE_SANDBOX.iter().map(|cache| kept.join(cache)));
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
