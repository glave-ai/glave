//! Each file a turn made, read apart from the server (`specs/session.md`, "Files it made"):
//! glave starts itself again as a reader, with an empty environment, which takes the file on its
//! input, describes and previews it with glave-office, and prints what it found as JSON. A file
//! crafted to take more memory or time than reading it should ends the reader, never the
//! server. Before it reads anything the reader holds itself to [`MOST_MEMORY`] of allocations,
//! past which it ends (on Linux `RLIMIT_DATA`, which counts what it maps too; on macOS, which
//! enforces no such limit, glave's own allocator, `allocator.rs`), leaves no core dump of what it
//! decoded, and on Linux is the first the kernel ends when memory runs out, so the readers' own
//! never take the server. The server stops it past [`WITHIN`], with all it started. A reader
//! that ends otherwise than as it should has read nothing. Readers run inside look-overs, one
//! file at a time each, so as many run at once as look-overs do: at most 4 on a server and 2 for
//! one organization (glave-harness's `made.rs`), their memory in all at most 4 times
//! [`MOST_MEMORY`].

use std::ffi::OsString;
use std::io::{Read as _, Write as _};
use std::process::ExitCode;
use std::sync::Arc;
use std::time::Duration;

use glave_harness::MAX_FILE;
use glave_ports::{MadeRead, ReadMade};
use glave_process::{Processes, Program};
use rustix::process::{Resource, Rlimit, setrlimit};

/// What glave is started with to read a file a turn made, followed by its media type.
const READER_ARG: &str = "--glave-read-made";

/// What starts each line a reader writes to its standard error saying why it couldn't hold
/// itself to what reading may take, never anything of the file.
const SAYS: &str = "glave's reader: ";

/// What a reader exits with when it couldn't hold itself to what reading may take: only then are
/// the lines it wrote that start with [`SAYS`] its own, and logged. A reader that ends any other
/// way, a panic in what read the file among them, may have written anything, which never is.
const NOT_HELD: u8 = 75;

/// The longest a file is read for.
const WITHIN: Duration = Duration::from_secs(20);

/// The most memory a reader allocates: what it can't have ends it. Short of 2 GB, so the whole
/// reader, its program and stack with it, stays under 2 GB on every computer.
const MOST_MEMORY: usize = 1536 * 1024 * 1024;

/// The most a reader prints that is read, on each of its outputs: more than any preview
/// (256 KB) written as JSON.
const MOST_ANSWER: usize = 2 * 1024 * 1024;

