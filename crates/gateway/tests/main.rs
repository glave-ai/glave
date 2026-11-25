//! gateway's integration tests, one program: each file is a module of it.

#![cfg(test)]

#[path = "support/fake.rs"]
mod support;

mod credits;
mod end_users;
mod fallback;
mod filters;
mod floor;
mod media;
mod metering;
mod routes;
mod search;
