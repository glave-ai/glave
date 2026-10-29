//! The ledger: one small file per running process group glave started, in
//! `GLAVE_HOME/run/processes`, named by the group's leader. Each says what the process is (its
//! id and start), its group, its owner, and which glave started it (that glave's id and
//! start). It is written whole and put in place in one step (`glave_ports::file`), so a reader
//! never sees half of one, and removed when the group is gone (kept, for the next start to
//! settle, when a stop left something of it behind). Every glave on the computer shares
//! the folder; each entry is only ever written by the glave that started it.

use std::collections::HashSet;
use std::os::unix::fs::DirBuilderExt as _;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock, PoisonError};
use std::time::{Duration, SystemTime};

use glave_ports::file::{self, WRITING};
use glave_ports::{Clock as _, SystemClock};
use serde::{Deserialize, Serialize};

use crate::error::ProcessError;
use crate::owner::Owner;
use crate::start::{self, Start};

/// The most entries the ledger holds, for every glave on the computer together.
pub(crate) const LIMIT: usize = 4096;
/// The version of an entry's format.
const VERSION: u32 = 1;
/// How old a file being written must be before a reaper takes it for one a crash left.
const ABANDONED: Duration = Duration::from_secs(60);

/// A process, as the operating system knows it: its id and when it started.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Process {
    pub(crate) pid: i32,
    pub(crate) start: Start,
}

impl Process {
    /// Whether this very process is still there, its id not given to another since.
    pub(crate) fn alive_in(&self, starts: &std::collections::HashMap<i32, Start>) -> bool {
        starts.get(&self.pid) == Some(&self.start)
    }
}

/// One entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct Record {
    pub(crate) version: u32,
    /// The group's leader.
    pub(crate) process: Process,
    pub(crate) group: i32,
    pub(crate) owner: Owner,
    /// The glave that started it.
    pub(crate) glave: Process,
}

/// The ledger in one folder.
#[derive(Debug)]
pub(crate) struct Ledger {
    folder: PathBuf,
    limit: usize,
    /// This glave, read the first time it records anything.
    glave: OnceLock<Option<Process>>,
    /// One record at a time from this glave, so its count of entries holds.
    writing: Mutex<()>,
}

impl Ledger {
    pub(crate) fn new(folder: PathBuf, limit: usize) -> Self {
        Self {
            folder,
            limit,
            glave: OnceLock::new(),
            writing: Mutex::new(()),
        }
    }

    /// This glave, as its entries name it; none where no start can be read.
    pub(crate) fn glave(&self) -> Option<&Process> {
        self.glave
            .get_or_init(|| {
                let pid = i32::try_from(std::process::id()).ok()?;
                let start = start::of(&[pid]).remove(&pid)?;
                Some(Process { pid, start })
            })
            .as_ref()
    }

    fn path(&self, pid: i32) -> PathBuf {
        self.folder.join(format!("{pid}.json"))
    }

    fn failed(&self, source: std::io::Error) -> ProcessError {
        ProcessError::Ledger {
            path: self.folder.clone(),
            source,
        }
    }

    /// Records the group `pid` leads for `owner`. None when its start cannot be read, which
    /// means it is already gone (or this system tells no starts): there is nothing to record.
    /// Refuses when the ledger is full of processes still running. Blocking.
    pub(crate) fn record(&self, pid: i32, owner: &Owner) -> Result<Option<Entry>, ProcessError> {
        let Some(glave) = self.glave().cloned() else {
            return Ok(None);
        };
        let Some(start) = start::of(&[pid]).remove(&pid) else {
            return Ok(None);
        };
        let _writing = self.writing.lock().unwrap_or_else(PoisonError::into_inner);
        // Private to the person, as the rest of glave's run folder is.
        std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(&self.folder)
            .map_err(|error| self.failed(error))?;
        if self.entries()?.len() >= self.limit {
            self.forget_gone()?;
            if self.entries()?.len() >= self.limit {
                return Err(ProcessError::Full(self.limit));
            }
        }
        let record = Record {
            version: VERSION,
            process: Process { pid, start },
            group: pid,
            owner: owner.clone(),
            glave,
        };
        let text = serde_json::to_vec(&record).map_err(|error| self.failed(error.into()))?;
        let path = self.path(pid);
        file::replace(&path, &text).map_err(|error| self.failed(error))?;
        Ok(Some(Entry { path: Some(path) }))
    }

    /// The paths of the entries in place, not those being written.
    fn entries(&self) -> Result<Vec<PathBuf>, ProcessError> {
        let listed = match std::fs::read_dir(&self.folder) {
            Ok(listed) => listed,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => return Err(self.failed(error)),
        };
        Ok(listed
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| {
                path.extension()
                    .is_some_and(|extension| extension == "json")
            })
            .collect())
    }

    /// Every entry, read. One that cannot be read, damaged or from a format this glave does
    /// not know, is skipped and left for the glave that wrote it.
    pub(crate) fn records(&self) -> Result<Vec<(PathBuf, Record)>, ProcessError> {
        Ok(self
            .entries()?
            .into_iter()
            .filter_map(|path| {
                let text = std::fs::read(&path).ok()?;
                let record: Record = serde_json::from_slice(&text).ok()?;
                (record.version == VERSION).then_some((path, record))
            })
            .collect())
    }

    /// Forgets the entries whose process has ended, or whose id another process has now.
    fn forget_gone(&self) -> Result<(), ProcessError> {
        let records = self.records()?;
        let pids: Vec<i32> = records
            .iter()
            .map(|(_, record)| record.process.pid)
            .collect();
        let starts = start::of(&pids);
        for (path, record) in records {
            if !record.process.alive_in(&starts) {
                let _ = std::fs::remove_file(path);
            }
        }
        Ok(())
    }

    /// Removes what a crash left half written, once it is old enough not to be a write under
    /// way.
    pub(crate) fn clear_abandoned(&self) {
        let Ok(listed) = std::fs::read_dir(&self.folder) else {
            return;
        };
        let now = SystemTime::from(SystemClock.now());
        for path in listed.filter_map(Result::ok).map(|entry| entry.path()) {
            let old = std::fs::metadata(&path)
                .and_then(|metadata| metadata.modified())
                .is_ok_and(|at| now.duration_since(at).is_ok_and(|age| age >= ABANDONED));
            if path
                .extension()
                .is_some_and(|extension| extension == WRITING)
                && old
            {
                let _ = std::fs::remove_file(path);
            }
        }
    }

    /// The ids of every process the entries name, leaders and glaves.
    pub(crate) fn pids(records: &[(PathBuf, Record)]) -> Vec<i32> {
        let all: HashSet<i32> = records
            .iter()
            .flat_map(|(_, record)| [record.process.pid, record.glave.pid])
            .collect();
        all.into_iter().collect()
    }
}

/// A process's entry, removed when this drops, unless what it names was left behind.
#[derive(Debug)]
pub(crate) struct Entry {
    /// None once left behind.
    path: Option<PathBuf>,
}

impl Entry {
    /// Keeps the entry once this drops: its group outlived SIGKILL's wait (a process stuck in
    /// the kernel, on a hung network mount), so the next glave to start settles it.
    pub(crate) fn left_behind(mut self) {
        self.path = None;
    }
}

impl Drop for Entry {
    fn drop(&mut self) {
        if let Some(path) = &self.path {
            let _ = std::fs::remove_file(path);
        }
    }
}
