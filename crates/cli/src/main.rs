//! The `glave` command: run hope, or any model behind an OpenAI-compatible API or Anthropic's
//! own, on the project in the current folder, from the terminal.

#![expect(
    clippy::print_stdout,
    clippy::print_stderr,
    reason = "glave's binary: the terminal is where it answers"
)]

mod accounts;
mod app;
mod args;
mod asking;
mod catalogs;
mod client;
mod cloud;
mod commands;
mod computer;
mod config;
mod connections;
mod credentials;

mod data;
mod gateway;
mod home;
mod identity;
mod input;
mod keys;
mod mcp;
mod memory;
mod model;
mod project_folders;
mod projects;
mod providers;
mod reach;
mod remote;
mod reread;
mod sandboxing;
mod server;
mod serving;
mod session;
mod setup;
mod skills;
mod socket;
mod system;
mod tasks;
mod telemetry;
mod terminal;
mod threads;
mod trust;
mod turns;
mod waiting;
mod wiring;
mod worktrees;

fn main() -> std::process::ExitCode {
    if let Some(code) = computer::sandbox_helper() {
        return code;
    }
    session::main()
}
