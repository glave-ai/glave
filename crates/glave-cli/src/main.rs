//! Glave CLI — the composition root. Wires concrete adapters (Docker runtime,
//! OpenRouter provider, JSONL sink) into the core's orchestration. This is the
//! only place that knows about all three.

use anyhow::{anyhow, Context, Result};
use clap::{Parser, Subcommand, ValueEnum};
use std::path::{Path, PathBuf};
use std::process::Command;

use glave_core::agent::{Agent, ModelAgent, NoopAgent, OneShotAgent, SolveAgent};
use glave_core::run::run_task;
use glave_core::sinks::{InMemorySink, JsonlSink};
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
    /// Score a task-set: bare (one-shot) vs +harness; report solve rate, $, lift.
    Score {
        /// Task dirs, or directories to walk for tasks (e.g. `tasks/`).
        tasks: Vec<PathBuf>,
        #[arg(long)]
        model: Option<String>,
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
        Cmd::Score { tasks, model } => score(&tasks, &resolve_model(model)),
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

struct Stat {
    solved: bool,
    cost: f64,
}

/// Run one task in one config (bare one-shot, or +harness) and read back the
/// outcome + cost from an in-memory trace.
fn run_config(task_dir: &Path, model: &str, harness: bool) -> Result<Stat> {
    let task = TaskSpec::load(task_dir)?;
    let run_id = uuid::Uuid::new_v4().to_string();
    let provider = OpenRouterProvider::from_env(model.to_string())?;
    let (mut agent, label): (Box<dyn Agent>, String) = if harness {
        let policy = Box::new(AutonomousPolicy::new(&task.editable_surface));
        (
            Box::new(ModelAgent::new(Box::new(provider), policy)),
            format!("+harness:{model}"),
        )
    } else {
        (
            Box::new(OneShotAgent::new(Box::new(provider))),
            format!("bare:{model}"),
        )
    };
    let mut rt = DockerRuntime::prepare(&task, &run_id).context("preparing docker runtime")?;
    let mut sink = InMemorySink::default();
    let outcome = run_task(&task, &run_id, &label, agent.as_mut(), &mut rt, &mut sink);
    rt.cleanup();
    outcome?;
    Ok(Stat {
        solved: sink.solved(),
        cost: sink.cost_usd(),
    })
}

fn score(task_paths: &[PathBuf], model: &str) -> Result<()> {
    let tasks = discover_tasks(task_paths)?;
    if tasks.is_empty() {
        return Err(anyhow!("no tasks found under the given path(s)"));
    }
    println!("scoring {} task(s) on {model}: bare (one-shot) vs +harness\n", tasks.len());
    println!("{:<40} {:>9} {:>9}", "task", "bare", "+harness");

    let mut bare_solved = 0usize;
    let mut harn_solved = 0usize;
    let mut bare_cost = 0.0f64;
    let mut harn_cost = 0.0f64;

    for t in &tasks {
        let id = t
            .file_name()
            .map_or_else(String::new, |s| s.to_string_lossy().into_owned());
        let b = run_config(t, model, false)?;
        let h = run_config(t, model, true)?;
        if b.solved {
            bare_solved += 1;
        }
        if h.solved {
            harn_solved += 1;
        }
        bare_cost += b.cost;
        harn_cost += h.cost;
        println!(
            "{:<40} {:>9} {:>9}",
            clip_id(&id, 40),
            if b.solved { "SOLVED" } else { "—" },
            if h.solved { "SOLVED" } else { "—" }
        );
    }

    let n = tasks.len();
    println!("\n--- scorecard ({model}) ---");
    println!(
        "  bare:     {bare_solved}/{n} solved   ${bare_cost:.4}   $/correct {}",
        per_correct(bare_cost, bare_solved)
    );
    println!(
        "  +harness: {harn_solved}/{n} solved   ${harn_cost:.4}   $/correct {}",
        per_correct(harn_cost, harn_solved)
    );
    println!(
        "  harness lift: +{} solved   {}",
        harn_solved.saturating_sub(bare_solved),
        cost_ratio(harn_cost, bare_cost)
    );
    Ok(())
}

fn per_correct(cost: f64, solved: usize) -> String {
    if solved == 0 {
        "n/a".to_string()
    } else {
        format!("${:.4}", cost / solved as f64)
    }
}

fn cost_ratio(harn: f64, bare: f64) -> String {
    if bare <= 0.0 {
        String::new()
    } else {
        format!("({:.1}x the cost)", harn / bare)
    }
}

fn clip_id(s: &str, max: usize) -> String {
    s.chars().take(max).collect()
}

fn discover_tasks(paths: &[PathBuf]) -> Result<Vec<PathBuf>> {
    let mut out = Vec::new();
    for p in paths {
        walk_tasks(p, &mut out)?;
    }
    out.sort();
    out.dedup();
    Ok(out)
}

fn walk_tasks(dir: &Path, out: &mut Vec<PathBuf>) -> Result<()> {
    if dir.join("task.toml").is_file() {
        out.push(dir.to_path_buf());
        return Ok(());
    }
    if dir.is_dir() {
        for entry in std::fs::read_dir(dir)? {
            let p = entry?.path();
            if p.is_dir() {
                walk_tasks(&p, out)?;
            }
        }
    }
    Ok(())
}
