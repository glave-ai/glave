//! glave as a terminal's starter: started as `glave __glave-terminal <shell> <arguments>…` in
//! a pseudo-terminal, it starts a session of its own, makes the terminal its controlling one,
//! then runs the shell in it, staying the session's leader, so Ctrl-C, jobs and hang-ups work as
//! in any terminal. Once the shell exits, or the starter is hung up or told to stop however glave
//! ended (its side of the terminal closing, the lifeline, the next start's ledger), it ends what
//! is left in the session, the shell's jobs in groups of their own among it, and ends too. Rust's
//! standard library starts no session, and glave writes no `unsafe`, so glave's own program does
//! it.

use std::ffi::OsString;
use std::time::Duration;

use tokio::signal::unix::{SignalKind, signal};

use super::session::Session;

/// The first argument that makes glave a terminal's starter (see [`terminal_helper`]).
pub(crate) const TERMINAL_HELPER_ARG: &str = "__glave-terminal";

/// How long a shell ended with its session has to be reaped: it was killed a moment before.
const REAPED_WITHIN: Duration = Duration::from_secs(5);

/// Runs this process as a terminal's starter when `args` (the whole command line) are its:
/// `glave __glave-terminal <shell> <arguments>…` takes the terminal on its input as its own,
/// in a session of its own, runs the shell until it exits or the starter is hung up or told to
/// stop, and ends what is left in the session. Returns `None` when they are not, else what went
/// wrong, empty once the shell ran. A program that passes its own path to
/// [`crate::LocalComputers::with_helper`] calls this first thing in `main`.
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
    let leader = rustix::process::getpid();
    #[cfg(target_os = "macos")]
    let session = Session::new(
        leader,
        rustix::termios::ttyname(std::io::stdin(), Vec::new())
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default(),
    );
    #[cfg(not(target_os = "macos"))]
    let session = Session::new(leader);
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build();
    let ran = match runtime {
        Ok(runtime) => {
            let ran = runtime.block_on(run(shell, arguments, session));
            // A shell that outlived its session's end is not waited for again.
            runtime.shutdown_timeout(REAPED_WITHIN);
            ran
        }
        Err(error) => Err(format!("could not start: {error}")),
    };
    Some(ran.err().unwrap_or_default())
}

/// Runs `shell` with `arguments` in `session` until it exits, or this starter is hung up (its
/// terminal's other side closed, glave gone) or told to stop (the lifeline, or the ledger at
/// glave's next start); then ends everything else in the session, the shell too if it still
/// runs.
async fn run(shell: &OsString, arguments: &[OsString], session: Session) -> Result<(), String> {
    let (mut hung_up, mut told) = signal(SignalKind::hangup())
        .and_then(|hung_up| Ok((hung_up, signal(SignalKind::terminate())?)))
        .map_err(|error| format!("could not listen for a hang-up: {error}"))?;
    let (shell, arguments) = (shell.clone(), arguments.to_vec());
    let mut shell = tokio::task::spawn_blocking(move || {
        glave_process::Program::new(&shell)
            .args(&arguments)
            .status_blocking()
    });
    let exited = tokio::select! {
        ran = &mut shell => Some(ran),
        _ = hung_up.recv() => None,
        _ = told.recv() => None,
    };
    // Asking the system what is in the session blocks, so off this thread.
    let _ = tokio::task::spawn_blocking(move || session.end_others()).await;
    let ran = match exited {
        Some(ran) => ran,
        None => match tokio::time::timeout(REAPED_WITHIN, shell).await {
            Ok(ran) => ran,
            Err(_) => return Ok(()),
        },
    };
    match ran {
        Ok(Ok(_)) => Ok(()),
        Ok(Err(error)) => Err(error.to_string()),
        Err(error) => Err(error.to_string()),
    }
}
