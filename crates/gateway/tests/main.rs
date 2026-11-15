//! gateway's integration tests, one program: each file is a module of it.

#![cfg(test)]

#[path = "support/fake.rs"]
mod support;

mod credits;
mod fallback;
mod filters;
mod floor;
mod media;
mod routes;
mod search;
mod sticky;
