//! Glave eval harness — slice 1: deterministic skeleton.
//!
//! Runs ONE task in an isolated Docker container with a stub (non-model) agent,
//! scores it with a two-sided oracle (FAIL_TO_PASS + PASS_TO_PASS), and writes a
//! flat-file JSONL trace correlated by run id. No model / no network for the
//! agent — that arrives in slice 2 (gateway -> OpenRouter).

mod agent;
mod oracle;
mod runtime;
mod task;
mod trace;

use anyhow::{anyhow, Context, Result};
use clap::{Parser, Subcommand, ValueEnum};
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::agent::{Agent, NoopAgent, SolveAgent};
use crate::runtime::DockerRuntime;
use crate::task::TaskSpec;
use crate::trace::{RunEvent, Trace};

#[derive(Parser)]
#[command(name = "harness", about = "Glave eval harness — slice 1 (deterministic skeleton)")]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Build a per-language base image (docker build).
    BuildBase {
        #[arg(long, default_value = "python")]
        lang: String,
    },
    /// Run one task with a stub agent and write a trace.
    Run {
        /// Path to the task directory.
        task: PathBuf,
        #[arg(long, value_enum, default_value_t = AgentKind::Solve)]
        agent: AgentKind,
    },
    /// Validate that a task's oracle discriminates: noop must fail, solve must pass.
    Validate {
        /// Path to the task directory.
        task: PathBuf,
    },
}

#[derive(Copy, Clone, ValueEnum)]
enum AgentKind {
    /// Does nothing — leaves the buggy workspace as-is (should stay UNSOLVED).
    Noop,
    /// Applies the task's reference solution (should become SOLVED).
    Solve,
}

impl AgentKind {
    fn label(self) -> &'static str {
        match self {
            AgentKind::Noop => "stub:noop",
            AgentKind::Solve => "stub:solve",
        }
    }
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.cmd {
        Cmd::BuildBase { lang } => build_base(&lang),
        Cmd::Run { task, agent } => {
            do_run(&task, agent)?;
            Ok(())
        }
        Cmd::Validate { task } => validate(&task),
    }
}

fn build_base(lang: &str) -> Result<()> {
    let ctx = Path::new("harness/images").join(lang);
    let tag = format!("glave/{lang}-base:latest");
    println!("building {tag} from {}", ctx.display());
    let status = Command::new("docker")
        .args(["build", "-t", &tag, &ctx.to_string_lossy()])
        .status()
        .context("running docker build")?;
    if !status.success() {
        return Err(anyhow!("docker build failed for {tag}"));
    }
    println!("built {tag}");
    Ok(())
}

/// Full pipeline for one task + one stub agent. Returns whether it was solved.
fn do_run(task_dir: &Path, kind: AgentKind) -> Result<bool> {
    let task = TaskSpec::load(task_dir)?;
    let run_id = uuid::Uuid::new_v4().to_string();
    let mut tr = Trace::new(&run_id)?;
    tr.emit(RunEvent::run_started(&run_id, &task, kind.label()))?;

    let start = std::time::Instant::now();
    let mut rt = DockerRuntime::prepare(&task, &run_id).context("preparing docker runtime")?;
    let outcome = run_agent_and_eval(&mut rt, &task, &run_id, &mut tr, kind);
    let _ = rt.cleanup(); // best-effort teardown regardless of outcome
    let verdict = outcome?;

    tr.emit(RunEvent::oracle_result(&run_id, &verdict))?;
    let dur = start.elapsed().as_millis() as u64;
    tr.emit(RunEvent::run_completed(&run_id, verdict.solved, dur))?;

    println!(
        "task={} agent={} solved={} (f2p_ok={}, p2p_ok={}) {}ms",
        task.id, kind.label(), verdict.solved, verdict.fail_to_pass_ok, verdict.pass_to_pass_ok, dur
    );
    println!("trace: {}", tr.path().display());
    Ok(verdict.solved)
}

fn run_agent_and_eval(
    rt: &mut DockerRuntime,
    task: &TaskSpec,
    run_id: &str,
    tr: &mut Trace,
    kind: AgentKind,
) -> Result<oracle::Verdict> {
    let mut agent: Box<dyn Agent> = match kind {
        AgentKind::Noop => Box::new(NoopAgent),
        AgentKind::Solve => Box::new(SolveAgent),
    };
    agent.act(rt, task, run_id, tr)?;
    // Cost is zero for the stub (no model). The event exists from day one so the
    // cost-per-correct contract is in the trace before the gateway lands.
    tr.emit(RunEvent::cost_usage_zero(run_id))?;
    oracle::evaluate(&*rt, task)
}

fn validate(task_dir: &Path) -> Result<()> {
    println!("validating {}\n", task_dir.display());
    let noop_solved = do_run(task_dir, AgentKind::Noop)?;
    println!();
    let solve_solved = do_run(task_dir, AgentKind::Solve)?;

    println!("\n--- discrimination check ---");
    println!("  noop  -> {}", if noop_solved { "SOLVED" } else { "UNSOLVED" });
    println!("  solve -> {}", if solve_solved { "SOLVED" } else { "UNSOLVED" });
    if !noop_solved && solve_solved {
        println!("\nOK: oracle discriminates (noop fails, solve passes). Task is well-formed.");
        Ok(())
    } else {
        Err(anyhow!(
            "oracle does NOT discriminate: noop_solved={noop_solved}, solve_solved={solve_solved} (expected false, true)"
        ))
    }
}
