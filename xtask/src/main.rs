//! Developer automation for the Glave workspace: `cargo xtask <command>`.
//!
//! Run `cargo xtask help` for the command list.

mod bench;
mod ci;
mod cli;
mod cmd;
mod deps;
mod env_file;
mod eval;
mod file_sizes;
mod lock;
mod release_workflow;
mod schema;
mod secrets;
mod spawns;
mod typescript;
mod web;
mod workspace;

fn main() -> std::process::ExitCode {
    cli::main()
}
