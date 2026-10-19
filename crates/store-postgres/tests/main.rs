//! store-postgres's integration tests, one program: each file is a module of it.

#![cfg(test)]

#[path = "support/scratch.rs"]
mod support;

mod postgres;
mod sealing;
