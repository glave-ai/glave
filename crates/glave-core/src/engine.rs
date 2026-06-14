//! The run engine — the single entry point for "run an agent task."
//!
//! The CLI (dev / eval), the serving layer (product), and the future
//! auto-research loop all call [`RunEngine::execute`], so there is exactly one
//! code path. It depends only on ports; concrete providers and runtimes are
//! supplied by [`ProviderFactory`]/[`RuntimeFactory`], which makes the engine
//! both testable and trivially reusable across the CLI and a multi-tenant server.

use std::sync::Arc;

use anyhow::Result;
use serde::Serialize;

use crate::agent::{Agent, ModelAgent, NoopAgent, SolveAgent};
use crate::event::RunEvent;
use crate::policy::AutonomousPolicy;
use crate::ports::{ProviderFactory, RuntimeFactory, TraceSink};
use crate::runtime::{Plan, Place, Sandbox, Source};
use crate::run::run_task;
use crate::task::TaskSpec;

/// Which agent drives a run.
#[derive(Debug, Clone)]
pub enum AgentSpec {
    /// The full harnessed agent on the given model — the product path.
    Model { model: String },
    /// Stub: apply the reference solution (for the oracle discrimination check).
    Solve,
    /// Stub: change nothing (for the oracle discrimination check).
    Noop,
}

/// One unit of work for the engine.
#[derive(Debug, Clone)]
pub struct RunRequest {
    pub run_id: String,
    pub task: TaskSpec,
    pub agent: AgentSpec,
}

/// The structured result of a run — everything a caller needs without re-reading
/// the trace sink.
#[derive(Debug, Clone, Serialize)]
pub struct RunOutcome {
    pub run_id: String,
    pub task_id: String,
    pub agent_label: String,
    pub solved: bool,
    pub fail_to_pass_ok: bool,
    pub pass_to_pass_ok: bool,
    pub cost_usd: f64,
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    /// Reasoning tokens summed across turns. Zero here while reasoning is
    /// configured "on" means the provider silently ignored it — an unfair leg.
    pub reasoning_tokens: u64,
}

/// Holds the factories that mint a fresh provider and runtime per run. Cheap to
/// clone (`Arc`), `Send + Sync` — one instance serves the CLI and every
/// concurrent server request.
#[derive(Clone)]
pub struct RunEngine {
    providers: Arc<dyn ProviderFactory>,
    runtimes: Arc<dyn RuntimeFactory>,
}

impl RunEngine {
    #[must_use]
    pub fn new(providers: Arc<dyn ProviderFactory>, runtimes: Arc<dyn RuntimeFactory>) -> Self {
        Self {
            providers,
            runtimes,
        }
    }

    /// Run one task to completion. Every trace event is forwarded to `sink`
    /// (JSONL for the CLI, a channel for streaming, in-memory for tests); the
    /// outcome is returned directly. The runtime is prepared per run and cleaned
    /// up when it drops at the end of this call — including on the error path.
    pub fn execute(&self, req: &RunRequest, sink: &mut dyn TraceSink) -> Result<RunOutcome> {
        let (mut agent, label) = self.build_agent(&req.agent, &req.task)?;
        // TaskSpec -> Plan at the eval edge; the oracle dir is passed alongside,
        // never inside the plan, so it reaches only the grader.
        let plan = eval_plan(&req.task);
        let oracle = req.task.oracle_dir();
        let mut rt = self.runtimes.prepare(&plan, &oracle, &req.run_id)?;
        let mut tap = OutcomeTap::new(sink);
        let verdict = run_task(
            &req.task,
            &req.run_id,
            &label,
            agent.as_mut(),
            rt.as_mut(),
            &mut tap,
        )?;
        Ok(RunOutcome {
            run_id: req.run_id.clone(),
            task_id: req.task.id.clone(),
            agent_label: label,
            solved: verdict.solved,
            fail_to_pass_ok: verdict.fail_to_pass_ok,
            pass_to_pass_ok: verdict.pass_to_pass_ok,
            cost_usd: tap.cost_usd,
            prompt_tokens: tap.prompt_tokens,
            completion_tokens: tap.completion_tokens,
            reasoning_tokens: tap.reasoning_tokens,
        })
    }

    fn build_agent(&self, spec: &AgentSpec, task: &TaskSpec) -> Result<(Box<dyn Agent>, String)> {
        let built: (Box<dyn Agent>, String) = match spec {
            AgentSpec::Model { model } => {
                let provider = self.providers.create(model)?;
                let policy = Box::new(AutonomousPolicy::new(&task.editable_surface));
                (
                    Box::new(ModelAgent::new(provider, policy)),
                    format!("model:{model}"),
                )
            }
            AgentSpec::Solve => (Box::new(SolveAgent), "stub:solve".to_string()),
            AgentSpec::Noop => (Box::new(NoopAgent), "stub:noop".to_string()),
        };
        Ok(built)
    }
}

