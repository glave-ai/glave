//! Glave CLI — the composition root. Wires concrete adapters (Docker runtime,
//! OpenRouter provider, JSONL sink) into the core's orchestration.

mod baseline_cache;
mod ingest;

use anyhow::{anyhow, Context, Result};
use baseline_cache::{BaselineCache, Trial};
use clap::{Parser, Subcommand, ValueEnum};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

use glave_core::engine::{AgentSpec, RunEngine, RunRequest};
use glave_core::sinks::JsonlSink;
use glave_core::TaskSpec;
use glave_docker::DockerRuntimeFactory;
use glave_openrouter::OpenRouterFactory;

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
        /// Source language recipe (rust, go).
        #[arg(long, default_value = "rust")]
        lang: String,
    },
    /// Run the agent-run API server — the brain, server-side. Clients POST a
    /// task to /v1/runs and receive the run streamed as SSE.
    Serve {
        /// Address to bind (host:port).
        #[arg(long, default_value = "127.0.0.1:8080")]
        addr: String,
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
        /// Trials per task — solve-rate over N runs beats noisy single shots.
        #[arg(long, default_value_t = 1)]
        trials: usize,
        /// Ignore the cached baseline and re-run (re-pay) the frontier leg.
        #[arg(long)]
        refresh_baseline: bool,
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
        Cmd::Ingest { dataset, index, out, lang } => {
            ingest::ingest(&ingest::IngestArgs { dataset: &dataset, index, out: &out, lang: &lang })
        }
        Cmd::Serve { addr } => {
            let socket = addr.parse().context("parsing --addr (expected host:port)")?;
            glave_serve::serve_blocking(socket, build_engine())
        }
        Cmd::Score {
            tasks,
            model,
            baseline,
            trials,
            refresh_baseline,
        } => score(
            &tasks,
            &resolve_model(model),
            baseline.as_deref(),
            trials,
            refresh_baseline,
        ),
    }
}

fn resolve_model(flag: Option<String>) -> String {
    flag.or_else(|| std::env::var("GLAVE_MODEL").ok())
        .unwrap_or_else(|| DEFAULT_MODEL.to_string())
}

/// The single run engine the whole CLI shares: OpenRouter providers + Docker
/// sandboxes behind the core's factory ports.
fn build_engine() -> RunEngine {
    RunEngine::new(
        Arc::new(OpenRouterFactory::from_env()),
        Arc::new(DockerRuntimeFactory),
    )
}

