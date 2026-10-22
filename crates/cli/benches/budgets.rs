//! The performance budgets in AGENTS.md, measured on the release build: the binary's size,
//! a command answered by the person's server while it runs, recording turns, and resuming a
//! 10,000-event thread from SQLite.
//! Fails when any measurement is over its budget. Run it through `cargo xtask bench`, alone
//! on an idle machine, since these are wall-clock numbers.
//!
//! The size and the answered command are measured on the program [`BINARY`] names when it is
//! set: in CI, glave as the release builds it for x86-64 Linux (`budgets.yml`). Otherwise they
//! are measured on the glave this benchmark's own build made, with what the tests' features
//! add to it, so close to what ships but not it.

#![expect(clippy::print_stdout, reason = "a benchmark: its table is its output")]

use std::io::{BufRead as _, BufReader};
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
/// Commands timed against the running server.
const ANSWERS: usize = 30;
const RESUMES: usize = 10;
/// The release binary's budget in MB (AGENTS.md), the one place it is set: pull requests,
/// the merge queue and `main` all run this benchmark (`budgets.yml`).
const BINARY_MB: f64 = 40.0;
/// The variable naming the glave program to measure, as it ships.
const BINARY: &str = "GLAVE_BUDGETS_BINARY";

fn main() -> anyhow::Result<()> {
    let dir = std::env::temp_dir().join(format!("glave-budgets-{}", std::process::id()));
    std::fs::create_dir_all(&dir)?;
    let result = measure(&dir);
    let _ = std::fs::remove_dir_all(&dir);
    result
}

fn measure(dir: &Path) -> anyhow::Result<()> {
    let binary = std::env::var_os(BINARY)
        .map_or_else(|| PathBuf::from(env!("CARGO_BIN_EXE_glave")), PathBuf::from);
    println!("measuring {}", binary.display());
    let size = std::fs::metadata(&binary)
        .with_context(|| format!("no glave program at {}", binary.display()))?
        .len();
    let answer = median(answered(&binary, dir)?);
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
            "answered by the server",
            "< 50 ms",
            format!("{:.1} ms (median of {ANSWERS})", ms(answer)),
            ms(answer) < 50.0,
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

/// Times a command the person's server answers while it runs (`glave tasks`, a client of its
/// socket), from launch to exit; the server is started first, and stopped after.
fn answered(binary: &Path, dir: &Path) -> anyhow::Result<Vec<Duration>> {
    let home = dir.join("home");
    std::fs::create_dir_all(&home)?;
    let mut server = Command::new(binary)
        .args(["serve", "--http", "127.0.0.1:0"])
        .current_dir(dir)
        .env("GLAVE_HOME", &home)
        .env("GLAVE_BASE_URL", "http://127.0.0.1:9/v1")
        .env("GLAVE_MODEL", "m")
        .env_remove("GLAVE_API_KEY")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .context("could not start glave serve")?;
    let said = server.stderr.take().context("glave serve's output")?;
    let (ready, serving) = std::sync::mpsc::channel();
    // What it says is read to its end, so it never blocks writing.
    let reading = std::thread::spawn(move || {
        for line in BufReader::new(said).lines().map_while(Result::ok) {
            if line.starts_with("glave: serving you on ") {
                let _ = ready.send(());
            }
        }
    });
    let timed = serving
        .recv_timeout(Duration::from_secs(20))
        .context("glave serve did not start")
        .and_then(|()| tasks(binary, dir, &home));
    let _ = Command::new(binary)
        .args(["server", "stop"])
        .env("GLAVE_HOME", &home)
        .stdout(Stdio::null())
        .status();
    let _ = server.wait();
    let _ = reading.join();
    timed
}

/// Times `glave tasks` against the server running for `home`, `ANSWERS` times.
fn tasks(binary: &Path, dir: &Path, home: &Path) -> anyhow::Result<Vec<Duration>> {
    let mut times = Vec::with_capacity(ANSWERS);
    for _ in 0..ANSWERS {
        let started = Instant::now();
        let status = Command::new(binary)
            .arg("tasks")
            .current_dir(dir)
            .env("GLAVE_HOME", home)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .context("could not start glave")?;
        times.push(started.elapsed());
        if !status.success() {
            bail!("glave tasks exited with {status}");
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
    let bucket = glave_bucket::Bucket::folder(&dir.join("blobs")).await?;
    let store = Store::open(&database, keys()?, std::sync::Arc::new(bucket.clone())).await?;
    let scope = glave_ports::Scope::glaves_own(
        store.org(uuid(1)).await?,
        glave_ports::ProjectId::from_uuid(uuid(2)),
    );
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
        let store = Store::open(&database, keys()?, std::sync::Arc::new(bucket.clone())).await?;
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
        upstream: None,
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
