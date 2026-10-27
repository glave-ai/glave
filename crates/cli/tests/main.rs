//! cli's integration tests, one program: each file is a module of it.

#![cfg(test)]

#[path = "support/http.rs"]
mod http;
#[path = "../../session/tests/support/issuer.rs"]
#[expect(
    dead_code,
    reason = "the session's test issuer; these tests use some of it"
)]
mod issuer;
#[path = "support/served.rs"]
mod served;
#[path = "support/binary.rs"]
mod support;

mod cloud;
mod computer;
mod config;
mod connected;
mod connections;
mod credentials;
mod e2e;
mod folders;
mod gateway;
mod https;
mod installed;
mod mcp;
mod memory;
mod onpath;
mod orphans;
mod postgres;
mod project;
mod projects;
mod providers;
mod remote;
mod search;
mod serve;
mod server;
mod sessions;
mod settings;
mod skills;
mod starter;
mod subagents;
mod tasks;
mod telemetry;
mod threads;
mod turns;