/// Fairness check: warn loudly if a model leg ran with reasoning configured but
/// produced zero reasoning tokens — the provider silently ignored it, which makes
/// the comparison unfair. Stub agents (noop/solve) don't call a model, so skip.
fn check_reasoning(agent_label: &str, reasoning_tokens: u64) {
    if !agent_label.starts_with("model:") {
        return;
    }
    let on = !matches!(
        std::env::var("GLAVE_REASONING").as_deref().unwrap_or("high"),
        "off" | "none" | ""
    );
    if on && reasoning_tokens == 0 {
        eprintln!(
            "WARNING: {agent_label} produced 0 reasoning tokens while reasoning is ON \
             — the provider may be ignoring it; this leg's comparison is unfair."
        );
    }
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
    let agent = match kind {
        AgentKind::Noop => AgentSpec::Noop,
        AgentKind::Solve => AgentSpec::Solve,
        AgentKind::Model => AgentSpec::Model { model: model.to_string() },
    };
    let mut sink = JsonlSink::create(Path::new("traces"), &run_id)?;
    let req = RunRequest { run_id, task, agent };
    let outcome = build_engine().execute(&req, &mut sink)?;

    println!(
        "task={} agent={} solved={} (f2p_ok={}, p2p_ok={}) reasoning_tokens={}",
        outcome.task_id,
        outcome.agent_label,
        outcome.solved,
        outcome.fail_to_pass_ok,
        outcome.pass_to_pass_ok,
        outcome.reasoning_tokens
    );
    check_reasoning(&outcome.agent_label, outcome.reasoning_tokens);
    println!("trace: {}", sink.path().display());
    Ok(outcome.solved)
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

/// Aggregate over task × trial runs for one model.
#[derive(Default)]
struct Agg {
    solved: usize,
    runs: usize,
    cost: f64,
}

impl Agg {
    fn add(&mut self, solved: usize, runs: usize, cost: f64) {
        self.solved += solved;
        self.runs += runs;
        self.cost += cost;
    }
    fn rate(&self) -> f64 {
        if self.runs == 0 {
            0.0
        } else {
            self.solved as f64 / self.runs as f64
        }
    }
}

/// Run a task `trials` times sequentially (one container at a time — the host
/// can't fan out); return (passes, total cost).
fn run_trials(engine: &RunEngine, task_dir: &Path, model: &str, trials: usize) -> Vec<Trial> {
    let mut out = Vec::with_capacity(trials);
    for trial in 0..trials {
        // A single trial erroring (infra hiccup, model/provider glitch) must not
        // abort the scorecard — count it as unsolved and press on.
        match run_one(engine, task_dir, model) {
            Ok(t) => out.push(t),
            Err(e) => {
                eprintln!(
                    "  WARNING: {} trial {}/{trials} errored (counted unsolved): {e}",
                    task_name(task_dir),
                    trial + 1
                );
                out.push(Trial { solved: false, cost: 0.0 });
            }
        }
    }
    out
}

/// Baseline trials via the cache: reuse what's already paid for, run only the
/// shortfall, persist the union. Returns the trials plus a provenance label.
fn baseline_trials(
    engine: &RunEngine,
    cache: &BaselineCache,
    task_dir: &Path,
    model: &str,
    want: usize,
    refresh: bool,
) -> Result<(Vec<Trial>, &'static str)> {
    let cached = if refresh { None } else { cache.lookup(model, task_dir) };
    let created = cached.as_ref().map_or_else(now_unix, |c| c.created_unix);
    let mut trials = cached.map(|c| c.trials).unwrap_or_default();
    let label = if trials.len() >= want {
        "cached"
    } else if trials.is_empty() {
        "fresh"
    } else {
        "topped-up"
    };
    if trials.len() < want {
        trials.extend(run_trials(engine, task_dir, model, want - trials.len()));
        cache.store(model, task_dir, &task_name(task_dir), trials.clone(), created)?;
    }
    trials.truncate(want);
    Ok((trials, label))
}

fn now_unix() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// Run one task with one model via the engine, keeping the trace: measurement
/// runs are exactly the data the flywheel mines (debugging failures, and LoRA
/// training pairs from successful attempts).
fn run_one(engine: &RunEngine, task_dir: &Path, model: &str) -> Result<Trial> {
    let task = TaskSpec::load(task_dir)?;
    let req = RunRequest {
        run_id: uuid::Uuid::new_v4().to_string(),
        task,
        agent: AgentSpec::Model { model: model.to_string() },
    };
    let mut sink = JsonlSink::create(Path::new("traces"), &req.run_id)?;
    let outcome = engine.execute(&req, &mut sink)?;
    check_reasoning(&outcome.agent_label, outcome.reasoning_tokens);
    Ok(Trial {
        solved: outcome.solved,
        cost: outcome.cost_usd,
    })
}

fn score(
    task_paths: &[PathBuf],
    model: &str,
    baseline: Option<&str>,
    trials: usize,
    refresh_baseline: bool,
) -> Result<()> {
    let tasks = discover_tasks(task_paths)?;
    if tasks.is_empty() {
        return Err(anyhow!("no tasks found under the given path(s)"));
    }
    let trials = trials.max(1);
    println!(
        "scoring {} task(s) x {trials} trial(s): {model} (armed){}\n",
        tasks.len(),
        baseline.map_or_else(String::new, |b| format!(" vs {b} (armed)")),
    );

    let engine = build_engine();
    let cache = BaselineCache::new(PathBuf::from("cache/baselines"));
    let mut q = Agg::default();
    let mut f = Agg::default();
    let mut cached_legs = 0usize;

    for t in &tasks {
        let id = task_name(t);
        let qt = run_trials(&engine, t, model, trials);
        let qs = qt.iter().filter(|t| t.solved).count();
        q.add(qs, qt.len(), qt.iter().map(|t| t.cost).sum());
        let mut line = format!("{:<34} {:>10}", clip_id(&id, 34), format!("{qs}/{trials}"));

        if let Some(bl) = baseline {
            let (ft, label) = baseline_trials(&engine, &cache, t, bl, trials, refresh_baseline)?;
            let fs = ft.iter().filter(|t| t.solved).count();
            f.add(fs, ft.len(), ft.iter().map(|t| t.cost).sum());
            if label == "cached" {
                cached_legs += 1;
            }
            line.push_str(&format!(" {:>10}", format!("{fs}/{trials}")));
            if label != "fresh" {
                line.push_str(&format!("  ({label})"));
            }
        }
        println!("{line}");
    }
    if baseline.is_some() && cached_legs > 0 {
        println!(
            "\n  note: {cached_legs}/{} baseline leg(s) reused from cache (--refresh-baseline to re-run)",
            tasks.len()
        );
    }

    println!("\n--- scorecard ({trials} trial(s)/task) ---");
    println!(
        "  {model} (armed):  {}/{} passed ({:.0}%)   ${:.4}   $/correct {}",
        q.solved,
        q.runs,
        q.rate() * 100.0,
        q.cost,
        per_correct(q.cost, q.solved)
    );
    if let Some(bl) = baseline {
        println!(
            "  {bl} (armed):  {}/{} passed ({:.0}%)   ${:.4}   $/correct {}",
            f.solved,
            f.runs,
            f.rate() * 100.0,
            f.cost,
            per_correct(f.cost, f.solved)
        );
        println!("  --- promise vs frontier ---");
        if f.rate() > 0.0 {
            println!(
                "  relative success: {:.0}% of frontier  (pass-rate {:.0}% vs {:.0}%)",
                q.rate() / f.rate() * 100.0,
                q.rate() * 100.0,
                f.rate() * 100.0
            );
        } else {
            println!("  relative success: n/a (frontier passed 0)");
        }
        match (per_correct_val(q.cost, q.solved), per_correct_val(f.cost, f.solved)) {
            (Some(qp), Some(fp)) if fp > 0.0 => println!(
                "  relative $/correct: {:.1}% of frontier   (frontier costs {:.0}x more)",
                qp / fp * 100.0,
                fp / qp
            ),
            _ => println!("  relative $/correct: n/a"),
        }
    }
    Ok(())
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
