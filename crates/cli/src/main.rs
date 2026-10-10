//! The `glave` command: run hope, or any model behind an OpenAI-compatible API or Anthropic's
//! own, on the project in the current folder, from the terminal.

mod args;
mod asking;
mod client;
mod computer;
mod config;
mod gateway;
mod home;
mod identity;
mod input;
mod keys;
mod mcp;
mod model;
mod policy;
mod session;
mod setup;
mod system;
mod terminal;
mod waiting;

fn main() -> std::process::ExitCode {
    session::main()
}
