//! Running external commands from the workspace root. One that runs to its end on its own (a
//! build, the tests, the web checks, the evals) is tied to xtask by glave's lifeline
//! (`glave_process`): it, and all it starts that stays in its process group, ends when xtask
//! does, however xtask ends (`specs/one-server.md`, "No orphaned processes"). One whose output
//! is read is short, and ends with the call.

use std::ffi::OsString;
use std::path::Path;
use std::process::{Command, ExitCode};

use glave_process::{Processes, Program};

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
    let output = Command::new(program)
        .args(args)
        .current_dir(crate::workspace::root()?)
        .stderr(std::process::Stdio::inherit())
        .output()
        .map_err(|err| format!("could not start `{program}`: {err}"))?;
    if !output.status.success() {
        return Err(format!(
            "`{program} {}` failed ({})",
            args.join(" "),
            output.status
        ));
    }
    String::from_utf8(output.stdout).map_err(|err| {
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
    let failed = |err: &dyn std::fmt::Display| format!("could not run `{program}`: {err}");
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|err| failed(&err))?;
    let status = runtime.block_on(async {
        let helper = glave_process::this_program().map_err(|err| failed(&err))?;
        let processes = Processes::unrecorded()
            .with_lifeline(helper)
            .await
            .map_err(|err| failed(&err))?;
        let mut command = Program::new(program);
        // Nothing it runs reads the terminal: in a process group of its own, a program that
        // did would be stopped (nextest reads keys from one it is given).
        command
            .args(args)
            .envs(env.iter().map(|(name, value)| (*name, value)))
            .current_dir(dir)
            .stdin(std::process::Stdio::null());
        let mut child = processes
            .spawn(&mut command)
            .await
            .map_err(|err| failed(&err))?;
        let status = child.wait().await.map_err(|err| failed(&err))?;
        // What it left running in its group goes with it.
        child.stop_left().await;
        Ok::<_, String>(status)
    })?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("`{program} {}` failed ({status})", args.join(" ")))
    }
}
