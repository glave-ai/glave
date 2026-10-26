//! Developer automation for the Glave workspace: `cargo xtask <command>`.
//!
//! Run `cargo xtask help` for the command list.

#![expect(
    clippy::print_stdout,
    clippy::print_stderr,
    reason = "a developer's command: it reports in the terminal"
)]

mod bench;
mod changed;
mod ci;
mod cli;
mod cmd;
mod deps;
mod env_file;
mod eval;
mod file_sizes;
mod leftovers;
mod lock;
mod nextest;
mod package_age;
mod parallel;
mod release_workflow;
mod schema;
mod secrets;
mod sources;
mod spawns;
mod typescript;
mod waits;
mod web;
mod workspace;

fn main() -> std::process::ExitCode {
    cli::main()
}
