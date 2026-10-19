//! gateway's integration tests, one program: each file is a module of it.

#![cfg(test)]

#[path = "support/fake.rs"]
mod support;

mod budgets;
mod fallback;
mod routes;
