//! Each file a turn made, read apart from the server (`specs/session.md`, "Files it made"):
//! glave starts itself again as a reader, with an empty environment, which takes the file on its
//! input, describes and previews it with glave-office, and prints what it found as JSON. A file
//! crafted to take more memory or time than reading it should ends the reader, never the
//! server: on every computer the reader's allocations are held to [`MOST_MEMORY`], past which
//! it ends, and the server stops it past [`WITHIN`], with all it started. A reader that ends
//! otherwise than as it should has read nothing. At most [`MOST_READERS`] run at once on a
//! server, and at most [`MOST_PER_ORG`] for one organization, so no one's files hold up
//! everyone's.

use std::collections::HashMap;
use std::ffi::OsString;
use std::io::{Read as _, Write as _};
use std::process::ExitCode;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use glave_harness::MAX_FILE;
use glave_ports::{MadeRead, OrgId, ReadMade};
use glave_process::{Processes, Program};
use tokio::sync::Notify;

/// What glave is started with to read a file a turn made, followed by its media type.
const READER_ARG: &str = "--glave-read-made";

/// The longest a file is read for.
const WITHIN: Duration = Duration::from_secs(20);

/// The most memory a reader allocates: what it can't have ends it. Short of 2 GB, so the whole
/// reader, its program and stack with it, stays under 2 GB on every computer.
const MOST_MEMORY: usize = 1536 * 1024 * 1024;

/// The most a reader prints that is read, on each of its outputs: more than any preview
/// (256 KB) written as JSON.
const MOST_ANSWER: usize = 2 * 1024 * 1024;

/// The most readers running at once on a server.
const MOST_READERS: usize = 4;

/// The most readers running at once for one organization.
const MOST_PER_ORG: usize = 2;

/// Runs glave as a reader when it was started as one, and then returns the code to exit with.
pub(crate) fn helper(args: &[OsString]) -> Option<ExitCode> {
    if args.get(1).is_none_or(|first| first != READER_ARG) {
        return None;
    }
    let Some(media_type) = args.get(2).and_then(|media_type| media_type.to_str()) else {
        return Some(ExitCode::from(2));
    };
    if crate::allocator::GLOBAL.set_limit(MOST_MEMORY).is_err() {
        return Some(ExitCode::FAILURE);
    }
    let mut bytes = Vec::new();
    let mut input = std::io::stdin().take(MAX_FILE.saturating_add(1));
    if input.read_to_end(&mut bytes).is_err() {
        return Some(ExitCode::FAILURE);
    }
    let described = glave_office::describe(&bytes, media_type);
    let preview = glave_office::preview(&bytes, media_type).ok();
    let read = MadeRead {
        title: described.title,
        parts: described.parts,
        cut: preview.as_ref().is_some_and(|preview| preview.cut),
        markdown: preview.map(|preview| preview.markdown),
    };
    let Ok(printed) = serde_json::to_vec(&read) else {
        return Some(ExitCode::FAILURE);
    };
    match std::io::stdout().write_all(&printed) {
        Ok(()) => Some(ExitCode::SUCCESS),
        Err(_) => Some(ExitCode::FAILURE),
    }
}

/// What reads each file a turn made: this program as a reader, in a process of its own among
/// `processes`, when a share of the readers is free. `None` where this program can't be found.
pub(crate) fn reader(processes: &Processes) -> Option<ReadMade> {
    let program = glave_process::this_program().ok()?;
    let processes = processes.clone();
    let readers = Arc::new(Readers::default());
    Some(Arc::new(move |org, bytes, media_type| {
        let (processes, program, readers) =
            (processes.clone(), program.clone(), Arc::clone(&readers));
        Box::pin(async move {
            let _share = readers.share(org).await;
            let mut reading = Program::new(&program);
            reading.arg(READER_ARG).arg(&media_type).env_clear();
            let read = processes.output_with(&mut reading, &bytes, WITHIN, MOST_ANSWER);
            let output = read.await.ok().filter(|output| output.status.success())?;
            serde_json::from_slice(&output.stdout).ok()
        })
    }))
}

/// The readers running now, in all and by organization, and what tells those waiting that one
/// ended.
#[derive(Default)]
struct Readers {
    running: Mutex<Running>,
    ended: Notify,
}

#[derive(Default)]
struct Running {
    all: usize,
    by_org: HashMap<OrgId, usize>,
}

/// A reader's place among those running, given up as it drops.
struct Share<'a> {
    readers: &'a Readers,
    org: OrgId,
}

impl Readers {
    /// A place for a reader of `org`'s, once fewer than [`MOST_READERS`] run in all and fewer
    /// than [`MOST_PER_ORG`] of `org`'s. What waits for it is held to the look-over's deadline,
    /// which the harness wraps every reader in.
    async fn share(&self, org: OrgId) -> Share<'_> {
        loop {
            let ended = self.ended.notified();
            {
                let mut running = self.running.lock().unwrap_or_else(PoisonError::into_inner);
                let theirs = running.by_org.get(&org).copied().unwrap_or(0);
                if running.all < MOST_READERS && theirs < MOST_PER_ORG {
                    running.all = running.all.saturating_add(1);
                    running.by_org.insert(org, theirs.saturating_add(1));
                    return Share { readers: self, org };
                }
            }
            ended.await;
        }
    }
}

impl Drop for Share<'_> {
    fn drop(&mut self) {
        let mut running = self
            .readers
            .running
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        running.all = running.all.saturating_sub(1);
        let left = running.by_org.get(&self.org).copied().unwrap_or(0);
        if left <= 1 {
            running.by_org.remove(&self.org);
        } else {
            running.by_org.insert(self.org, left.saturating_sub(1));
        }
        drop(running);
        self.readers.ended.notify_waiters();
    }
}
