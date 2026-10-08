//! The `glave` command: run hope, or any OpenAI-compatible model, on the project in the current
//! folder, from the terminal.

mod args;
mod config;
mod home;
mod session;
mod system;
mod terminal;

fn main() -> std::process::ExitCode {
    session::main()
}
