//! glave started again as a helper of its own: the lifeline's, which stops what glave started
//! once glave ends however it ends (`specs/one-server.md`, "No orphaned processes"), the
//! Linux sandbox's, which confines itself and then becomes a command (`specs/sandbox.md`), the
//! outbound proxy's on Linux, the forwarder that holds a thread's network namespace, carrying
//! its connections to the proxy, and the one that joins it and then becomes a command
//! (`specs/sandbox.md`, "The outbound proxy"), and
//! a terminal's, which takes its terminal in a session of its own and runs the person's shell
//! in it, staying the session's leader (`specs/computers.md`, "Terminals").
//! Each is run first thing in `main`, before glave starts a thread. And this glave, first on
//! every command's `PATH`, so the `glave doc` a skill teaches is the glave that taught it.

use std::ffi::OsString;
use std::path::Path;
use std::process::ExitCode;

use anyhow::Context as _;
use glave_computer::LocalComputers;

/// The folder in glave's own that is first on every command's `PATH`, holding `glave`.
const ON_PATH: &str = "bin";

/// Runs glave as one of its helpers when it was started as one, and then returns the code to
/// exit with: the lifeline's once what glave left running is stopped, and the others' only
/// when holding or joining a thread's network, confining itself, taking the terminal or
/// starting the program failed.
pub(crate) fn run() -> Option<ExitCode> {
    let args: Vec<OsString> = std::env::args_os().collect();
    if let Some(code) = glave_process::lifeline_helper(&args) {
        return Some(code);
    }
    if let Some(failure) = glave_computer::forward_helper(&args) {
        eprintln!("glave sandbox: {failure}");
        return Some(ExitCode::from(126));
    }
    if let Some(failure) = glave_computer::terminal_helper(&args) {
        if failure.is_empty() {
            return Some(ExitCode::SUCCESS);
        }
        eprintln!("glave terminal: {failure}");
        return Some(ExitCode::from(126));
    }
    let failure = glave_computer::sandbox_helper(&args)?;
    eprintln!("glave sandbox: {failure}");
    Some(ExitCode::from(126))
}

/// `computers` with this program as glave's own, which Linux needs to sandbox commands where
/// bubblewrap cannot start, and every computer to open a terminal.
pub(crate) fn with_helper(computers: LocalComputers) -> LocalComputers {
    match glave_process::this_program() {
        Ok(program) => computers.with_helper(program),
        Err(_) => computers,
    }
}

/// `computers` with this glave first on every command's `PATH`: `glave` in glave's folder
/// `home`, linked to this program's path (never `/proc/self/exe`, which a command would read
/// as its own), made anew at each start.
pub(crate) async fn with_glave_on_path(
    computers: LocalComputers,
    home: &Path,
) -> anyhow::Result<LocalComputers> {
    let program =
        std::env::current_exe().context("could not find this glave to put it on commands' PATH")?;
    let folder = home.join(ON_PATH);
    glave_computer::glave_on_path(&folder, &program)
        .await
        .with_context(|| format!("could not put glave in {}", folder.display()))?;
    Ok(computers.with_glave_on_path(folder))
}
