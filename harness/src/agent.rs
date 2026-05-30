//! Agents. Slice 1 ships two stubs (no model). The real model-driven agent
//! implements this same trait in slice 2, driven by the gateway.

use anyhow::Result;
use std::path::Path;

use crate::runtime::WorkspaceRuntime;
use crate::task::TaskSpec;
use crate::trace::{RunEvent, Trace};

pub trait Agent {
    fn act(
        &mut self,
        rt: &mut dyn WorkspaceRuntime,
        task: &TaskSpec,
        run_id: &str,
        trace: &mut Trace,
    ) -> Result<()>;
}

/// Makes no changes. The buggy workspace should stay UNSOLVED — proves the
/// oracle isn't a rubber stamp.
pub struct NoopAgent;

impl Agent for NoopAgent {
    fn act(
        &mut self,
        _rt: &mut dyn WorkspaceRuntime,
        _task: &TaskSpec,
        run_id: &str,
        trace: &mut Trace,
    ) -> Result<()> {
        trace.emit(RunEvent::agent_action(
            run_id,
            "noop",
            None,
            Some("stub agent made no changes".to_string()),
        ))
    }
}

/// Applies the task's reference solution. Should become SOLVED — proves the
/// full pipeline (mount -> edit -> oracle) works end to end.
pub struct SolveAgent;

impl Agent for SolveAgent {
    fn act(
        &mut self,
        rt: &mut dyn WorkspaceRuntime,
        task: &TaskSpec,
        run_id: &str,
        trace: &mut Trace,
    ) -> Result<()> {
        let sol = task.solution_dir();
        let mut files = Vec::new();
        collect(&sol, &sol, &mut files)?;
        files.sort();
        for rel in files {
            let contents = std::fs::read_to_string(sol.join(&rel))?;
            rt.write_file(&rel, &contents)?;
            trace.emit(RunEvent::agent_action(
                run_id,
                "write_file",
                Some(rel.clone()),
                Some("applied reference solution".to_string()),
            ))?;
        }
        Ok(())
    }
}

fn collect(root: &Path, dir: &Path, out: &mut Vec<String>) -> Result<()> {
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let p = entry.path();
        if p.is_dir() {
            collect(root, &p, out)?;
        } else if let Ok(rel) = p.strip_prefix(root) {
            out.push(rel.to_string_lossy().replace('\\', "/"));
        }
    }
    Ok(())
}
