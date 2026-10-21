//! The session API's tests, one binary: the conformance suite every transport passes, then
//! what each part of the API does beyond it, against a scripted model. All of it is test code,
//! where clippy lets a test unwrap and index.

#![cfg(test)]

mod support;

mod awake;
mod background_commands;
mod browsers;
mod choices;
mod commands;
mod conformance;
mod credentials;
mod durable;
mod files;
mod goals;
mod gone;
mod http;
mod isolation;
mod leases;
mod listing;
mod local;
mod many_projects;
mod memory;
mod models;
mod modes;
mod outside;
mod passkeys;
mod placement;
mod projects;
mod providers;
mod questions;
mod remote;
mod schedules;
mod server;
mod site;
mod standing;
mod steer;
mod subagents;
mod tasks;
mod titles;
mod tracing;
mod trust;
mod turn_streams;
mod watching;
mod worktrees;
