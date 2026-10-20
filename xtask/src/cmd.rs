//! Running external commands from the workspace root.

use std::process::Command;

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
    let status = Command::new(program)
        .args(args)
        .envs(env.iter().map(|(name, value)| (*name, value)))
        .current_dir(crate::workspace::root()?)
        .status()
        .map_err(|err| format!("could not start `{program}`: {err}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("`{program} {}` failed ({status})", args.join(" ")))
    }
}