/// Runs glave as a reader when it was started as one, and then returns the code to exit with.
pub(crate) fn helper(args: &[OsString]) -> Option<ExitCode> {
    if args.get(1).is_none_or(|first| first != READER_ARG) {
        return None;
    }
    let Some(media_type) = args.get(2).and_then(|media_type| media_type.to_str()) else {
        return Some(ExitCode::from(2));
    };
    if let Err(why) = held_to_reading() {
        let _ = writeln!(std::io::stderr(), "{SAYS}{why}");
        return Some(ExitCode::from(NOT_HELD));
    }
    // What reads the file says nothing of it, however it fails: a panic's message may quote it.
    std::panic::set_hook(Box::new(|_| {}));
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

/// Holds this process, a reader, to what reading a file may take, first thing: no core dump of
/// what it decodes, which would leave the file unsealed on the drive; at most [`MOST_MEMORY`]
/// of allocations; and on Linux, the first process the kernel ends when memory runs out, and
/// none dumped by anything else. That last comes last: once a process is not dumpable, the
/// kernel gives its `/proc/self` files to root, which a server not run as root can't write.
/// Says why, in a sentence of its own, when it can't.
fn held_to_reading() -> Result<(), String> {
    fn said(what: &'static str) -> impl Fn(rustix::io::Errno) -> String {
        move |error| format!("{what}: {error}")
    }
    let nothing = Rlimit {
        current: Some(0),
        maximum: Some(0),
    };
    setrlimit(Resource::Core, nothing).map_err(said("its core dumps couldn't be turned off"))?;
    #[cfg(target_os = "linux")]
    {
        use rustix::process::{DumpableBehavior, set_dumpable_behavior};
        std::fs::write("/proc/self/oom_score_adj", "1000").map_err(|error| {
            format!("it couldn't be put first to go when memory runs out: {error}")
        })?;
        let most = u64::try_from(MOST_MEMORY).unwrap_or(u64::MAX);
        let data = Rlimit {
            current: Some(most),
            maximum: Some(most),
        };
        setrlimit(Resource::Data, data).map_err(said("its memory couldn't be limited"))?;
        set_dumpable_behavior(DumpableBehavior::NotDumpable)
            .map_err(said("it couldn't be kept from being dumped"))?;
    }
    #[cfg(target_os = "macos")]
    crate::allocator::GLOBAL
        .set_limit(MOST_MEMORY)
        .map_err(|()| "its memory couldn't be limited".to_owned())?;
    Ok(())
}

/// What reads each file a turn made: this program as a reader, in a process of its own among
/// `processes`, when a share of the readers is free. `None` where this program can't be found.
pub(crate) fn reader(processes: &Processes) -> Option<ReadMade> {
    let program = glave_process::this_program().ok()?;
    let processes = processes.clone();
    Some(Arc::new(move |bytes, media_type| {
        let (processes, program) = (processes.clone(), program.clone());
        Box::pin(async move {
            let mut reading = Program::new(&program);
            reading.arg(READER_ARG).arg(&media_type).env_clear();
            let read = processes.output_with(&mut reading, &bytes, WITHIN, MOST_ANSWER);
            let output = read.await.ok()?;
            for why in said_why(output.status.code(), &output.stderr) {
                tracing::warn!(why, "a reader of a file a turn made read nothing");
            }
            if !output.status.success() {
                return None;
            }
            serde_json::from_slice(&output.stdout).ok()
        })
    }))
}

/// Why a reader that ended with `code`, having written `stderr`, read nothing, in its own words:
/// only when it ended as one that couldn't hold itself to its limits ([`NOT_HELD`]), so nothing
/// else it wrote, which might quote the file, is ever logged.
fn said_why(code: Option<i32>, stderr: &[u8]) -> Vec<String> {
    if code != Some(i32::from(NOT_HELD)) {
        return Vec::new();
    }
    String::from_utf8_lossy(stderr)
        .lines()
        .filter_map(|line| line.strip_prefix(SAYS))
        .map(str::to_owned)
        .collect()
}

#[cfg(test)]
mod tests {
    use rustix::process::getrlimit;

    use super::*;

    #[test]
    fn only_a_reader_that_could_not_hold_itself_is_heard_and_never_what_it_read() {
        let said = format!("{SAYS}its memory couldn't be limited: EPERM\n");
        assert_eq!(
            said_why(Some(i32::from(NOT_HELD)), said.as_bytes()),
            ["its memory couldn't be limited: EPERM"]
        );
        // A reader that ended any other way, a panic among them, may have written the file.
        let forged = format!("thread 'main' panicked\n{SAYS}Q3 revenue: 4.2M\n");
        assert_eq!(said_why(Some(101), forged.as_bytes()), Vec::<String>::new());
        assert_eq!(said_why(None, forged.as_bytes()), Vec::<String>::new());
    }

    #[test]
    fn a_reader_leaves_no_core_dump_and_is_held_to_its_memory() {
        held_to_reading().unwrap();
        let core = getrlimit(Resource::Core);
        assert_eq!((core.current, core.maximum), (Some(0), Some(0)));
        #[cfg(target_os = "linux")]
        {
            use rustix::process::{DumpableBehavior, dumpable_behavior};
            assert_eq!(dumpable_behavior().unwrap(), DumpableBehavior::NotDumpable);
            let adjusted = std::fs::read_to_string("/proc/self/oom_score_adj").unwrap();
            assert_eq!(adjusted.trim(), "1000", "the first the kernel ends");
            let data = getrlimit(Resource::Data);
            assert_eq!(data.current, u64::try_from(MOST_MEMORY).ok());
        }
        #[cfg(target_os = "macos")]
        assert_eq!(crate::allocator::GLOBAL.limit(), MOST_MEMORY);
    }
}
