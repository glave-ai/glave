//! The performance budgets in AGENTS.md, measured on the release build: the binary's size,
//! cold start to the prompt, recording turns, and resuming a 10,000-event thread from SQLite.
//! Fails when any measurement is over its budget. Run it through `cargo xtask bench`, alone
//! on an idle machine, since these are wall-clock numbers.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use anyhow::{Context as _, bail};
use glave_computer::LocalComputers;
use glave_event::{Arguments, ModelRef, Part, StopReason, TokenCounts, Usage};
use glave_harness::{AllowAll, Cancel, Harness, Parts, Settings, Workspace};
use glave_keys::{KeyFileRoot, KeySettings};
use glave_ports::testing::ScriptedModel;
use glave_ports::{Completion, OsEntropy, RequestedCall, SystemClock};
use glave_store_sqlite::{SqliteBlobs, SqliteLog, Store};

/// Events in the resumed thread.
const EVENTS: usize = 10_000;
/// Each scripted turn records 8 events: turn.started, the person's message, the call, its
/// response, the result, the answer, its response, turn.ended.
const EVENTS_PER_TURN: usize = 8;
const STARTS: usize = 30;
const RESUMES: usize = 10;
/// The release binary's budget in MB (AGENTS.md), the one place it is set: pull requests,
/// the merge queue and `main` all run this benchmark (`budgets.yml`).
const BINARY_MB: f64 = 40.0;

fn main() -> anyhow::Result<()> {
    let dir = std::env::temp_dir().join(format!("glave-budgets-{}", std::process::id()));
    std::fs::create_dir_all(&dir)?;
    let result = measure(&dir);
    let _ = std::fs::remove_dir_all(&dir);
    result
}

fn measure(dir: &Path) -> anyhow::Result<()> {
    let binary = PathBuf::from(env!("CARGO_BIN_EXE_glave"));
    let size = std::fs::metadata(&binary)?.len();
    let start = median(cold_starts(&binary, dir)?);
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    let (recorded, resume) = runtime.block_on(threads(dir))?;
    let binary_budget = format!("< {BINARY_MB} MB");
    let rows = [
        (
            "release binary",
            binary_budget.as_str(),
            format!("{:.1} MB", mb(size)),
            mb(size) < BINARY_MB,
        ),
        (
            "cold start to prompt",
            "< 50 ms",
            format!("{:.1} ms (median of {STARTS})", ms(start)),
            ms(start) < 50.0,
        ),
        (
            "resume 10,000 events",
            "< 100 ms",
            format!("{:.1} ms (median of {RESUMES})", ms(resume)),
            ms(resume) < 100.0,
        ),
        (
            "record a turn (8 events)",
            "-",
            format!(
                "{:.2} ms (all {} turns)",
                ms(recorded / u32::try_from(turns())?),
                turns()
            ),
            true,
        ),
    ];
    println!("\n{:<26} {:<10} measured", "budget", "target");
    for (name, target, measured, within) in &rows {
        let mark = if *within { "" } else { "  OVER BUDGET" };
        println!("{name:<26} {target:<10} {measured}{mark}");
    }
    if rows.iter().any(|(.., within)| !within) {
        bail!("a performance budget was missed");
    }
    Ok(())
}

/// Times `glave` from launch until it has shown the prompt and seen the end of input.
fn cold_starts(binary: &Path, dir: &Path) -> anyhow::Result<Vec<Duration>> {
    let project = dir.join("project");
    std::fs::create_dir_all(&project)?;
    let mut times = Vec::with_capacity(STARTS);
    for _ in 0..STARTS {
        let started = Instant::now();
        let status = Command::new(binary)
            .current_dir(&project)
            .env("GLAVE_HOME", dir.join("home"))
            .env("GLAVE_BASE_URL", "http://127.0.0.1:9/v1")
            .env("GLAVE_MODEL", "m")
            .env_remove("GLAVE_API_KEY")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .context("could not start glave")?;
        times.push(started.elapsed());
        if !status.success() {
            bail!("glave exited with {status}");
        }
    }
    Ok(times)
}

fn turns() -> usize {
    EVENTS.saturating_sub(1) / EVENTS_PER_TURN
}

