//! Running external commands from the workspace root, each tied to xtask by glave's lifeline
//! (`glave_process`): it, and all it starts that stays in its process group, ends when xtask
//! does, however xtask ends (`specs/one-server.md`, "No orphaned processes"). That covers the
//! long ones (the builds, the tests, the web checks, the evals) and the ones whose output is
//! read, which build too (`cargo nextest list`, the schema's example). Each runs in a process
//! group of its own with no input: a program there reading the terminal would be stopped
//! (nextest reads keys from one it is given). Ctrl-C ends xtask, and its helper then stops
//! what xtask ran.
//!
//! A program xtask only asks something of (git's listing, cargo's metadata) answers within
//! [`QUERY_WITHIN`] ([`query`]). The steps themselves (a build, the tests, the web checks, an
//! eval) are the work that was asked for, and take as long as the machine needs: the person at
//! the terminal stops one with Ctrl-C, and in CI the job's own time limit does.

use std::ffi::OsString;
use std::path::Path;
use std::process::{ExitCode, ExitStatus, Stdio};
use std::time::Duration;

use glave_process::{Processes, Program};
use tokio::io::AsyncReadExt as _;

/// How long a program xtask only asks something of has to answer: moments as a rule.
const QUERY_WITHIN: Duration = Duration::from_mins(2);

/// Runs xtask as the lifeline's helper when it was started as one, and then returns the code
/// to exit with.
pub(crate) fn helper() -> Option<ExitCode> {
    let args: Vec<OsString> = std::env::args_os().collect();
    glave_process::lifeline_helper(&args)
}

/// Runs `program` with `args` from the workspace root, within [`QUERY_WITHIN`], and returns
/// what it wrote to standard output; standard error passes through.
///
/// Fails if the program cannot start, exits unsuccessfully or does not answer in time.
pub(crate) fn query(program: &str, args: &[&str]) -> Result<String, String> {
    output_within(program, args, Some(QUERY_WITHIN))
}

/// [`query`], for a step whose output xtask reads (nextest's list of the tests, which builds
/// them; the schema, which builds its example): as long as the step takes.
pub(crate) fn output(program: &str, args: &[&str]) -> Result<String, String> {
    output_within(program, args, None)
}

fn output_within(program: &str, args: &[&str], limit: Option<Duration>) -> Result<String, String> {
    let (status, output) = tied(&crate::workspace::root()?, program, args, &[], true, limit)?;
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
    let (status, _) = tied(dir, program, args, env, false, None)?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("`{program} {}` failed ({status})", args.join(" ")))
    }
}

/// Runs `program` with `args` and `env` in `dir`, tied to xtask, to its end, or until `limit`
/// passes, when it is stopped: its exit status, and what it wrote to standard output when
/// `read` says to keep it rather than stream it.
fn tied(
    dir: &Path,
    program: &str,
    args: &[&str],
    env: &[(&str, String)],
    read: bool,
    limit: Option<Duration>,
) -> Result<(ExitStatus, Vec<u8>), String> {
    let failed = |err: &dyn std::fmt::Display| format!("could not run `{program}`: {err}");
    with_lifeline(|processes| async move {
        let mut command = Program::new(program);
        command
            .args(args)
            .envs(env.iter().map(|(name, value)| (*name, value)))
            .current_dir(dir)
            .no_input();
        if read {
            command.stdout(Stdio::piped());
        }
        let mut child = processes
            .spawn(&mut command)
            .await
            .map_err(|err| failed(&err))?;
        let ran = async {
            let mut output = Vec::new();
            if let Some(mut stdout) = child.stdout.take() {
                stdout.read_to_end(&mut output).await?;
            }
            Ok::<_, std::io::Error>((child.wait().await?, output))
        };
        let ran = match limit {
            None => ran.await,
            Some(limit) => {
                let Ok(ran) = tokio::time::timeout(limit, ran).await else {
                    child.stop().await;
                    return Err(format!(
                        "`{program} {}` did not finish within {}",
                        args.join(" "),
                        glave_ports::spoken(limit)
                    ));
                };
                ran
            }
        };
        let (status, output) = ran.map_err(|err| failed(&err))?;
        // What it left running in its group goes with it.
        child.stop_left().await;
        Ok((status, output))
    })
}

/// Runs what `run` does with programs it starts tied to xtask by glave's lifeline, to its end,
/// on a runtime of its own.
pub(crate) fn with_lifeline<T, F>(run: impl FnOnce(Processes) -> F) -> Result<T, String>
where
    F: Future<Output = Result<T, String>>,
{
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|err| format!("could not start a runtime: {err}"))?;
    runtime.block_on(async {
        let helper = glave_process::this_program()
            .map_err(|err| format!("could not find xtask's own program: {err}"))?;
        let processes = Processes::unrecorded()
            .with_lifeline(helper)
            .await
            .map_err(|err| format!("could not tie what xtask runs to it: {err}"))?;
        run(processes).await
    })
}
