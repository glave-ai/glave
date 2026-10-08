//! glave's eval factory. It runs the harness in-process, with no CLI in between, on task
//! folders laid out the way Harbor lays them out, against real models. A run is graded only
//! from the thread's events, the files left behind and each task's own test script, never by
//! another model, so it needs no person and no judge.
//!
//! `cargo xtask eval` runs it; see `glave-eval --help` for the options.

mod check;
pub mod cli;
mod error;
mod oracle;
mod prepare;
mod report;
mod run;
mod suite;
mod task;
mod trace;

pub use check::Check;
pub use error::EvalError;
pub use oracle::oracle;
pub use report::Report;
pub use run::{Attempt, Setup, attempt};
pub use suite::{Plan, run};
pub use task::{Task, Tier, load, load_all};
