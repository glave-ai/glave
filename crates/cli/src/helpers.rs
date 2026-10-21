//! glave started again as a helper of its own: the lifeline's, which stops what glave started
//! once glave ends however it ends (`specs/one-server.md`, "No orphaned processes"), and the
//! Linux sandbox's, which confines itself and then becomes a command (`specs/sandbox.md`).
//! Each is run first thing in `main`, before glave starts a thread.

use std::ffi::OsString;
use std::path::PathBuf;
use std::process::ExitCode;

use glave_computer::LocalComputers;

/// Runs glave as one of its helpers when it was started as one, and then returns the code to
/// exit with: the lifeline's once what glave left running is stopped, the sandbox's only when
/// confining itself or starting the command failed.
pub(crate) fn run() -> Option<ExitCode> {
    let args: Vec<OsString> = std::env::args_os().collect();
    if let Some(code) = glave_process::lifeline_helper(&args) {
        return Some(code);
    }
    let failure = glave_computer::sandbox_helper(&args)?;
    eprintln!("glave sandbox: {failure}");
    Some(ExitCode::from(126))
}

/// This program, to start again as a helper. On Linux the file the system runs glave from,
/// which still starts after an upgrade replaced or removed the one at glave's path.
pub(crate) fn this_program() -> std::io::Result<PathBuf> {
    if cfg!(target_os = "linux") {
        Ok(PathBuf::from("/proc/self/exe"))
    } else {
        std::env::current_exe()
    }
}

/// `computers` with this program as the sandbox's helper, which Linux needs where
/// bubblewrap cannot start.
pub(crate) fn with_sandbox_helper(computers: LocalComputers) -> LocalComputers {
    match this_program() {
        Ok(program) => computers.with_sandbox_helper(program),
        Err(_) => computers,
    }
}
