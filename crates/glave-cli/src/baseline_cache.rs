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
//! Escape hatches: `--refresh-baseline`, or delete `cache/baselines/`.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::hash::{Hash, Hasher};
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

/// Env knobs that shape a run, captured by value (unset hashes differently from
/// set-to-default on purpose — cheaper than duplicating default logic here).
const RUN_SHAPING_ENV: [&str; 6] = [
    "GLAVE_REASONING",
    "GLAVE_PROVIDER",
    "GLAVE_QUANT",
    "GLAVE_TOKEN_BUDGET",
    "GLAVE_MAX_TURNS",
    "GLAVE_MAX_TOKENS",
];

/// One paid-for baseline trial: did it solve, and what did it cost.
#[derive(Serialize, Deserialize, Clone, Copy)]
pub struct Trial {
    pub solved: bool,
    pub cost: f64,
}

#[derive(Serialize, Deserialize)]
pub struct CachedBaseline {
    pub model: String,
    pub task_id: String,
    /// Unix seconds when first written — shown so a stale reference is visible.
    pub created_unix: u64,
    pub trials: Vec<Trial>,
}

pub struct BaselineCache {
    dir: PathBuf,
}

impl BaselineCache {
    #[must_use]
    pub fn new(dir: PathBuf) -> Self {
        Self { dir }
    }

    /// Cached trials for this (model, task, config, code) — `None` if never paid.
    pub fn lookup(&self, model: &str, task_dir: &Path) -> Option<CachedBaseline> {
        let bytes = std::fs::read(self.path(model, task_dir)).ok()?;
        serde_json::from_slice(&bytes).ok()
    }

    /// Persist trials (overwrites; callers append to what `lookup` returned).
    pub fn store(
        &self,
        model: &str,
        task_dir: &Path,
        task_id: &str,
        trials: Vec<Trial>,
        created_unix: u64,
    ) -> Result<()> {
        std::fs::create_dir_all(&self.dir)?;
        let entry = CachedBaseline {
            model: model.to_string(),
            task_id: task_id.to_string(),
            created_unix,
            trials,
        };
        let path = self.path(model, task_dir);
        std::fs::write(&path, serde_json::to_vec_pretty(&entry)?)
            .with_context(|| format!("writing baseline cache {}", path.display()))?;
        Ok(())
    }

    fn path(&self, model: &str, task_dir: &Path) -> PathBuf {
        let slug: String = model
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
            .collect();
        self.dir.join(format!("{slug}-{:016x}.json", key(model, task_dir)))
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
    std::fs::read(path).unwrap_or_default().hash(h);
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
    fn roundtrip_and_task_sensitivity() -> Result<()> {
        let base = std::env::temp_dir().join(format!("glave-bc-{}", std::process::id()));
        let task = task_fixture(&base)?;
        let cache = BaselineCache::new(base.join("cache"));

        assert!(cache.lookup("m/x", &task).is_none());
        cache.store("m/x", &task, "t", vec![Trial { solved: true, cost: 0.5 }], 1)?;
        let hit = cache
            .lookup("m/x", &task)
            .ok_or_else(|| anyhow::anyhow!("expected a cached entry"))?;
        assert_eq!(hit.trials.len(), 1);
        assert!(hit.trials.first().is_some_and(|t| t.solved));

        // A different model misses; a changed oracle (the ruler!) misses.
        assert!(cache.lookup("m/y", &task).is_none());
        std::fs::write(task.join("oracle/f2p.txt"), "tests/a.py::t2\n")?;
        assert!(cache.lookup("m/x", &task).is_none());

        let _ = std::fs::remove_dir_all(&base);
        Ok(())
    }
}