/// Records a 10,000-event thread through the harness, then resumes it from a freshly opened
/// store, as `glave --continue` does: how long recording took, and the median resume.
async fn threads(dir: &Path) -> anyhow::Result<(Duration, Duration)> {
    let project = dir.join("thread");
    std::fs::create_dir_all(&project)?;
    std::fs::write(project.join("README.md"), "A line of notes.\n".repeat(80))?;
    let database = dir.join("bench.db");
    let keys = || -> anyhow::Result<KeySettings> {
        let root = KeyFileRoot::open_or_create(database.with_extension("key"))?;
        Ok(KeySettings::new(std::sync::Arc::new(root)))
    };
    let store = Store::open(&database, keys()?).await?;
    let scope = glave_ports::Scope {
        org: store.org(uuid(1)).await?,
        project: glave_ports::ProjectId::from_uuid(uuid(2)),
    };
    let harness = harness_on(&store, &project).await?;
    let mut thread = harness.start_thread(scope).await?;
    let started = Instant::now();
    for _ in 0..turns() {
        harness
            .run_turn(&mut thread, "read the notes", &Cancel::new(), &mut |_| {})
            .await?;
    }
    let recorded = started.elapsed();
    let mut resumes = Vec::with_capacity(RESUMES);
    for _ in 0..RESUMES {
        let started = Instant::now();
        let store = Store::open(&database, keys()?).await?;
        let fresh = harness_on(&store, &project).await?;
        let again = fresh.resume_thread(scope, thread.id).await?;
        resumes.push(started.elapsed());
        if again.messages().len() != thread.messages().len() {
            bail!("the resumed thread differs from the recorded one");
        }
    }
    Ok((recorded, median(resumes)))
}

type Benched = Harness<ScriptedModel, SqliteLog, SqliteBlobs, AllowAll>;

async fn harness_on(store: &Store, project: &Path) -> anyhow::Result<Benched> {
    let mut settings = Settings::new("bench-model", "bench");
    // A thread this long would be compacted; the benchmark measures the full log instead.
    settings.compaction.window_tokens = u64::MAX / 100;
    let parts = Parts {
        provider: scripted()?,
        log: store.log(),
        blobs: store.blobs(),
        approver: AllowAll,
        clock: Box::new(SystemClock),
        entropy: Box::new(OsEntropy),
        workspace: {
            let (computers, info) = LocalComputers::here(project, std::env::vars_os()).await?;
            Workspace::new(computers, info)
        },
    };
    Ok(Harness::new(parts, settings))
}

/// A model that reads the notes, then answers, turn after turn.
fn scripted() -> anyhow::Result<ScriptedModel> {
    let usage = Usage::new(
        TokenCounts {
            input: 2_000,
            cache_read: 1_500,
            cache_write: 0,
            output: 60,
            reasoning: 0,
        },
        false,
        None,
    )?;
    let completion = |parts, tool_calls, stop| Completion {
        model: ModelRef {
            provider: "bench".into(),
            name: "bench-model".into(),
        },
        response_id: None,
        parts,
        tool_calls,
        stop,
        provider_stop: None,
        usage: usage.clone(),
        cost: None,
        served: None,
    };
    let mut arguments = serde_json::Map::new();
    arguments.insert("path".into(), "README.md".into());
    let read = RequestedCall {
        provider_call_id: Some("call_read".into()),
        tool: "read".into(),
        arguments: Arguments::Json(arguments),
    };
    let said = Part::Text {
        text: "The notes repeat one line, eighty times over.".into(),
    };
    Ok(ScriptedModel::cycling(vec![
        completion(Vec::new(), vec![read], StopReason::ToolUse),
        completion(vec![said], Vec::new(), StopReason::EndTurn),
    ])
    .unrecorded())
}

fn uuid(n: u8) -> uuid::Uuid {
    uuid::Uuid::from_bytes([n; 16])
}

fn median(mut times: Vec<Duration>) -> Duration {
    times.sort();
    times.get(times.len() / 2).copied().unwrap_or_default()
}

fn ms(duration: Duration) -> f64 {
    duration.as_secs_f64() * 1_000.0
}

#[expect(
    clippy::cast_precision_loss,
    reason = "a size in megabytes, for reading"
)]
fn mb(bytes: u64) -> f64 {
    bytes as f64 / 1_048_576.0
}
