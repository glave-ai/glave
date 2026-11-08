//! glave as a terminal's starter: started as `glave __glave-terminal <shell> <arguments>…` in
//! a pseudo-terminal, it starts a session of its own, makes the terminal its controlling one,
//! then runs the shell in it until the shell exits, so Ctrl-C, jobs and hang-ups work as in any
//! terminal; then it ends what the shell left in the session, and ends too. Rust's standard library starts no session, and
//! glave writes no `unsafe`, so glave's own program does it.

use std::ffi::OsString;

/// The first argument that makes glave a terminal's starter (see [`terminal_helper`]).
pub(crate) const TERMINAL_HELPER_ARG: &str = "__glave-terminal";

/// Runs this process as a terminal's starter when `args` (the whole command line) are its:
/// `glave __glave-terminal <shell> <arguments>…` takes the terminal on its input as its own,
/// in a session of its own, runs the shell to its end and ends what it left. Returns `None` when
/// they are not, else what went wrong, empty once the shell ran. A program that passes its own path
/// to [`crate::LocalComputers::with_helper`] calls this first thing in `main`.
pub fn terminal_helper(args: &[OsString]) -> Option<String> {
    if args.get(1).is_none_or(|first| first != TERMINAL_HELPER_ARG) {
        return None;
    }
    let Some((shell, arguments)) = args.get(2..).unwrap_or_default().split_first() else {
        return Some("usage: glave __glave-terminal <shell> <arguments>…".into());
    };
    if let Err(error) = rustix::process::setsid() {
        return Some(format!("could not start a session: {error}"));
    }
    if let Err(error) = rustix::process::ioctl_tiocsctty(std::io::stdin()) {
        return Some(format!("could not take the terminal as its own: {error}"));
    }
    // It stays the session's leader while the shell runs, so the shell's jobs keep the terminal
    // and are found by it once the shell exits, then ended with it.
    let ran = glave_process::Program::new(shell)
        .args(arguments)
        .status_blocking();
    let leader = rustix::process::getpid();
    #[cfg(target_os = "macos")]
    let session = super::session::Session::new(
        leader,
        rustix::termios::ttyname(std::io::stdin(), Vec::new())
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default(),
    );
    #[cfg(not(target_os = "macos"))]
    let session = super::session::Session::new(leader);
    session.end_others();
    match ran {
        Ok(_) => Some(String::new()),
        Err(error) => Some(error.to_string()),
    }
}
