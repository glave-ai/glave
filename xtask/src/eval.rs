//! `cargo xtask eval`: glave's harness on the task folders in `evals/tasks`, against real
//! models, graded from the events and files left behind (`glave-eval --help` lists the
//! options). Not part of `ci`: it needs a model endpoint and costs money, so run it after
//! changes the model will feel.

use crate::lock::{self, Mode};

/// Runs `glave-eval` with `args`, sharing the machine with tests but never with benchmarks.
pub fn run(args: &[String]) -> Result<(), String> {
    let _guard = lock::acquire(Mode::Shared)?;
    let mut command = vec!["run", "--quiet", "--package", "glave-eval", "--"];
    command.extend(args.iter().map(String::as_str));
    crate::cmd::run("cargo", &command)
}
