//! Developer automation for the Glave workspace: `cargo xtask <command>`.
//!
//! Run `cargo xtask help` for the command list.

mod bench;
mod ci;
mod cli;
mod cmd;
mod eval;
mod file_sizes;
mod lock;
mod schema;
mod typescript;
mod workspace;

fn main() -> std::process::ExitCode {
    cli::main()
}
