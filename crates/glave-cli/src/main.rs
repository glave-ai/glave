//! Glave CLI — the composition root. Wires concrete adapters (Docker runtime,
//! OpenRouter provider, JSONL sink) into the core's orchestration.

mod baseline_cache;
mod ingest;

use anyhow::{anyhow, Context, Result};
use baseline_cache::{BaselineCache, Trial};
use clap::{Parser, Subcommand, ValueEnum};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

use glave_core::engine::{AgentSpec, RunEngine, RunRequest};
use glave_core::sinks::JsonlSink;
use glave_core::{
    open, step, Decision, Message, ModelProvider, Plan, Policy, ProviderFactory, Role, Session,
    SessionStore, TaskSpec, ToolCall, Turn, Workspace,
};
use glave_docker::DockerRuntimeFactory;
use glave_openrouter::ChatProviderFactory;

const DEFAULT_MODEL: &str = "qwen/qwen3.6-27b";

#[derive(Parser)]
#[command(
    name = "glave",
    about = "Glave — the coding agent (bare `glave` opens an interactive session)"
)]
struct Cli {
    #[command(subcommand)]
    cmd: Option<Cmd>,
}

#[derive(Subcommand)]
enum Cmd {
    /// Interactive product session: the agent works on the current folder. This
    /// is also what bare `glave` runs.
    Chat {
        /// One-shot: run a single prompt and exit (otherwise interactive).
        #[arg(short = 'p', long)]
        prompt: Option<String>,
        /// Resume a session by id (otherwise start a new one).
        #[arg(long)]
        resume: Option<String>,
        #[arg(long)]
        model: Option<String>,
    },
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
        /// Concurrent task workers. Each run's container reserves the task's
        /// [resource] memory (2GB default), so size this to Docker's RAM.
        #[arg(long, default_value_t = 1)]
        jobs: usize,
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
        // Bare `glave` opens an interactive session in the current folder.
        None => chat(None, None, &resolve_model(None)),
        Some(Cmd::Chat {
            prompt,
            resume,
            model,
        }) => chat(prompt, resume, &resolve_model(model)),
        Some(Cmd::BuildBase { lang }) => build_base(&lang),
        Some(Cmd::Run { task, agent, model }) => {
            do_run(&task, agent, &resolve_model(model))?;
            Ok(())
        }
        Some(Cmd::Validate { task }) => validate(&task),
        Some(Cmd::Ingest {
            dataset,
            index,
            out,
            lang,
        }) => ingest::ingest(&ingest::IngestArgs {
            dataset: &dataset,
            index,
            out: &out,
            lang: &lang,
        }),
        Some(Cmd::Serve { addr }) => {
            let socket = addr
                .parse()
                .context("parsing --addr (expected host:port)")?;
            glave_serve::serve_blocking(socket, build_engine())
        }
        Some(Cmd::Score {
            tasks,
            model,
            baseline,
            trials,
            refresh_baseline,
            jobs,
        }) => score(
            &tasks,
            &resolve_model(model),
            baseline.as_deref(),
            trials,
            refresh_baseline,
            jobs,
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
        Arc::new(ChatProviderFactory::from_env()),
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
        std::env::var("GLAVE_REASONING")
            .as_deref()
            .unwrap_or("high"),
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
        AgentKind::Model => AgentSpec::Model {
            model: model.to_string(),
        },
    };
    let mut sink = JsonlSink::create(Path::new("traces"), &run_id)?;
    let req = RunRequest {
        run_id,
        task,
        agent,
    };
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
    println!(
        "  noop  -> {}",
        if noop_solved { "SOLVED" } else { "UNSOLVED" }
    );
    println!(
        "  solve -> {}",
        if solve_solved { "SOLVED" } else { "UNSOLVED" }
    );
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
                out.push(Trial {
                    solved: false,
                    cost: 0.0,
                });
            }
        }
    }
    out
}

