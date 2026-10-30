//! The one way glave starts a process that runs on its own: in a group of its own, tied to
//! glave by the lifeline and recorded in the ledger with its owner for as long as it runs.

use std::path::PathBuf;
use std::sync::Arc;

use tokio::io::AsyncWriteExt as _;

use crate::child::Child;
use crate::error::ProcessError;
use crate::ledger::{self, Ledger};
use crate::lifeline::Lifeline;
use crate::lifeline::gate::GO;
use crate::owner::Owner;
use crate::program::{Input, Program};

/// Why a program could not be told it may run.
const NO_GATE: &str = "glave could not hold it before it ran";

/// Where processes are recorded, and for whom the next ones start.
#[derive(Debug, Clone)]
pub struct Processes {
    /// None for a library user with no glave folder: processes are still owned, only not
    /// recorded, so a glave that crashes leaves them behind.
    ledger: Option<Arc<Ledger>>,
    /// None for a library user with no helper: processes are still owned, only not stopped at
    /// once if glave is killed outright.
    lifeline: Option<Arc<Lifeline>>,
    owner: Owner,
}

impl Processes {
    /// Processes recorded in `run` (`GLAVE_HOME/run`), in its `processes` folder, owned by
    /// glave itself until [`Processes::owned_by`] says otherwise.
    pub fn recorded(run: impl Into<PathBuf>) -> Self {
        Self::with_ledger(Ledger::new(run.into().join("processes"), ledger::LIMIT))
    }

    pub(crate) fn with_ledger(ledger: Ledger) -> Self {
        Self {
            ledger: Some(Arc::new(ledger)),
            lifeline: None,
            owner: Owner::Server,
        }
    }

    /// Processes recorded nowhere: for tests, and libraries run without a glave folder.
    pub fn unrecorded() -> Self {
        Self {
            ledger: None,
            lifeline: None,
            owner: Owner::Server,
        }
    }

    /// The same processes, tied to this one by a lifeline: `helper` is this program, which
    /// calls [`crate::lifeline_helper`] first thing in `main`. Whatever it starts then stops at
    /// once when this process ends, however it ends. The helper starts now, and this fails,
    /// saying why, when it does not start as one.
    pub async fn with_lifeline(mut self, helper: impl Into<PathBuf>) -> Result<Self, ProcessError> {
        self.lifeline = Some(Arc::new(Lifeline::start(helper.into()).await?));
        Ok(self)
    }

    /// The lifeline, for the tests that look at its helper.
    #[cfg(test)]
    pub(crate) fn lifeline(&self) -> Option<&Lifeline> {
        self.lifeline.as_deref()
    }

    /// The same ledger and lifeline, for processes started for `owner`.
    #[must_use]
    pub fn owned_by(&self, owner: Owner) -> Self {
        Self {
            ledger: self.ledger.clone(),
            lifeline: self.lifeline.clone(),
            owner,
        }
    }

    /// Starts `program` to run on its own: in a process group of its own (unless it starts
    /// a session itself), tied to glave by the lifeline and recorded for this owner. Refused,
    /// and stopped, when it cannot be tied (the helper is gone and cannot start again) or
    /// recorded: nothing runs without an entry while there is a ledger. With a lifeline it
    /// runs only once both are done: until then glave's own program waits in its place to
    /// become it, and a glave that ends meanwhile leaves nothing of it running (#996).
    pub async fn spawn(&self, program: &mut Program) -> Result<Child, ProcessError> {
        let grouped = program.grouped;
        let Some(lifeline) = &self.lifeline else {
            let command = program.direct();
            if grouped {
                command.process_group(0);
            }
            let child = Child::new(command.spawn().map_err(|error| program.failed(error))?);
            return self.record(child, program).await;
        };
        // It becomes the program only once glave holds it (`lifeline/start.rs`), so one it
        // cannot become is refused here, as starting it directly would refuse it.
        program.found()?;
        let mut command = program.gated(lifeline.starter());
        if grouped {
            command.process_group(0);
        }
        let inner = {
            let _starting = lifeline.starting();
            command.spawn()
        }
        .map_err(|error| program.failed(error))?;
        let mut child = Child::new(inner);
        let gate = child.stdin.take();
        let Some(pid) = child.id().and_then(|pid| i32::try_from(pid).ok()) else {
            child.stop().await;
            return Err(program.failed(std::io::Error::other(NO_GATE)));
        };
        match lifeline.tie(pid).await {
            Ok(tie) => child = child.tied(tie),
            Err(error) => {
                child.stop().await;
                return Err(error);
            }
        }
        let mut child = self.record(child, program).await?;
        // Held by the helper and recorded: now it may run.
        let mut gate = gate.ok_or_else(|| program.failed(std::io::Error::other(NO_GATE)))?;
        if let Err(error) = gate.write_all(GO).await {
            child.stop().await;
            return Err(program.failed(error));
        }
        if program.input == Input::Piped {
            child.stdin = Some(gate);
        }
        Ok(child)
    }

    /// `child`, which `program` started, recorded in the ledger for this owner, when there is
    /// one; stopped, and refused, when it cannot be.
    async fn record(&self, mut child: Child, program: &Program) -> Result<Child, ProcessError> {
        let pid = child.id().and_then(|pid| i32::try_from(pid).ok());
        let (Some(ledger), Some(pid)) = (&self.ledger, pid) else {
            return Ok(child);
        };
        let (ledger, owner) = (Arc::clone(ledger), self.owner.clone());
        let recorded = tokio::task::spawn_blocking(move || ledger.record(pid, &owner)).await;
        match recorded {
            Ok(Ok(entry)) => Ok(child.recorded(entry)),
            Ok(Err(error)) => {
                child.stop().await;
                Err(error)
            }
            Err(error) => {
                child.stop().await;
                Err(ProcessError::Start {
                    program: program.name(),
                    source: std::io::Error::other(error),
                })
            }
        }
    }

    /// Stops what glaves that are gone left running, as a glave starts: each group whose
    /// leader is still the process recorded (same id, same start) gets SIGTERM, then SIGKILL
    /// after a grace period. An entry whose process has ended, or whose id now belongs to
    /// another process, is forgotten without a signal. A live glave's entries are left alone.
    /// Returns how many groups were stopped.
    pub async fn reap(&self) -> Result<usize, ProcessError> {
        match &self.ledger {
            Some(ledger) => crate::reap::reap(Arc::clone(ledger)).await,
            None => Ok(0),
        }
    }
}

#[cfg(test)]
mod tests;
