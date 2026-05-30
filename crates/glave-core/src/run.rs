//! Run orchestration — the application service that ties an agent, a runtime,
//! and a trace sink together around the oracle. Depends only on ports.

use anyhow::Result;

use crate::agent::Agent;
use crate::event::RunEvent;
use crate::oracle::{self, Verdict};
use crate::ports::{TraceSink, WorkspaceRuntime};
use crate::task::TaskSpec;

pub fn run_task(
    task: &TaskSpec,
    run_id: &str,
    agent_label: &str,
    agent: &mut dyn Agent,
    rt: &mut dyn WorkspaceRuntime,
    trace: &mut dyn TraceSink,
) -> Result<Verdict> {
    let start = std::time::Instant::now();
    trace.emit(RunEvent::run_started(run_id, task, agent_label))?;
    agent.act(rt, task, run_id, trace)?;
    let verdict = oracle::evaluate(rt, task)?;
    trace.emit(RunEvent::oracle_result(run_id, &verdict))?;
    let dur = u64::try_from(start.elapsed().as_millis()).unwrap_or(u64::MAX);
    trace.emit(RunEvent::run_completed(run_id, verdict.solved, dur))?;
    Ok(verdict)
}
