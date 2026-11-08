//! The performance budgets in AGENTS.md, measured on the release build: the binary's size,
//! a command answered by the person's server while it runs, recording turns, and resuming a
//! 10,000-event thread from SQLite: one never compacted, read whole, and one compacted as its
//! window fills, read from its end.
//! Fails when any measurement is over its budget. Run it through `cargo xtask bench`, alone
//! on an idle machine, since these are wall-clock numbers.
//!
//! The size and the answered command are measured on the glave program `GLAVE_BUDGETS_BINARY`
//! names when it is set: in CI, glave as the release builds it for x86-64 Linux
//! (`budgets.yml`). Otherwise they are measured on the glave in this benchmark's own build
//! folder, which `cargo xtask bench` builds first in the same profile: close to what ships,
//! but without the web app inside. The threads are recorded and resumed in this program, on
//! glave's own allocator.
//!
//! `budgets record <folder>` and `budgets resume <folder>` count a resume's instructions, which
//! unlike its milliseconds don't depend on the CPU (#1286): the first records the 10,000-event
//! thread there at full speed, and the second, run under Valgrind, only resumes it, so what
//! Valgrind counts is the resume (`budgets.yml`).
//!
//! It lives in a package with no program of its own, so building it links no second glave.

#![expect(clippy::print_stdout, reason = "a benchmark: its table is its output")]

#[cfg(target_env = "musl")]
#[path = "../../cli/src/allocator.rs"]
mod allocator;

