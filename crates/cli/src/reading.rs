//! Each file a turn made, read apart from the server (`specs/session.md`, "Files it made"):
//! glave starts itself again as a reader, which takes the file on its input, describes and
//! previews it with glave-office, and prints what it found as JSON. A file crafted to take more
//! memory or time than reading it should ends the reader, never the server: on Linux the reader
//! can't take more than [`MOST_MEMORY`], and the server stops it past [`WITHIN`], with all it
//! started. A reader that ends otherwise than as it should has read nothing.

use std::ffi::OsString;
use std::io::{Read as _, Write as _};
use std::process::ExitCode;
use std::sync::Arc;
use std::time::Duration;

use glave_harness::MAX_FILE;
use glave_ports::{MadeRead, ReadMade};
use glave_process::{Processes, Program};

/// What glave is started with to read a file a turn made, followed by its media type.
const READER_ARG: &str = "--glave-read-made";

/// The longest a file is read for.
const WITHIN: Duration = Duration::from_secs(20);

/// The most memory a reader may take, where the operating system holds it to that (Linux).
#[cfg(target_os = "linux")]
const MOST_MEMORY: u64 = 2 * 1024 * 1024 * 1024;

/// The most a reader prints that is read: more than any preview (256 KB) written as JSON.
const MOST_ANSWER: usize = 2 * 1024 * 1024;

/// Runs glave as a reader when it was started as one, and then returns the code to exit with.
pub(crate) fn helper(args: &[OsString]) -> Option<ExitCode> {
    if args.get(1).is_none_or(|first| first != READER_ARG) {
        return None;
    }
    let Some(media_type) = args.get(2).and_then(|media_type| media_type.to_str()) else {
        return Some(ExitCode::from(2));
    };
    held_to_its_memory();
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
/// `processes`. `None` where this program can't be found.
pub(crate) fn reader(processes: &Processes) -> Option<ReadMade> {
    let program = glave_process::this_program().ok()?;
    let processes = processes.clone();
    Some(Arc::new(move |bytes, media_type| {
        let (processes, program) = (processes.clone(), program.clone());
        Box::pin(async move {
            let mut reading = Program::new(&program);
            reading.arg(READER_ARG).arg(&media_type);
            let read = processes.output_with(&mut reading, &bytes, WITHIN, MOST_ANSWER);
            let output = read.await.ok().filter(|output| output.status.success())?;
            serde_json::from_slice(&output.stdout).ok()
        })
    }))
}

/// Holds this reader to [`MOST_MEMORY`], on Linux, so a file that asks for more fails to be read
/// rather than taking the computer's memory. Elsewhere, where the limit is not kept, a reader
/// that can't have what it asks for ends.
fn held_to_its_memory() {
    #[cfg(target_os = "linux")]
    {
        let most = rustix::process::Rlimit {
            current: Some(MOST_MEMORY),
            maximum: Some(MOST_MEMORY),
        };
        let _ = rustix::process::setrlimit(rustix::process::Resource::As, most);
    }
}
