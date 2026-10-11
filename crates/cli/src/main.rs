//! The `glave` command: run hope, or any model behind an OpenAI-compatible API or Anthropic's
//! own, on the project in the current folder, from the terminal.

mod args;
mod asking;
mod client;
mod cloud;
mod commands;
mod computer;
mod config;
mod daemon;
mod gateway;
mod home;
mod identity;
mod input;
mod keys;
mod mcp;
mod memory;
mod model;
mod policy;
mod session;
mod setup;
mod skills;
mod system;
mod tasks;
mod terminal;
mod threads;
mod trust;
mod waiting;
mod worktrees;

fn main() -> std::process::ExitCode {
    if let Some(code) = computer::sandbox_helper() {
        return code;
    }
    session::main()
}
