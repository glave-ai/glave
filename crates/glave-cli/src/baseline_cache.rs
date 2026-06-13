//! Cache for the expensive frontier-baseline leg of `glave score`.
//!
//! The baseline (e.g. Opus) is a *reference point*: its result on a fixed task
//! under a fixed harness doesn't change when we iterate on the cheap model. So
//! we pay for it once and reuse it, re-running only when something that shapes
//! the run actually changes. The key covers all of it:
//!
//! - the baseline model id,
//! - the task content (task.toml + oracle files — what the run is graded by),
//! - run-shaping env config (reasoning, provider pin, budgets),
//! - the run-shaping source code, embedded at compile time below.
//!
//! That last one auto-enforces fairness: swapping models or config reuses the
//! cache (cheap iteration), while any change to the agent loop / tools / policy
//! / prompts / request builder rebuilds the binary, moves the hash, and forces
//! a fresh armed-vs-armed baseline. Conservative by design — it may re-run on a
//! trivial code edit, never the reverse.
//!
//! **Format & concurrency:** each task's cache is an append-only JSONL file, one
//! `Trial` per line. `append` opens with `O_APPEND` and writes the batch in a
//! single `write_all`, which POSIX makes atomic — so two workers persisting
//! trials of the *same* baseline task (which the planned task×trial fan-out will
//! do) can't clobber each other. No read-modify-write, no lock file.
//!
//! Escape hatches: `--refresh-baseline` (clears the file first), or delete
//! `cache/baselines/`.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::hash::{Hash, Hasher};
use std::io::Write;
use std::path::{Path, PathBuf};

/// Run-shaping source, embedded at compile time — the binary's run behavior can
/// only change if these change (which rebuilds the binary and moves the hash).
/// Grow this list when a new file starts shaping runs.
const RUN_SHAPING_SRC: [&str; 5] = [
    include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/../glave-core/src/agent.rs")),
    include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/../glave-core/src/tools.rs")),
    include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/../glave-core/src/policy.rs")),
    include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/../glave-core/src/capability.rs")),
    include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/../glave-openrouter/src/lib.rs")),
];

/// Env knobs that shape a run, captured by value. Both token vars matter and are
/// distinct: `GLAVE_TOKEN_BUDGET` is the agent's total output-token budget;
/// `GLAVE_MAX_TOKENS` is the OpenRouter per-request output cap. Changing either
/// changes the run, so both belong in the key.
const RUN_SHAPING_ENV: [&str; 6] = [
    "GLAVE_REASONING",
    "GLAVE_PROVIDER",
    "GLAVE_QUANT",
    "GLAVE_TOKEN_BUDGET",
    "GLAVE_MAX_TOKENS",
    "GLAVE_MAX_TURNS",
];

/// One paid-for baseline trial: did it solve, and what did it cost.
#[derive(Serialize, Deserialize, Clone, Copy)]
pub struct Trial {
    pub solved: bool,
    pub cost: f64,
}

pub struct BaselineCache {
    dir: PathBuf,
}

impl BaselineCache {
    #[must_use]
    pub fn new(dir: PathBuf) -> Self {
        Self { dir }
    }

    /// Trials already paid for this (model, task, config, code), oldest first.
    /// A truncated/garbled line is skipped rather than failing the read — the
    /// worst case is re-running a trial, never a crash.
    pub fn lookup(&self, model: &str, task_dir: &Path) -> Vec<Trial> {
        let Ok(text) = std::fs::read_to_string(self.path(model, task_dir)) else {
            return Vec::new();
        };
        text.lines()
            .filter_map(|l| serde_json::from_str::<Trial>(l).ok())
            .collect()
    }

    /// Append freshly-run trials. Atomic under `O_APPEND` (one `write_all` of a
    /// small batch), so concurrent appenders for the same task don't interleave.
    pub fn append(&self, model: &str, task_dir: &Path, trials: &[Trial]) -> Result<()> {
        if trials.is_empty() {
            return Ok(());
        }
        std::fs::create_dir_all(&self.dir)?;
        let mut batch = String::new();
        for t in trials {
            batch.push_str(&serde_json::to_string(t)?);
            batch.push('\n');
        }
        let path = self.path(model, task_dir);
        let mut f = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .with_context(|| format!("opening baseline cache {}", path.display()))?;
        f.write_all(batch.as_bytes())
            .with_context(|| format!("appending baseline cache {}", path.display()))?;
        Ok(())
    }

