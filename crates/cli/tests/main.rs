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
mod connections;
mod credentials;
mod e2e;
mod gateway;
mod https;
mod mcp;
mod memory;
mod orphans;
mod postgres;
mod project;
mod projects;
mod providers;
mod remote;
mod serve;
mod server;
mod sessions;
mod settings;
mod skills;
mod subagents;
mod tasks;
mod telemetry;
mod threads;
mod turns;
