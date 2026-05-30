//! Glave CLI — the composition root. Wires concrete adapters (Docker runtime,
//! OpenRouter provider, JSONL sink) into the core's orchestration. This is the
//! only place that knows about all three.

use anyhow::{anyhow, Context, Result};
use clap::{Parser, Subcommand, ValueEnum};
use std::path::{Path, PathBuf};
use std::process::Command;

use glave_core::agent::{Agent, ModelAgent, NoopAgent, SolveAgent};
use glave_core::run::run_task;
use glave_core::sinks::JsonlSink;
use glave_core::{AutonomousPolicy, ModelProvider, TaskSpec};
use glave_docker::DockerRuntime;
use glave_openrouter::{reasoning_effort, OpenRouterProvider};

const DEFAULT_MODEL: &str = "qwen/qwen3.6-27b";

#[derive(Parser)]
#[command(name = "glave", about = "Glave eval harness")]
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
    /// Run one task with the chosen agent and write a trace.
    Run {
        /// Path to the task directory.
        task: PathBuf,
        #[arg(long, value_enum, default_value_t = AgentKind::Solve)]
        agent: AgentKind,
        /// OpenRouter model slug for `--agent model`
        /// (else GLAVE_MODEL env, else the built-in default).
        #[arg(long)]
        model: Option<String>,
    },
    /// Validate that a task's oracle discriminates: noop must fail, solve must pass.
    Validate {
        /// Path to the task directory.
        task: PathBuf,
    },
}

#[derive(Copy, Clone, ValueEnum)]
enum AgentKind {
    Noop,
    Solve,
    Model,
}

fn main() -> Result<()> {
    let _ = dotenvy::dotenv();
    let cli = Cli::parse();
    match cli.cmd {
        Cmd::BuildBase { lang } => build_base(&lang),
        Cmd::Run { task, agent, model } => {
            do_run(&task, agent, &resolve_model(model))?;
            Ok(())
        }
        Cmd::Validate { task } => validate(&task),
    }
}

fn resolve_model(flag: Option<String>) -> String {
    flag.or_else(|| std::env::var("GLAVE_MODEL").ok())
        .unwrap_or_else(|| DEFAULT_MODEL.to_string())
}

fn build_base(lang: &str) -> Result<()> {
    let ctx = Path::new("images").join(lang);
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

fn do_run(task_dir: &Path, kind: AgentKind, model: &str) -> Result<bool> {
    let task = TaskSpec::load(task_dir)?;
    let run_id = uuid::Uuid::new_v4().to_string();

    let (mut agent, label): (Box<dyn Agent>, String) = match kind {
        AgentKind::Noop => (Box::new(NoopAgent), "stub:noop".to_string()),
        AgentKind::Solve => (Box::new(SolveAgent), "stub:solve".to_string()),
        AgentKind::Model => {
            let provider = OpenRouterProvider::from_env(model.to_string())?;
            let label = format!("model:{} reasoning={}", provider.model_id(), reasoning_effort());
            let policy = Box::new(AutonomousPolicy::new(&task.editable_surface));
            (
                Box::new(ModelAgent::new(Box::new(provider), policy)),
                label,
            )
        }
    };

    let mut rt = DockerRuntime::prepare(&task, &run_id).context("preparing docker runtime")?;
    let mut sink = JsonlSink::create(Path::new("traces"), &run_id)?;

    let outcome = run_task(&task, &run_id, &label, agent.as_mut(), &mut rt, &mut sink);
    rt.cleanup(); // best-effort teardown regardless of outcome
    let verdict = outcome?;

    println!(
        "task={} agent={} solved={} (f2p_ok={}, p2p_ok={})",
        task.id, label, verdict.solved, verdict.fail_to_pass_ok, verdict.pass_to_pass_ok
    );
    println!("trace: {}", sink.path().display());
    Ok(verdict.solved)
}

fn validate(task_dir: &Path) -> Result<()> {
    println!("validating {}\n", task_dir.display());
    let noop_solved = do_run(task_dir, AgentKind::Noop, "")?;
    println!();
    let solve_solved = do_run(task_dir, AgentKind::Solve, "")?;

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
