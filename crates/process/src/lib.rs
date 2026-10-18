//! Every operating-system process glave owns starts here, so none is started without an owner
//! (`specs/one-server.md`, "No orphaned processes").
//!
//! A [`Program`] is what to run. [`Processes::spawn`] starts one that runs on its own: in a
//! process group of its own, recorded in the ledger under `GLAVE_HOME/run/processes` with its
//! owner while it runs, and stopped with everything in its group when its [`Child`] goes.
//! [`Processes::reap`], as a glave starts, stops what a glave that is gone left running; a
//! process whose start time differs from the one recorded has the same number but is another
//! process, and is never touched. A program its caller waits for ([`Program::output`]) ends
//! with the call, and the person's server ([`Program::detach`]) is a glave of its own.

mod child;
mod error;
mod group;
mod ledger;
mod owner;
mod processes;
mod program;
mod reap;
mod start;

pub use child::Child;
pub use error::ProcessError;
pub use owner::Owner;
pub use processes::Processes;
pub use program::Program;
