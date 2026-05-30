//! Glave CLI — the composition root. Wires concrete adapters (Docker runtime,
//! OpenRouter provider, JSONL sink) into the core's orchestration.

mod ingest;

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
        #[arg(long)]
        model: Option<String>,
    },
    /// Validate that a task's oracle discriminates: noop must fail, solve must pass.
    Validate {
        /// Path to the task directory.
        task: PathBuf,
    },
    /// Ingest a real dataset instance (Multi-SWE-bench) into a runnable task.
    Ingest {
        /// Path to the dataset .jsonl file.
        #[arg(long)]
        dataset: PathBuf,
        /// 0-based instance index within the file.
        #[arg(long, default_value_t = 0)]
        index: usize,
        /// Output task directory (e.g. .ingest/rust/bytes-0).
        #[arg(long)]
        out: PathBuf,
    },
    /// Score a task-set: the harnessed model, optionally vs a frontier model in
    /// the same harness (armed vs armed).
    Score {
        /// Task dirs, or directories to walk for tasks (e.g. `tasks/`).
        tasks: Vec<PathBuf>,
        /// The model under test (default: the built-in econo model).
        #[arg(long)]
        model: Option<String>,
        /// Frontier model to compare against, run in the SAME harness.
        #[arg(long)]
        baseline: Option<String>,
        /// Also run the no-tools one-shot probe (a control, not a fair baseline).
        #[arg(long)]
        with_bare: bool,
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
        Cmd::Ingest { dataset, index, out } => {
            ingest::ingest(&ingest::IngestArgs { dataset: &dataset, index, out: &out })
        }
        Cmd::Score {
            tasks,
            model,
            baseline,
            with_bare,
        } => score(&tasks, &resolve_model(model), baseline.as_deref(), with_bare),
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
            (Box::new(ModelAgent::new(Box::new(provider), policy)), label)
        }
    };

    let mut rt = DockerRuntime::prepare(&task, &run_id).context("preparing docker runtime")?;
    let mut sink = JsonlSink::create(Path::new("traces"), &run_id)?;
    let outcome = run_task(&task, &run_id, &label, agent.as_mut(), &mut rt, &mut sink);
    rt.cleanup();
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

#[derive(Copy, Clone)]
enum Mode {
    Bare,
    Harness,
}

/// Run one task with one model in one mode; read back outcome + cost.
fn run_one(task_dir: &Path, model: &str, mode: Mode) -> Result<Stat> {
    let task = TaskSpec::load(task_dir)?;
    let run_id = uuid::Uuid::new_v4().to_string();
    let provider = OpenRouterProvider::from_env(model.to_string())?;
    let (mut agent, label): (Box<dyn Agent>, String) = match mode {
        Mode::Harness => {
            let policy = Box::new(AutonomousPolicy::new(&task.editable_surface));
            (
                Box::new(ModelAgent::new(Box::new(provider), policy)),
                format!("harness:{model}"),
            )
        }
        Mode::Bare => (
            Box::new(OneShotAgent::new(Box::new(provider))),
            format!("bare:{model}"),
        ),
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

fn score(task_paths: &[PathBuf], model: &str, baseline: Option<&str>, with_bare: bool) -> Result<()> {
    let tasks = discover_tasks(task_paths)?;
    if tasks.is_empty() {
        return Err(anyhow!("no tasks found under the given path(s)"));
    }
    println!(
        "scoring {} task(s): {model} (armed){}{}\n",
        tasks.len(),
        baseline.map_or_else(String::new, |b| format!(" vs {b} (armed)")),
        if with_bare { " [+bare probe]" } else { "" }
    );

    let mut q_solved = 0usize;
    let mut q_cost = 0.0f64;
    let mut f_solved = 0usize;
    let mut f_cost = 0.0f64;
    let mut b_solved = 0usize;
    let mut b_cost = 0.0f64;

    for t in &tasks {
        let id = task_name(t);
        let q = run_one(t, model, Mode::Harness)?;
        if q.solved {
            q_solved += 1;
        }
        q_cost += q.cost;
        let mut line = format!("{:<34} {:>14}", clip_id(&id, 34), mark(q.solved));

        if let Some(b) = baseline {
            let f = run_one(t, b, Mode::Harness)?;
            if f.solved {
                f_solved += 1;
            }
            f_cost += f.cost;
            line.push_str(&format!(" {:>14}", mark(f.solved)));
        }
        if with_bare {
            let bare = run_one(t, model, Mode::Bare)?;
            if bare.solved {
                b_solved += 1;
            }
            b_cost += bare.cost;
            line.push_str(&format!(" {:>6}", mark(bare.solved)));
        }
        println!("{line}");
    }

    let n = tasks.len();
    println!("\n--- scorecard ---");
    println!(
        "  {model} (armed):  {q_solved}/{n} solved   ${q_cost:.4}   $/correct {}",
        per_correct(q_cost, q_solved)
    );
    if let Some(b) = baseline {
        println!(
            "  {b} (armed):  {f_solved}/{n} solved   ${f_cost:.4}   $/correct {}",
            per_correct(f_cost, f_solved)
        );
        println!("  --- promise vs frontier ---");
        match (q_solved, f_solved) {
            (_, 0) => println!("  relative success: n/a (frontier solved 0)"),
            (q, f) => println!(
                "  relative success: {:.0}% of frontier",
                ratio(q, f) * 100.0
            ),
        }
        match (per_correct_val(q_cost, q_solved), per_correct_val(f_cost, f_solved)) {
            (Some(qp), Some(fp)) if fp > 0.0 => println!(
                "  relative $/correct: {:.1}% of frontier   (frontier costs {:.0}x more)",
                qp / fp * 100.0,
                fp / qp
            ),
            _ => println!("  relative $/correct: n/a"),
        }
    }
    if with_bare {
        println!("  bare (one-shot probe):  {b_solved}/{n} solved   ${b_cost:.4}");
    }
    Ok(())
}

fn mark(solved: bool) -> &'static str {
    if solved {
        "SOLVED"
    } else {
        "—"
    }
}

fn ratio(a: usize, b: usize) -> f64 {
    if b == 0 {
        0.0
    } else {
        a as f64 / b as f64
    }
}

fn per_correct(cost: f64, solved: usize) -> String {
    per_correct_val(cost, solved).map_or_else(|| "n/a".to_string(), |v| format!("${v:.4}"))
}

fn per_correct_val(cost: f64, solved: usize) -> Option<f64> {
    if solved == 0 {
        None
    } else {
        Some(cost / solved as f64)
    }
}

fn task_name(t: &Path) -> String {
    t.file_name()
        .map_or_else(String::new, |s| s.to_string_lossy().into_owned())
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
