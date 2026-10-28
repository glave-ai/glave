//! Every operating-system process glave owns starts here, so none is started without an owner
//! (`specs/one-server.md`, "No orphaned processes").
//!
//! A [`Program`] is what to run. [`Processes::spawn`] starts one that runs on its own: in a
//! process group of its own, recorded in the ledger under `GLAVE_HOME/run/processes` with its
//! owner while it runs, and stopped with everything in its group when its [`Child`] goes.
//! With a lifeline ([`Processes::with_lifeline`]), a helper of glave's own stops each group at
//! once when glave ends, however it ends ([`lifeline_helper`]). [`Processes::reap`], as a glave
//! starts, stops what a glave that is gone left running all the same, for when the helper went
//! too; a process whose start time differs from the one recorded has the same number but is
//! another process, and is never touched. A program its caller waits for
//! ([`Processes::output`]) starts the same way, and ends with the call, within the time the
//! caller gives it, its group killed whole if it does not; the person's server
//! ([`Program::detach`]) is a glave of its own. What glave starts outside this computer's
//! processes (workers) names the [`Glave`] that started it, so a later glave tells whether
//! that one is gone.

mod child;
mod error;
mod glave;
mod group;
mod ledger;
mod lifeline;
mod owner;
mod processes;
mod program;
mod reap;
mod start;
#[cfg(test)]
mod testing;
mod waited;
mod within;

pub use child::Child;
pub use error::ProcessError;
pub use glave::Glave;
pub use lifeline::helper::lifeline_helper;
pub use lifeline::this_program;
pub use owner::Owner;
pub use processes::Processes;
pub use program::Program;
