//! `cargo xtask bench`: benchmarks, run alone on the machine.

use crate::lock::{self, Mode};

/// Runs `cargo bench` for the workspace under the exclusive benchmark lock.
///
/// Extra arguments are passed through to `cargo bench`.
pub(crate) fn run(extra: &[String]) -> Result<(), String> {
    let _guard = lock::acquire(Mode::Exclusive)?;
    let mut args = vec!["bench", "--workspace"];
    args.extend(extra.iter().map(String::as_str));
    crate::cmd::run("cargo", &args)
}
