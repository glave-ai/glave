//! computer's integration tests, one program: each file is a module of it.

#![cfg(test)]

#[path = "support/faulty.rs"]
mod faulty;
#[path = "support/ledger.rs"]
mod ledger;

mod faults;
mod remote;
