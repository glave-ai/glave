//! Running external commands from the workspace root, each tied to xtask by glave's lifeline
//! (`glave_process`): it, and all it starts that stays in its process group, ends when xtask
//! does, however xtask ends (`specs/one-server.md`, "No orphaned processes"). That covers the
//! long ones (the builds, the tests, the web checks, the evals) and the ones whose output is
//! read, which build too (`cargo nextest list`, the schema's example). Each runs in a process
//! group of its own with no input: a program there reading the terminal would be stopped
//! (nextest reads keys from one it is given). Ctrl-C ends xtask, and its helper then stops
//! what xtask ran.

use std::ffi::OsString;
use std::path::Path;
use std::process::{ExitCode, ExitStatus, Stdio};

use glave_process::{Processes, Program};
use tokio::io::AsyncReadExt as _;

/// Runs xtask as the lifeline's helper when it was started as one, and then returns the code
/// to exit with.
pub(crate) fn helper() -> Option<ExitCode> {
    let args: Vec<OsString> = std::env::args_os().collect();
    glave_process::lifeline_helper(&args)
}

/// Runs `program` with `args` from the workspace root and returns what it wrote to standard
/// output; standard error passes through.
///
/// Fails if the program cannot start or exits unsuccessfully.
pub(crate) fn output(program: &str, args: &[&str]) -> Result<String, String> {
    let (status, output) = tied(&crate::workspace::root()?, program, args, &[], true)?;
    if !status.success() {
        return Err(format!("`{program} {}` failed ({status})", args.join(" ")));
    }
    String::from_utf8(output).map_err(|err| {
        format!(
            "`{program} {}` wrote something not UTF-8: {err}",
            args.join(" ")
        )
    })
}

/// Runs `program` with `args` from the workspace root, streaming its output.
///
/// Fails if the program cannot start or exits unsuccessfully.
pub(crate) fn run(program: &str, args: &[&str]) -> Result<(), String> {
    run_with(program, args, &[])
}

/// [`run`], with `env` added to the program's environment.
pub(crate) fn run_with(program: &str, args: &[&str], env: &[(&str, String)]) -> Result<(), String> {
    run_in(&crate::workspace::root()?, program, args, env)
}

/// [`run_with`], in `dir`.
pub(crate) fn run_in(
    dir: &Path,
    program: &str,
    args: &[&str],
    env: &[(&str, String)],
) -> Result<(), String> {
    let (status, _) = tied(dir, program, args, env, false)?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("`{program} {}` failed ({status})", args.join(" ")))
    }
}

/// Runs `program` with `args` and `env` in `dir`, tied to xtask, to its end: its exit status,
/// and what it wrote to standard output when `read` says to keep it rather than stream it.
fn tied(
    dir: &Path,
    program: &str,
    args: &[&str],
    env: &[(&str, String)],
    read: bool,
) -> Result<(ExitStatus, Vec<u8>), String> {
    let failed = |err: &dyn std::fmt::Display| format!("could not run `{program}`: {err}");
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|err| failed(&err))?;
    runtime.block_on(async {
        let helper = glave_process::this_program().map_err(|err| failed(&err))?;
        let processes = Processes::unrecorded()
            .with_lifeline(helper)
            .await
            .map_err(|err| failed(&err))?;
        let mut command = Program::new(program);
        command
            .args(args)
            .envs(env.iter().map(|(name, value)| (*name, value)))
            .current_dir(dir)
            .stdin(Stdio::null());
        if read {
            command.stdout(Stdio::piped());
        }
        let mut child = processes
            .spawn(&mut command)
            .await
            .map_err(|err| failed(&err))?;
        let mut output = Vec::new();
        if let Some(mut stdout) = child.stdout.take() {
            stdout
                .read_to_end(&mut output)
                .await
                .map_err(|err| failed(&err))?;
        }
        let status = child.wait().await.map_err(|err| failed(&err))?;
        // What it left running in its group goes with it.
        child.stop_left().await;
        Ok((status, output))
    })
}
