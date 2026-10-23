//! memory's integration tests, one program: each file is a module of it.

#![cfg(test)]

#[path = "support/kit.rs"]
mod support;

mod conformance;
mod hidden;
mod kept;
mod loading;
mod recall;
mod safety;
mod store;
mod tidy;
