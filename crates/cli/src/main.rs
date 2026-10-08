//! The `glave` command: run hope, or any OpenAI-compatible model, on the project in the current
//! folder, from the terminal.

mod args;
mod client;
mod config;
mod home;
mod input;
mod session;
mod system;
mod terminal;
mod waiting;

fn main() -> std::process::ExitCode {
    session::main()
}
