//! harness's integration tests, one program: each file is a module of it.

#![cfg(test)]

#[path = "support/script.rs"]
mod support;

#[path = "support/worker.rs"]
mod worker;

#[path = "support/wire.rs"]
mod wire;

mod actors;
mod approvals;
mod background;
mod blobs;
mod cancel;
mod checks;
mod compaction;
mod computers;
mod faults;
mod files;
mod goals;
mod homes;
mod hooks;
mod install;
mod instructions;
mod judge;
mod log_failures;
mod modes;
mod network;
mod outside_sandbox;
mod plan;
mod project_folders;
mod purposes;
mod retry;
mod service;
mod skills;
mod sources;
mod steer;
mod step_limit;
mod subagents;
mod too_long;
mod turn;
mod untrusted;
mod untrusted_after;
mod workers;
mod worktrees;
