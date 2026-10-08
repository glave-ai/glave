//! Running external commands from the workspace root.

use std::process::Command;

/// Runs `program` with `args` from the workspace root, streaming its output.
///
/// Fails if the program cannot start or exits unsuccessfully.
pub fn run(program: &str, args: &[&str]) -> Result<(), String> {
    let status = Command::new(program)
        .args(args)
        .current_dir(crate::workspace::root()?)
        .status()
        .map_err(|err| format!("could not start `{program}`: {err}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("`{program} {}` failed ({status})", args.join(" ")))
    }
}
