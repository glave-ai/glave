//! harness's integration tests, one program: each file is a module of it.

#![cfg(test)]

#[path = "support/script.rs"]
mod support;

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
mod hooks;
mod instructions;
mod judge;
mod network;
mod outside_sandbox;
mod project_folders;
mod purposes;
mod retry;
mod skills;
mod sources;
mod steer;
mod step_limit;
mod subagents;
mod too_long;
mod turn;
mod untrusted;
mod workers;
mod worktrees;