/// Map a task to its eval [`Plan`] — the `TaskSpec → Plan` boundary lives HERE,
/// at the eval edge, so no domain type ever crosses the `RuntimeFactory` port.
fn eval_plan(task: &TaskSpec) -> Plan {
    Plan::Sandboxed {
        place: Place::Local,
        source: Source::CopyFrom(task.workspace_dir()),
        sandbox: Sandbox {
            image: task.image.base.clone(),
            workdir: task.image.workdir.clone(),
            mem_mb: task.resource.mem_mb,
            cpus: task.resource.cpus,
            pids: task.resource.pids,
            network: task.resource.network.clone(),
        },
    }
}

/// Wraps the caller's sink: forwards every event unchanged while skimming the
/// run's outcome (solved + cost + tokens), so the engine returns a structured
/// result instead of making callers re-read the trace.
struct OutcomeTap<'a> {
    inner: &'a mut dyn TraceSink,
    solved: bool,
    fail_to_pass_ok: bool,
    pass_to_pass_ok: bool,
    cost_usd: f64,
    prompt_tokens: u64,
    completion_tokens: u64,
    reasoning_tokens: u64,
}

impl<'a> OutcomeTap<'a> {
    fn new(inner: &'a mut dyn TraceSink) -> Self {
        Self {
            inner,
            solved: false,
            fail_to_pass_ok: false,
            pass_to_pass_ok: false,
            cost_usd: 0.0,
            prompt_tokens: 0,
            completion_tokens: 0,
            reasoning_tokens: 0,
        }
    }
}

impl TraceSink for OutcomeTap<'_> {
    fn emit(&mut self, event: RunEvent) -> Result<()> {
        match &event {
            RunEvent::OracleResult {
                solved,
                fail_to_pass_ok,
                pass_to_pass_ok,
                ..
            } => {
                self.solved = *solved;
                self.fail_to_pass_ok = *fail_to_pass_ok;
                self.pass_to_pass_ok = *pass_to_pass_ok;
            }
            RunEvent::CostUsage {
                cost_usd,
                prompt_tokens,
                completion_tokens,
                ..
            } => {
                self.cost_usd += *cost_usd;
                self.prompt_tokens += *prompt_tokens;
                self.completion_tokens += *completion_tokens;
            }
            RunEvent::ModelTurn {
                reasoning_tokens, ..
            } => {
                self.reasoning_tokens += *reasoning_tokens;
            }
            _ => {}
        }
        self.inner.emit(event)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sinks::InMemorySink;
    use crate::testkit::{InMemoryProvider, InMemoryRuntime};

    struct FixedProviderFactory;
    impl ProviderFactory for FixedProviderFactory {
        fn create(&self, _model: &str) -> Result<Box<dyn crate::ports::ModelProvider>> {
            // Scripted native tool calls: create the file, then submit.
            Ok(Box::new(InMemoryProvider::with_tool_calls(vec![
                vec![("create", r#"{"path":"solver.py","content":"FIXED"}"#)],
                vec![("submit", "{}")],
            ])))
        }
    }

    struct FixedRuntimeFactory;
    impl RuntimeFactory for FixedRuntimeFactory {
        fn prepare(
            &self,
            _plan: &Plan,
            _oracle: &std::path::Path,
            _run_id: &str,
        ) -> Result<Box<dyn crate::ports::EvalRuntime>> {
            Ok(Box::new(
                InMemoryRuntime::new().with_file("solver.py", "BUGGY"),
            ))
        }
    }

    fn task() -> Result<TaskSpec> {
        TaskSpec::from_toml(
            "id = \"t\"\nlanguage = \"python\"\narchetype = \"a\"\ndifficulty = \"easy\"\n\
             [image]\nbase = \"x\"\n[oracle]\nfail_to_pass = \"true\"\n",
        )
    }

    #[test]
    fn engine_runs_and_returns_structured_outcome() -> Result<()> {
        let engine = RunEngine::new(
            Arc::new(FixedProviderFactory),
            Arc::new(FixedRuntimeFactory),
        );
        let req = RunRequest {
            run_id: "r1".to_string(),
            task: task()?,
            agent: AgentSpec::Model {
                model: "any".to_string(),
            },
        };
        let mut sink = InMemorySink::default();
        let outcome = engine.execute(&req, &mut sink)?;

        assert_eq!(outcome.run_id, "r1");
        assert_eq!(outcome.agent_label, "model:any");
        // `fail_to_pass = "true"` always passes, so the run is solved.
        assert!(outcome.solved);
        // Events were also forwarded to the caller's sink, not swallowed.
        assert!(sink
            .events
            .iter()
            .any(|e| matches!(e, RunEvent::RunCompleted { .. })));
        Ok(())
    }

    #[test]
    fn engine_outcome_matches_forwarded_sink() -> Result<()> {
        let engine = RunEngine::new(
            Arc::new(FixedProviderFactory),
            Arc::new(FixedRuntimeFactory),
        );
        let req = RunRequest {
            run_id: "r2".to_string(),
            task: task()?,
            agent: AgentSpec::Noop,
        };
        let mut sink = InMemorySink::default();
        let outcome = engine.execute(&req, &mut sink)?;
        // The tap's skim agrees with the sink's own read-back.
        assert_eq!(outcome.solved, sink.solved());
        Ok(())
    }
}
