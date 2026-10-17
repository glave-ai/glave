//! The session API's tests, one binary: the conformance suite every transport passes, then
//! what each part of the API does beyond it, against a scripted model. All of it is test code,
//! where clippy lets a test unwrap and index.

#![cfg(test)]

mod support;

mod background_commands;
mod commands;
mod conformance;
mod credentials;
mod goals;
mod http;
mod listing;
mod local;
mod memory;
mod models;
mod modes;
mod outside;
mod placement;
mod projects;
mod providers;
mod questions;
mod remote;
mod schedules;
mod server;
mod site;
mod spaces;
mod standing;
mod steer;
mod subagents;
mod tasks;
mod titles;
mod tracing;
mod trust;
mod worktrees;
