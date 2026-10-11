//! The `glave` command: run hope, or any model behind an OpenAI-compatible API or Anthropic's
//! own, on the project in the current folder, from the terminal.

mod args;
mod asking;
mod client;
mod commands;
mod computer;
mod config;
mod gateway;
mod home;
mod identity;
mod input;
mod keys;
mod local;
mod mcp;
mod model;
mod policy;
mod session;
mod setup;
mod skills;
mod system;
mod terminal;
mod threads;
mod trust;
mod waiting;
mod worktrees;

fn main() -> std::process::ExitCode {
    session::main()
}
