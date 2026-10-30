//! The session API's tests, one binary: the conformance suite every transport passes, then
//! what each part of the API does beyond it, against a scripted model. All of it is test code,
//! where clippy lets a test unwrap and index.

#![cfg(test)]

mod support;

mod accounts;
mod awake;
mod background_commands;
mod browsers;
mod changes;
mod choices;
mod commands;
mod conformance;
mod connections;
mod credentials;
mod discover;
mod durable;
mod files;
mod goals;
mod gone;
mod grouping;
mod homes;
mod http;
mod invites;
mod isolation;
mod languages;
mod leases;
mod list_watching;
mod listing;
mod local;
mod mailing;
mod many_projects;
mod memory;
mod models;
mod never_projects;
mod outside;
mod passkeys;
mod people;
mod permissions;
mod placement;
mod plans;
mod projects;
mod providers;
mod questions;
mod recovery;
mod remembering;
mod remote;
mod removing;
mod schedules;
mod server;
mod sharing;
mod signups;
mod site;
mod sources;
mod standing;
mod steer;
mod steps;
mod subagents;
mod tasks;
mod thought;
mod titles;
mod tracing;
mod trust;
mod turn_streams;
mod watching;
mod worktrees;