    /// Discard a task's cached baseline (used by `--refresh-baseline`).
    pub fn clear(&self, model: &str, task_dir: &Path) {
        let _ = std::fs::remove_file(self.path(model, task_dir));
    }

    fn path(&self, model: &str, task_dir: &Path) -> PathBuf {
        let slug: String = model
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
            .collect();
        self.dir.join(format!("{slug}-{:016x}.jsonl", key(model, task_dir)))
    }
}

/// The cache key. SipHash via `DefaultHasher` — deterministic across runs of the
/// same toolchain; not guaranteed across Rust upgrades, which merely invalidates
/// the cache (the safe direction).
fn key(model: &str, task_dir: &Path) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    model.hash(&mut h);
    // Task content: the spec + everything the oracle grades by.
    file_hash(&mut h, &task_dir.join("task.toml"));
    if let Ok(entries) = std::fs::read_dir(task_dir.join("oracle")) {
        let mut paths: Vec<_> = entries.flatten().map(|e| e.path()).collect();
        paths.sort();
        for p in paths {
            file_hash(&mut h, &p);
        }
    }
    for var in RUN_SHAPING_ENV {
        var.hash(&mut h);
        std::env::var(var).unwrap_or_default().hash(&mut h);
    }
    for src in RUN_SHAPING_SRC {
        src.hash(&mut h);
    }
    h.finish()
}

fn file_hash(h: &mut impl Hasher, path: &Path) {
    path.file_name().hash(h);
    // Tag read success vs failure so an *unreadable* file can't hash identically
    // to an empty or missing one. Otherwise a silently-unreadable oracle file
    // would collide keys and could reuse a stale baseline; a distinct tag makes
    // it a cache miss instead — the safe direction (re-run, never wrongly reuse).
    match std::fs::read(path) {
        Ok(bytes) => {
            0u8.hash(h);
            bytes.hash(h);
        }
        Err(e) => {
            1u8.hash(h);
            format!("{:?}", e.kind()).hash(h);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn task_fixture(base: &Path) -> Result<PathBuf> {
        let dir = base.join("task");
        std::fs::create_dir_all(dir.join("oracle"))?;
        std::fs::write(dir.join("task.toml"), "id = \"t\"")?;
        std::fs::write(dir.join("oracle/f2p.txt"), "tests/a.py::t1\n")?;
        Ok(dir)
    }

    #[test]
    fn append_lookup_clear_and_task_sensitivity() -> Result<()> {
        let base = std::env::temp_dir().join(format!("glave-bc-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let task = task_fixture(&base)?;
        let cache = BaselineCache::new(base.join("cache"));

        assert!(cache.lookup("m/x", &task).is_empty());
        cache.append("m/x", &task, &[Trial { solved: true, cost: 0.5 }])?;
        // A second append accumulates (the top-up path) — no clobber.
        cache.append("m/x", &task, &[Trial { solved: false, cost: 0.3 }])?;
        let hit = cache.lookup("m/x", &task);
        assert_eq!(hit.len(), 2);
        assert!(hit.first().is_some_and(|t| t.solved));
        assert!(hit.get(1).is_some_and(|t| !t.solved));

        // A different model misses; a changed oracle (the ruler!) misses.
        assert!(cache.lookup("m/y", &task).is_empty());
        std::fs::write(task.join("oracle/f2p.txt"), "tests/a.py::t2\n")?;
        assert!(cache.lookup("m/x", &task).is_empty());

        // clear discards.
        std::fs::write(task.join("oracle/f2p.txt"), "tests/a.py::t1\n")?; // back to the cached key
        assert_eq!(cache.lookup("m/x", &task).len(), 2);
        cache.clear("m/x", &task);
        assert!(cache.lookup("m/x", &task).is_empty());

        let _ = std::fs::remove_dir_all(&base);
        Ok(())
    }
}
