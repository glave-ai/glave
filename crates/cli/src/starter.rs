//! glave ends when what started it ends, however that ends, when it is handed a lifeline
//! (`specs/one-server.md`, "No orphaned processes"): `GLAVE_LIFELINE` names a Unix socket that
//! what started glave listens on. glave connects to it and waits; the system closes the socket
//! when its holder ends, killed outright included, and glave then sends itself SIGTERM, so it
//! stops as it does on one, and SIGKILL if it still runs after the grace. A glave it starts in
//! the background, such as the person's server, keeps the variable and connects too. Tests and
//! the end-to-end runs hand one to every glave they start, so a run killed outright leaves none
//! behind. A socket, not a pipe: on macOS a pipe's end reaches only one of the glaves reading
//! it, while each connection ends on its own.

use std::io::{ErrorKind, Read as _};
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::thread::JoinHandle;
use std::time::Duration;

use rustix::process::{Signal, getpid, kill_process};

/// The variable naming the socket.
const LIFELINE: &str = "GLAVE_LIFELINE";

/// How long glave has to stop after SIGTERM before it is killed: the person's server gives
/// running turns up to 10 seconds to reach a wait.
const GRACE: Duration = Duration::from_secs(20);

/// Watches the lifeline glave was handed, if it was: a thread of its own waits for the
/// socket to close, then sends glave SIGTERM, and SIGKILL if it still runs after the grace.
/// None when no lifeline was handed.
/// Fails, saying why, when the socket cannot be reached: glave was asked to end with what
/// started it, which is gone or cannot be told from.
pub(crate) fn watch() -> Result<Option<JoinHandle<()>>, String> {
    let Some(path) = std::env::var_os(LIFELINE) else {
        return Ok(None);
    };
    let path = Path::new(&path);
    let failed = |error: std::io::Error| {
        format!(
            "could not reach {} ({LIFELINE}), which ends glave with what started it: {error}",
            path.display()
        )
    };
    let mut held = UnixStream::connect(path).map_err(failed)?;
    let not_watched = |error: std::io::Error| format!("could not watch {LIFELINE}: {error}");
    std::thread::Builder::new()
        .name("lifeline".into())
        .spawn(move || {
            let mut read = [0_u8; 64];
            loop {
                match held.read(&mut read) {
                    Ok(0) => break,
                    Err(error) if error.kind() != ErrorKind::Interrupted => break,
                    // Nothing is sent; anything that is, is read past.
                    Ok(_) | Err(_) => {}
                }
            }
            let _ = kill_process(getpid(), Signal::TERM);
            std::thread::sleep(GRACE);
            let _ = kill_process(getpid(), Signal::KILL);
        })
        .map(Some)
        .map_err(not_watched)
}
