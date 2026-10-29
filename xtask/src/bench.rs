//! `cargo xtask bench`: benchmarks, run alone on the machine.

use crate::lock::{self, Mode};

/// Runs `cargo bench` for the workspace under the exclusive benchmark lock.
///
/// Extra arguments are passed through to `cargo bench`. Unless they say `--no-run`, glave is
/// built first in the benchmarks' profile, for the target they name, so the budgets benchmark
/// finds it in its own build folder.
pub(crate) fn run(extra: &[String]) -> Result<(), String> {
    let _guard = lock::acquire(Mode::Exclusive)?;
    if !extra.iter().any(|arg| arg == "--no-run") {
        let mut build = vec!["build", "--profile", "bench", "-p", "glave"];
        build.extend(target(extra));
        crate::cmd::run("cargo", &build)?;
    }
    let mut args = vec!["bench", "--workspace"];
    args.extend(extra.iter().map(String::as_str));
    crate::cmd::run("cargo", &args)
}

/// The `--target` among `extra`, as cargo takes it, or nothing.
fn target(extra: &[String]) -> Vec<&str> {
    extra
        .iter()
        .enumerate()
        .find_map(|(at, arg)| {
            if arg == "--target" {
                extra
                    .get(at + 1)
                    .map(|named| vec!["--target", named.as_str()])
            } else {
                arg.strip_prefix("--target=")
                    .map(|named| vec!["--target", named])
            }
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn glave_is_built_for_the_target_the_benchmarks_are() {
        let args = |args: &[&str]| args.iter().map(|&arg| arg.to_owned()).collect::<Vec<_>>();
        let musl = ["--target", "x86_64-unknown-linux-musl"];
        assert_eq!(target(&args(&["--bench", "budgets"])), Vec::<&str>::new());
        assert_eq!(target(&args(&musl)), musl);
        assert_eq!(target(&args(&["--target=x86_64-unknown-linux-musl"])), musl);
    }
}