use std::io::{BufRead as _, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::time::{Duration, Instant};

use anyhow::{Context as _, bail};
use glave_computer::LocalComputers;
use glave_event::{Arguments, ModelRef, Part, StopReason, ThreadId, TokenCounts, Usage};
use glave_harness::{AllowAll, Cancel, Harness, Parts, Settings, Thread, Workspace};
use glave_keys::{KeyFileRoot, KeySettings};
use glave_ports::testing::ScriptedModel;
use glave_ports::{Completion, OsEntropy, RequestedCall, Scope, SystemClock};
use glave_store_sqlite::{SqliteBlobs, SqliteLog, Store};
use tokio::runtime::Runtime;

/// Events in the resumed thread.
const EVENTS: usize = 10_000;
/// Each scripted turn records 8 events: turn.started, the person's message, the call, its
/// response, the result, the answer, its response, turn.ended.
const EVENTS_PER_TURN: usize = 8;
/// Turns between one compaction and the next in the compacted thread: about what a
/// 128,000-token window holds of these turns (each reads 1.4 KB of notes) before it compacts at
/// 80%.
const TURNS_PER_COMPACTION: usize = 250;
/// Commands timed against the running server.
const ANSWERS: usize = 30;
const RESUMES: usize = 10;
/// The release binary's budget in MB (AGENTS.md), the one place it is set: the merge queue,
/// `main` and a weekly run all run this benchmark (`budgets.yml`).
const BINARY_MB: f64 = 50.0;
/// The environment variable naming the glave program to measure: in CI, the build that ships.
const BINARY: &str = "GLAVE_BUDGETS_BINARY";
/// How long glave has to start serving, to answer a command, and to stop once told: guards
/// against a hang, not speed limits.
const STARTS_WITHIN: Duration = Duration::from_secs(20);
const ANSWERS_WITHIN: Duration = Duration::from_secs(20);
const STOPS_WITHIN: Duration = Duration::from_secs(30);
/// The file, in a folder `budgets record` filled, naming the thread it recorded.
const THREAD_ID: &str = "thread-id";

fn main() -> anyhow::Result<()> {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    let arguments: Vec<&str> = arguments.iter().map(String::as_str).collect();
    match arguments.as_slice() {
        ["record", folder] => runtime()?.block_on(record_to_count(Path::new(folder))),
        ["resume", folder] => runtime()?.block_on(resume_to_count(Path::new(folder))),
        // The budgets: with no arguments, or the `--bench` that `cargo bench` passes.
        _ => {
            let dir = std::env::temp_dir().join(format!("glave-budgets-{}", std::process::id()));
            std::fs::create_dir_all(&dir)?;
            let result = measure(&dir);
            let _ = std::fs::remove_dir_all(&dir);
            result
        }
    }
}

fn runtime() -> anyhow::Result<Runtime> {
    Ok(tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?)
}

fn measure(dir: &Path) -> anyhow::Result<()> {
    // Absolute, since the commands it times run in a folder of their own.
    let binary = std::path::absolute(match std::env::var_os(BINARY) {
        Some(named) => PathBuf::from(named),
        None => built_beside()?,
    })?;
    println!("measuring {}", binary.display());
    let size = std::fs::metadata(&binary)
        .with_context(|| {
            format!(
                "no glave program at {}: `cargo xtask bench` builds it, or {BINARY} names one",
                binary.display()
            )
        })?
        .len();
    let answer = median(answered(&binary, dir)?);
    let runtime = runtime()?;
    let (recorded, resume) = runtime.block_on(threads(dir, None))?;
    let (_, compacted) = runtime.block_on(threads(dir, Some(TURNS_PER_COMPACTION)))?;
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
            "resume it, compacted",
            "< 100 ms",
            format!("{:.1} ms (median of {RESUMES})", ms(compacted)),
            ms(compacted) < 100.0,
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

/// The glave in this benchmark's build folder: the benchmark runs from `<folder>/deps`.
fn built_beside() -> anyhow::Result<PathBuf> {
    let me = std::env::current_exe().context("could not find this benchmark's own program")?;
    let folder = me
        .parent()
        .and_then(Path::parent)
        .with_context(|| format!("{} is not in a build folder", me.display()))?;
    Ok(folder.join(format!("glave{}", std::env::consts::EXE_SUFFIX)))
}

/// Times a command the person's server answers while it runs (`glave tasks`, a client of its
/// socket), from launch to exit; the server is started first, and stopped after. Each wait is
/// bounded: a server that does not start, answer or stop fails the benchmark with what it
/// said, instead of holding it forever.
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
    // What it says is read to its end, so it never blocks writing, and kept for a failure.
    let reading = std::thread::spawn(move || {
        let mut lines = Vec::new();
        for line in BufReader::new(said).lines().map_while(Result::ok) {
            if line.starts_with("glave: serving you on ") {
                let _ = ready.send(());
            }
            lines.push(line);
        }
        lines
    });
    let timed = serving
        .recv_timeout(STARTS_WITHIN)
        .with_context(|| format!("glave serve did not start within {STARTS_WITHIN:?}"))
        .and_then(|()| tasks(binary, dir, &home));
    let stopped = timed.is_ok() && {
        let _ = Command::new(binary)
            .args(["server", "stop"])
            .env("GLAVE_HOME", &home)
            .stdout(Stdio::null())
            .status();
        ended(&mut server, STOPS_WITHIN)?.is_some()
    };
    if !stopped {
        let _ = server.kill();
        server.wait()?;
    }
    let said = reading
        .join()
        .map_err(|_| anyhow::anyhow!("glave serve's output could not be read"))?
        .join("\n");
    let answers = timed.with_context(|| format!("glave serve said:\n{said}"))?;
    if !stopped {
        bail!(
            "glave serve did not stop within {STOPS_WITHIN:?} of being told to; it said:\n{said}"
        );
    }
    Ok(answers)
}

/// Times `glave tasks` against the server running for `home`, `ANSWERS` times.
fn tasks(binary: &Path, dir: &Path, home: &Path) -> anyhow::Result<Vec<Duration>> {
    let mut times = Vec::with_capacity(ANSWERS);
    for _ in 0..ANSWERS {
        let started = Instant::now();
        let mut glave = Command::new(binary)
            .arg("tasks")
            .current_dir(dir)
            .env("GLAVE_HOME", home)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .context("could not start glave")?;
        let Some(status) = ended(&mut glave, ANSWERS_WITHIN)? else {
            let _ = glave.kill();
            glave.wait()?;
            bail!("glave tasks did not answer within {ANSWERS_WITHIN:?}");
        };
        times.push(started.elapsed());
        if !status.success() {
            bail!("glave tasks exited with {status}");
        }
    }
    Ok(times)
}

/// How `child` exited, or none if it still runs after `within`. It is looked at every tenth
/// of a millisecond, finer than anything measured with it.
fn ended(child: &mut Child, within: Duration) -> anyhow::Result<Option<ExitStatus>> {
    let started = Instant::now();
    loop {
        if let Some(status) = child.try_wait()? {
            return Ok(Some(status));
        }
        if started.elapsed() > within {
            return Ok(None);
        }
        std::thread::sleep(Duration::from_micros(100));
    }
}

fn turns() -> usize {
    EVENTS.saturating_sub(1) / EVENTS_PER_TURN
}

/// Records a 10,000-event thread through the harness, compacted every `compacting` turns
/// when that is given, then resumes it from a freshly opened store, as `glave --continue` does:
/// how long recording took, and the median resume.
async fn threads(dir: &Path, compacting: Option<usize>) -> anyhow::Result<(Duration, Duration)> {
    let dir = dir.join(if compacting.is_some() {
        "compacted"
    } else {
        "whole"
    });
    let files = Files::at(&dir, compacting).await?;
    let (store, harness) = files.open().await?;
    let scope = scope_on(&store).await?;
    let (thread, recorded) = record(&harness, scope, compacting).await?;
    let mut resumes = Vec::with_capacity(RESUMES);
    for _ in 0..RESUMES {
        let started = Instant::now();
        let (_store, fresh) = files.open().await?;
        let again = fresh.resume_thread(scope, thread.id).await?;
        resumes.push(started.elapsed());
        if again.messages().len() != thread.messages().len() {
            bail!("the resumed thread differs from the recorded one");
        }
    }
    Ok((recorded, median(resumes)))
}

/// `budgets record <folder>`: the thread `budgets resume` resumes.
async fn record_to_count(folder: &Path) -> anyhow::Result<()> {
    let files = Files::at(folder, None).await?;
    let (store, harness) = files.open().await?;
    let (thread, _) = record(&harness, scope_on(&store).await?, None).await?;
    std::fs::write(folder.join(THREAD_ID), thread.id.to_string())?;
    Ok(())
}

/// `budgets resume <folder>`: one resume of the thread `budgets record` left there, the store
/// opened afresh as the timed resumes open it.
async fn resume_to_count(folder: &Path) -> anyhow::Result<()> {
    let files = Files::at(folder, None).await?;
    let (store, harness) = files.open().await?;
    let id: uuid::Uuid = std::fs::read_to_string(folder.join(THREAD_ID))?.parse()?;
    let again = harness
        .resume_thread(scope_on(&store).await?, ThreadId::try_from(id)?)
        .await?;
    let messages = again.messages().len();
    if messages < turns() {
        bail!(
            "resumed only {messages} messages of a thread of {} turns",
            turns()
        );
    }
    Ok(())
}

/// Records the thread's turns, compacted every `compacting` turns when that is given: the
/// thread, and how long it took.
async fn record(
    harness: &Benched,
    scope: Scope,
    compacting: Option<usize>,
) -> anyhow::Result<(Thread, Duration)> {
    let mut thread = harness.start_thread(scope).await?;
    let started = Instant::now();
    for turn in 1..=turns() {
        harness
            .run_turn(&mut thread, "read the notes", &Cancel::new(), &mut |_| {})
            .await?;
        if compacting.is_some_and(|every| turn % every == 0) {
            harness
                .compact(&mut thread, &Cancel::new(), &mut |_| {})
                .await?;
        }
    }
    Ok((thread, started.elapsed()))
}

async fn scope_on(store: &Store) -> anyhow::Result<Scope> {
    Ok(Scope::glaves_own(
        store.org(uuid(1)).await?,
        glave_ports::ProjectId::from_uuid(uuid(2)),
    ))
}

/// Where a benchmark's thread is kept, and how it was recorded.
struct Files {
    project: PathBuf,
    database: PathBuf,
    bucket: glave_bucket::Bucket,
    compacting: Option<usize>,
}

impl Files {
    async fn at(dir: &Path, compacting: Option<usize>) -> anyhow::Result<Self> {
        let project = dir.join("thread");
        std::fs::create_dir_all(&project)?;
        std::fs::write(project.join("README.md"), "A line of notes.\n".repeat(80))?;
        Ok(Self {
            project,
            database: dir.join("bench.db"),
            bucket: glave_bucket::Bucket::folder(&dir.join("blobs")).await?,
            compacting,
        })
    }

    /// A freshly opened store, and a harness on it.
    async fn open(&self) -> anyhow::Result<(Store, Benched)> {
        let root = KeyFileRoot::open_or_create(self.database.with_extension("key"))?;
        let keys = KeySettings::new(std::sync::Arc::new(root));
        let bucket = std::sync::Arc::new(self.bucket.clone());
        let store = Store::open(&self.database, keys, bucket).await?;
        let harness = harness_on(&store, &self.project, self.compacting).await?;
        Ok((store, harness))
    }
}

type Benched = Harness<ScriptedModel, SqliteLog, SqliteBlobs, AllowAll>;

async fn harness_on(
    store: &Store,
    project: &Path,
    compacting: Option<usize>,
) -> anyhow::Result<Benched> {
    let mut settings = Settings::new("bench-model", "bench");
    // Compacted only when the benchmark says, never as the window fills.
    settings.compaction.window_tokens = u64::MAX / 100;
    let parts = Parts {
        provider: scripted(compacting)?,
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

/// A model that reads the notes, then answers, turn after turn, and summarizes the thread
/// after every `compacting` turns when that is given.
fn scripted(compacting: Option<usize>) -> anyhow::Result<ScriptedModel> {
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
    let turn = [
        completion(Vec::new(), vec![read], StopReason::ToolUse),
        completion(vec![said], Vec::new(), StopReason::EndTurn),
    ];
    let mut answers: Vec<Completion> = turn
        .iter()
        .cycle()
        .take(turn.len() * compacting.unwrap_or(1))
        .cloned()
        .collect();
    if compacting.is_some() {
        let summary = Part::Text {
            text: "We read the notes, which repeat one line, turn after turn.".into(),
        };
        answers.push(completion(vec![summary], Vec::new(), StopReason::EndTurn));
    }
    Ok(ScriptedModel::cycling(answers).unrecorded())
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