/// Baseline trials via the cache: reuse what's already paid for, run only the
/// shortfall, append the new trials. Returns the trials plus a provenance label.
/// A failed cache WRITE only warns — the measured data is still real.
fn baseline_trials(
    engine: &RunEngine,
    cache: &BaselineCache,
    task_dir: &Path,
    model: &str,
    want: usize,
    refresh: bool,
) -> (Vec<Trial>, &'static str) {
    if refresh {
        cache.clear(model, task_dir); // re-pay: drop the old cached lines first
    }
    let mut trials = cache.lookup(model, task_dir);
    let label = if trials.len() >= want {
        "cached"
    } else if trials.is_empty() {
        "fresh"
    } else {
        "topped-up"
    };
    if trials.len() < want {
        let run = run_trials(engine, task_dir, model, want - trials.len());
        if let Err(e) = cache.append(model, task_dir, &run) {
            eprintln!("  WARNING: baseline cache write failed (results kept): {e}");
        }
        trials.extend(run);
    }
    trials.truncate(want);
    (trials, label)
}

/// Run one task with one model via the engine, keeping the trace: measurement
/// runs are exactly the data the flywheel mines (debugging failures, and LoRA
/// training pairs from successful attempts).
fn run_one(engine: &RunEngine, task_dir: &Path, model: &str) -> Result<Trial> {
    let task = TaskSpec::load(task_dir)?;
    let req = RunRequest {
        run_id: uuid::Uuid::new_v4().to_string(),
        task,
        agent: AgentSpec::Model {
            model: model.to_string(),
        },
    };
    let mut sink = JsonlSink::create(Path::new("traces"), &req.run_id)?;
    let outcome = engine.execute(&req, &mut sink)?;
    check_reasoning(&outcome.agent_label, outcome.reasoning_tokens);
    Ok(Trial {
        solved: outcome.solved,
        cost: outcome.cost_usd,
    })
}

