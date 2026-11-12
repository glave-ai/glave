//! `recall`'s search of the sealed index (`specs/storage.md`, "Search index"): a project's rows
//! opened into memory by the first search after the store opens (cold), and a search after it
//! (warm), on a typical project and on one at the cap a load opens. Fails when a cold load at the
//! cap takes 500 ms or more. Run it through `cargo xtask bench`, alone on an idle machine, since
//! these are wall-clock numbers.

#![expect(clippy::print_stdout, reason = "a benchmark: its table is its output")]

use std::error::Error;
use std::fmt::Write as _;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use glave_event::{
    At, CallId, EventId, Header, Part, RawEvent, Seq, ThreadId, ToolCallCompleted, encode,
};
use glave_keys::{KeyFileRoot, KeySettings};
use glave_ports::search::MOST_LOADED;
use glave_ports::testing::TempDir;
use glave_ports::{EventLog as _, InMemoryObjects, OrgId, ProjectId, Scope};
use glave_store_sqlite::Store;
use uuid::{Builder, Uuid};

type Result<T> = std::result::Result<T, Box<dyn Error>>;

/// Cold loads timed for each project, each on the store opened again.
const RUNS: usize = 5;
/// Events appended to one thread at a time.
const THREAD: usize = 50;
/// The cold load at the cap, at most.
const BUDGET: Duration = Duration::from_millis(500);

/// A project to search: how many events, and how many distinct words each.
struct Project {
    name: &'static str,
    events: usize,
    words: usize,
}

fn main() -> Result<()> {
    let runtime = tokio::runtime::Builder::new_current_thread().build()?;
    // Each word's 8 bytes sealed, and a row's 40: the cap's worth of rows of 1,000 words.
    let at_cap = usize::try_from(MOST_LOADED / 8_040)?;
    let projects = [
        Project {
            name: "a thousand messages",
            events: 1_000,
            words: 60,
        },
        Project {
            name: "at the cap",
            events: at_cap,
            words: 1_000,
        },
    ];
    println!("\nrecall's sealed index, cold is the first search after opening");
    println!(
        "{:<22} {:>8} {:>12} {:>12} {:>10}",
        "project", "events", "cold", "warm", "cache"
    );
    let mut over = false;
    for project in &projects {
        let dir = TempDir::new("bench-recall")?;
        let path = dir.join("glave.db");
        runtime.block_on(fill(&path, project))?;
        let (cold, warm, bytes) = runtime.block_on(measure(&path))?;
        println!(
            "{:<22} {:>8} {:>9.1} ms {:>9.2} ms {:>7.1} MB",
            project.name,
            project.events,
            ms(cold),
            ms(warm),
            mb(bytes)
        );
        over |= project.events == at_cap && cold >= BUDGET;
    }
    println!(
        "(budget: a cold load at the cap < {} ms)",
        BUDGET.as_millis()
    );
    if over {
        return Err("a cold load at the cap took longer than its budget".into());
    }
    Ok(())
}

fn settings(path: &Path) -> Result<KeySettings> {
    let root = KeyFileRoot::open_or_create(path.with_extension("key"))?;
    Ok(KeySettings::new(Arc::new(root)))
}

async fn open(path: &Path) -> Result<Store> {
    Ok(Store::open(path, settings(path)?, Arc::new(InMemoryObjects::new())).await?)
}

fn scope() -> Scope {
    Scope::glaves_own(
        OrgId::from_uuid(Uuid::from_u128(1)),
        ProjectId::from_uuid(Uuid::from_u128(2)),
    )
}

/// `project`'s events, in threads of [`THREAD`], each a tool's output of its own words.
async fn fill(path: &Path, project: &Project) -> Result<()> {
    let store = open(path).await?;
    let log = store.log();
    let mut next = 0x9e37_79b9_u64;
    for (n, start) in (0..project.events).step_by(THREAD).enumerate() {
        let n = u64::try_from(n)?;
        let thread = ThreadId::try_from(id(n, 0))?;
        let mut events = Vec::new();
        for at in start..(start + THREAD).min(project.events) {
            let seq = u64::try_from(at - start + 1)?;
            events.push(output((n, thread), seq, &text(&mut next, project.words))?);
        }
        log.append(&scope(), &events).await?;
    }
    Ok(())
}

/// The median cold load, a warm search, and the cache's size after them.
async fn measure(path: &Path) -> Result<(Duration, Duration, usize)> {
    let mut colds = Vec::new();
    let mut warm = Duration::ZERO;
    let mut bytes = 0;
    for _ in 0..RUNS {
        let store = open(path).await?;
        let log = store.log();
        let started = Instant::now();
        log.search(&scope(), "w17 w4242 w90001", 10).await?;
        colds.push(started.elapsed());
        let started = Instant::now();
        log.search(&scope(), "w17 w31337", 10).await?;
        warm = started.elapsed();
        bytes = store.search_cache_bytes();
    }
    colds.sort();
    Ok((
        colds.get(RUNS / 2).copied().unwrap_or_default(),
        warm,
        bytes,
    ))
}

/// `words` words drawn from a vocabulary of 200,000, as a tool's output might hold.
fn text(next: &mut u64, words: usize) -> String {
    let mut text = String::new();
    for _ in 0..words {
        *next = next.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
        let _ = write!(text, "w{} ", (*next >> 33) % 200_000);
    }
    text
}

/// A tool's output of `text` at `seq` in `thread`, the `n`th.
fn output((n, thread): (u64, ThreadId), seq: u64, text: &str) -> Result<RawEvent> {
    let header = Header {
        id: EventId::try_from(id(n, seq))?,
        thread,
        seq: Seq::new(seq)?,
        at: At::try_from("2026-11-11T09:00:00.000Z".to_owned())?,
    };
    let done = ToolCallCompleted {
        call_id: CallId::new(format!("call_{seq}"))?,
        is_error: false,
        output: vec![Part::Text { text: text.into() }],
        structured: None,
        untrusted: false,
    };
    Ok(encode(header, &done)?)
}

/// A version 7 id told apart by `high` and `low`.
fn id(high: u64, low: u64) -> Uuid {
    let mut random = [0u8; 10];
    let bytes = high
        .to_be_bytes()
        .into_iter()
        .skip(4)
        .chain(low.to_be_bytes().into_iter().skip(2));
    for (slot, byte) in random.iter_mut().zip(bytes) {
        *slot = byte;
    }
    Builder::from_unix_timestamp_millis(1_791_000_000_000, &random).into_uuid()
}

fn ms(duration: Duration) -> f64 {
    duration.as_secs_f64() * 1_000.0
}

#[expect(
    clippy::cast_precision_loss,
    reason = "a size in megabytes, for reading"
)]
fn mb(bytes: usize) -> f64 {
    bytes as f64 / 1_048_576.0
}
