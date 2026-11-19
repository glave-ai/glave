//! The `glave` command: run hope, or any model behind an OpenAI-compatible API or Anthropic's
//! own, on the project in the current folder, from the terminal.

#![expect(
    clippy::print_stdout,
    clippy::print_stderr,
    reason = "glave's binary: the terminal is where it answers"
)]

mod accounts;
#[cfg(any(target_env = "musl", target_os = "macos"))]
mod allocator;
mod app;
mod args;
mod asking;
mod awake;
mod capabilities;
mod catalogs;
mod client;
mod cloud;
mod commands;
mod computer;
mod config;
mod connected;
mod connections;
mod connectors;
mod credentials;
mod credits;

mod data;
mod doc;
mod gateway;
mod helpers;
mod home;
mod identity;
mod input;
mod keys;
mod mcp;
mod memory;
mod model;
mod people;
mod project_folders;
mod projects;
mod providers;
mod reach;
mod reading;
mod remote;
mod reread;
mod said;
mod sandboxing;
mod search;
mod server;
mod serving;
mod session;
mod setup;
mod signup;
mod skills;
mod socket;
mod starter;
mod system;
mod tasks;
mod telemetry;
mod terminal;
mod threads;
mod trust;
mod turns;
mod wiring;
mod worktrees;

fn main() -> std::process::ExitCode {
    let kept = capabilities::keep_to_itself();
    if let Some(code) = helpers::run() {
        return code;
    }
    session::main(kept.as_deref())
}