/// One task's measured row, produced by a (possibly concurrent) worker.
struct Row {
    q_solved: usize,
    q_runs: usize,
    q_cost: f64,
    baseline: Option<(usize, usize, f64, &'static str)>,
}

/// Measure one task end to end (both legs) and print its line. Infallible by
/// design: per-trial errors are already absorbed as unsolved inside run_trials.
fn score_task(
    engine: &RunEngine,
    cache: &BaselineCache,
    t: &Path,
    model: &str,
    baseline: Option<&str>,
    trials: usize,
    refresh_baseline: bool,
) -> Row {
    let qt = run_trials(engine, t, model, trials);
    let qs = qt.iter().filter(|t| t.solved).count();
    let mut line = format!(
        "{:<34} {:>10}",
        clip_id(&task_name(t), 34),
        format!("{qs}/{trials}")
    );

    let baseline = baseline.map(|bl| {
        let (ft, label) = baseline_trials(engine, cache, t, bl, trials, refresh_baseline);
        let fs = ft.iter().filter(|t| t.solved).count();
        line.push_str(&format!(" {:>10}", format!("{fs}/{trials}")));
        if label != "fresh" {
            line.push_str(&format!("  ({label})"));
        }
        (fs, ft.len(), ft.iter().map(|t| t.cost).sum(), label)
    });
    println!("{line}");

    Row {
        q_solved: qs,
        q_runs: qt.len(),
        q_cost: qt.iter().map(|t| t.cost).sum(),
        baseline,
    }
}

fn score(
    task_paths: &[PathBuf],
    model: &str,
    baseline: Option<&str>,
    trials: usize,
    refresh_baseline: bool,
    jobs: usize,
) -> Result<()> {
    let tasks = discover_tasks(task_paths)?;
    if tasks.is_empty() {
        return Err(anyhow!("no tasks found under the given path(s)"));
    }
    let trials = trials.max(1);
    let jobs = jobs.clamp(1, tasks.len().max(1));
    println!(
        "scoring {} task(s) x {trials} trial(s){}: {model} (armed){}\n",
        tasks.len(),
        if jobs > 1 {
            format!(" across {jobs} workers (rows complete out of order)")
        } else {
            String::new()
        },
        baseline.map_or_else(String::new, |b| format!(" vs {b} (armed)")),
    );

    let engine = build_engine();
    let cache = BaselineCache::new(PathBuf::from("cache/baselines"));

    // Worker pool: an atomic cursor hands out tasks; rows stream back over a
    // channel. The engine was built Send+Sync for concurrent server requests —
    // this is the same property paying off for the eval. Each run's container
    // reserves the task's [resource] memory, so size --jobs to Docker's RAM.
    let next = std::sync::atomic::AtomicUsize::new(0);
    let (tx, rx) = std::sync::mpsc::channel::<Row>();
    std::thread::scope(|s| {
        for _ in 0..jobs {
            let tx = tx.clone();
            let (next, engine, cache, tasks) = (&next, &engine, &cache, &tasks);
            s.spawn(move || loop {
                let i = next.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                let Some(t) = tasks.get(i) else { break };
                let row = score_task(engine, cache, t, model, baseline, trials, refresh_baseline);
                if tx.send(row).is_err() {
                    break;
                }
            });
        }
    });
    drop(tx);

    let mut q = Agg::default();
    let mut f = Agg::default();
    let mut cached_legs = 0usize;
    for row in rx {
        q.add(row.q_solved, row.q_runs, row.q_cost);
        if let Some((fs, fr, fc, label)) = row.baseline {
            f.add(fs, fr, fc);
            if label == "cached" {
                cached_legs += 1;
            }
        }
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
        match (
            per_correct_val(q.cost, q.solved),
            per_correct_val(f.cost, f.solved),
        ) {
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

// ---- interactive product session (`glave` / `glave chat`) ----

const SYSTEM_PROMPT: &str = "You are Glave, a coding agent working in the user's project on their \
machine. Use the tools to read, search, edit files, and run commands. Read a file before editing \
it; keep changes minimal and say what you did. Be concise.";

/// Resolve the project id from `.glave/project.json`, creating the marker (a
/// fresh UUID) if absent. Identity is a UUID, never the path — the folder can be
/// renamed or moved and the session history still belongs to the same project.
fn resolve_or_create_project(root: &Path) -> Result<String> {
    let dir = root.join(".glave");
    let marker = dir.join("project.json");
    // An EXISTING marker is authoritative: parse it strictly and error on a
    // malformed/unsafe id rather than silently minting a NEW project id (which
    // would orphan all prior session history under the old id).
    if marker.exists() {
        let raw = std::fs::read_to_string(&marker)
            .with_context(|| format!("reading {}", marker.display()))?;
        let value: serde_json::Value = serde_json::from_str(&raw)
            .with_context(|| format!("project marker {} is not valid JSON", marker.display()))?;
        let id = value
            .get("project_id")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| anyhow!("project marker {} has no project_id", marker.display()))?;
        if !is_safe_component(id) {
            return Err(anyhow!(
                "project marker {} has an unsafe project_id {id:?}",
                marker.display()
            ));
        }
        return Ok(id.to_string());
    }
    // No marker yet: mint one and write it ATOMICALLY (temp + rename) so a crash
    // mid-write can't leave a half-written marker that silently re-IDs later.
    let id = uuid::Uuid::new_v4().to_string();
    std::fs::create_dir_all(&dir).context("creating .glave")?;
    let body = serde_json::to_string_pretty(&serde_json::json!({ "project_id": id }))?;
    let tmp = dir.join("project.json.tmp");
    std::fs::write(&tmp, format!("{body}\n")).with_context(|| format!("writing {}", tmp.display()))?;
    std::fs::rename(&tmp, &marker).with_context(|| format!("finalizing {}", marker.display()))?;
    Ok(id)
}

/// A safe single path component: no separators, no `..`/`.`, not empty, not
/// absolute — keeps a project/session id from escaping its root dir.
fn is_safe_component(s: &str) -> bool {
    use std::path::Component;
    let mut comps = std::path::Path::new(s).components();
    matches!(comps.next(), Some(Component::Normal(_))) && comps.next().is_none() && !s.is_empty()
}

/// A model-driven [`Turn`]: calls the provider over the transcript and runs the
/// requested tools against the workspace, bounded by `max_steps`. The local
/// product trusts the workspace, so the policy is allow-all; the OS-sandbox rung
/// is a later hardening (see TODOS). Subagent/MCP are deferred for chat.
struct ModelTurn {
    provider: Box<dyn ModelProvider>,
    policy: Box<dyn Policy>,
    /// An interactive REPL can prompt the human on `Ask`; one-shot (`-p`) can't,
    /// so `Ask` becomes a refusal there.
    interactive: bool,
    max_steps: u32,
}

/// The local-product mediation policy. The repo is trusted (edits/reads/normal
/// shell are allowed; git is the undo), but clearly-destructive shell is gated
/// behind `Ask` so the REPL can confirm with the human first. INTERIM only — the
/// real boundary is the OS sandbox (deferred); a determined prompt-injection can
/// phrase around a heuristic, so this is not airtight.
struct ProductPolicy;

impl Policy for ProductPolicy {
    fn decide(&self, call: &ToolCall) -> Decision {
        match call {
            ToolCall::Bash { cmd } if is_destructive_bash(cmd) => {
                Decision::Ask(format!("potentially destructive: `{cmd}`"))
            }
            _ => Decision::Allow,
        }
    }
}

/// Heuristic denylist of clearly-destructive / irreversible shell. Conservative
/// on purpose (a false positive just prompts) — interim, not a security boundary.
fn is_destructive_bash(cmd: &str) -> bool {
    const DANGER: &[&str] = &[
        "rm -rf", "rm -fr", "rm -r ", "rmdir ", "mkfs", " dd ", "dd if=", ":(){",
        "shred ", "> /dev/sd", "sudo ", "chmod -r", "chown -r", "git push",
        "git reset --hard", "git clean -", "| sh", "|sh", "| bash", "|bash",
    ];
    let lower = cmd.to_lowercase();
    DANGER.iter().any(|p| lower.contains(p))
}

/// Ask the human (interactive REPL) to confirm a gated command. Anything but an
/// explicit yes is a no.
fn confirm(prompt: &str) -> bool {
    eprint!("\n⚠  {prompt}\n   run it? [y/N] ");
    let _ = std::io::stderr().flush();
    let mut answer = String::new();
    if std::io::stdin().read_line(&mut answer).is_err() {
        return false;
    }
    matches!(answer.trim(), "y" | "Y" | "yes" | "Yes")
}

impl Turn for ModelTurn {
    fn respond(
        &mut self,
        transcript: &[Message],
        workspace: &mut dyn Workspace,
    ) -> Result<Vec<Message>> {
        let mut convo = transcript.to_vec();
        let mut produced = Vec::new();
        // read-before-edit is enforced in tools::execute; seed it from the prior
        // transcript so a file read in an earlier turn still counts this turn.
        let mut viewed = seed_viewed(transcript);
        for _ in 0..self.max_steps {
            let completion = self.provider.complete(&convo)?;
            if completion.tool_calls.is_empty() {
                if completion.content.trim().is_empty() {
                    // Empty stopped turn: nudge rather than record a blank reply.
                    convo.push(Message::user(
                        "(empty response — call a tool, or give your answer)",
                    ));
                    continue;
                }
                produced.push(Message::assistant(completion.content));
                return Ok(produced);
            }
            let assistant = Message::assistant_with_tool_calls(
                completion.content.clone(),
                completion.tool_calls.clone(),
            );
            convo.push(assistant.clone());
            produced.push(assistant);
            let mut submitted = false;
            for call in &completion.tool_calls {
                let observation = match ToolCall::from_call(&call.name, &call.arguments) {
                    Ok(ToolCall::Submit) => {
                        submitted = true;
                        "(done)".to_string()
                    }
                    Ok(ToolCall::Subagent { .. } | ToolCall::Mcp { .. }) => {
                        "(subagent / mcp aren't available in interactive chat yet)".to_string()
                    }
                    Ok(parsed) => match self.policy.decide(&parsed) {
                        Decision::Allow => {
                            glave_core::tools::execute(&parsed, workspace, &mut viewed)
                                .unwrap_or_else(|e| format!("ERROR: {e}"))
                        }
                        Decision::Ask(reason) if self.interactive && confirm(&reason) => {
                            glave_core::tools::execute(&parsed, workspace, &mut viewed)
                                .unwrap_or_else(|e| format!("ERROR: {e}"))
                        }
                        Decision::Ask(reason) => format!("DECLINED ({reason}) — not run"),
                        Decision::Deny(reason) => format!("DENIED: {reason}"),
                    },
                    Err(e) => format!("ERROR: invalid arguments: {e}"),
                };
                let result = Message::tool_result(call.id.clone(), observation);
                convo.push(result.clone());
                produced.push(result);
            }
            // Every tool_call was answered (the API requires it) and the model
            // called submit — the turn is complete.
            if submitted {
                return Ok(produced);
            }
        }
        // Ran out of steps mid-work — signal it instead of truncating silently.
        produced.push(Message::assistant(format!(
            "(reached the {}-step limit for this turn — ask me to continue)",
            self.max_steps
        )));
        Ok(produced)
    }
}

/// Seed the read-before-edit set from a session's prior transcript, so a file
/// read in an earlier turn still counts as "read" this turn (tools::execute
/// requires a read before an edit). Scans assistant `read` tool calls.
fn seed_viewed(transcript: &[Message]) -> std::collections::BTreeSet<String> {
    let mut viewed = std::collections::BTreeSet::new();
    for message in transcript {
        for call in &message.tool_calls {
            if call.name == "read" {
                if let Ok(value) = serde_json::from_str::<serde_json::Value>(&call.arguments) {
                    if let Some(path) = value.get("path").and_then(serde_json::Value::as_str) {
                        viewed.insert(path.to_string());
                    }
                }
            }
        }
    }
    viewed
}

/// Print the assistant's user-visible text from a turn's messages (tool traffic
/// stays in the session log).
fn print_replies(replies: &[Message]) {
    for m in replies {
        if m.role == Role::Assistant && !m.content.trim().is_empty() {
            println!("{}", m.content.trim());
        }
    }
}

/// Bare `glave` (and `glave chat`): open the cwd as a Direct workspace, drive a
/// model turn over a persisted session, and loop.
fn chat(prompt: Option<String>, resume: Option<String>, model: &str) -> Result<()> {
    let cwd = std::env::current_dir().context("getting current dir")?;
    let project = resolve_or_create_project(&cwd)?;
    let store = SessionStore::new(SessionStore::default_root()?.join(&project));
    let mut workspace = open(&Plan::Direct { folder: cwd })?;

    let mut session = if let Some(id) = resume {
        store
            .load(&id)
            .with_context(|| format!("resuming session {id}"))?
    } else {
        let mut s = Session::new(uuid::Uuid::new_v4().to_string());
        s.push(Message::system(SYSTEM_PROMPT));
        store.save(&s)?;
        s
    };

    let provider = ChatProviderFactory::from_env().create(model)?;
    let interactive = prompt.is_none();
    let mut turn = ModelTurn {
        provider,
        policy: Box::new(ProductPolicy),
        interactive,
        max_steps: 50,
    };
    eprintln!(
        "glave · project {project} · session {} · {model}",
        session.id()
    );

    if let Some(p) = prompt {
        let replies = step(&mut session, &store, workspace.as_mut(), &mut turn, &p)?;
        print_replies(&replies);
        return Ok(());
    }

    eprintln!("interactive — type a request, or /exit to quit.");
    let stdin = std::io::stdin();
    loop {
        eprint!("\n› ");
        let _ = std::io::stderr().flush();
        let mut line = String::new();
        if stdin.read_line(&mut line).context("reading input")? == 0 {
            break; // EOF (Ctrl-D)
        }
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if line == "/exit" || line == "/quit" {
            break;
        }
        match step(&mut session, &store, workspace.as_mut(), &mut turn, line) {
            Ok(replies) => print_replies(&replies),
            Err(e) => eprintln!("error: {e}"),
        }
    }
    Ok(())
}
