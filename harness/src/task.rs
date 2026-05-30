//! The task-spec contract — the most important schema in the system.
//!
//! A task lives in a directory:
//!   task.toml      the spec (this module)
//!   workspace/     the code the agent may edit   (mounted read-write)
//!   oracle/        the tests that judge it        (mounted read-only)
//!   solution/      reference fix — for the stub agent + task validation ONLY;
//!                  never exposed to a real model agent.

use anyhow::{Context, Result};
use serde::Deserialize;
use std::path::{Path, PathBuf};

#[derive(Debug, Deserialize)]
pub struct TaskSpec {
    pub id: String,
    pub language: String,
    pub archetype: String,
    pub difficulty: String,

    /// Globs (relative to workspace/) the agent is permitted to modify.
    /// Recorded in slice 1; hard-enforced once the real agent lands.
    #[serde(default)]
    pub editable_surface: Vec<String>,

    #[serde(default)]
    pub provenance: Provenance,
    pub image: Image,
    pub oracle: Oracle,
    #[serde(default)]
    pub resource: Resource,

    /// Filled in at load time; not part of the TOML.
    #[serde(skip)]
    pub dir: PathBuf,
}

#[derive(Debug, Deserialize, Default)]
pub struct Provenance {
    /// e.g. "authored", "real-slice:<repo>@<sha>"
    #[serde(default)]
    pub source: String,
    /// "none" | "low" | "known" — how exposed the task is to training data.
    #[serde(default)]
    pub contamination: String,
    /// Whether this task belongs to the external-calibration set.
    #[serde(default)]
    pub calibration: bool,
}

#[derive(Debug, Deserialize)]
pub struct Image {
    /// Docker image tag, e.g. "glave/python-base:latest".
    pub base: String,
    #[serde(default = "default_workdir")]
    pub workdir: String,
}

fn default_workdir() -> String {
    "/work".to_string()
}

#[derive(Debug, Deserialize)]
pub struct Oracle {
    /// Base test command run inside the container; selected node ids are appended.
    pub command: String,
    /// Tests that must go red -> green (the task is "solved" only if all pass).
    pub fail_to_pass: Vec<String>,
    /// Tests that must stay green (guards against "passed by breaking everything").
    #[serde(default)]
    pub pass_to_pass: Vec<String>,
}

#[derive(Debug, Deserialize)]
pub struct Resource {
    #[serde(default = "default_mem")]
    pub mem_mb: u64,
    #[serde(default = "default_cpus")]
    pub cpus: f64,
    #[serde(default = "default_pids")]
    pub pids: u64,
    #[serde(default = "default_network")]
    pub network: String,
    /// Measured peak RAM (filled in by baking later); informs the scheduler.
    #[serde(default)]
    pub measured_peak_mb: Option<u64>,
}

impl Default for Resource {
    fn default() -> Self {
        Self {
            mem_mb: default_mem(),
            cpus: default_cpus(),
            pids: default_pids(),
            network: default_network(),
            measured_peak_mb: None,
        }
    }
}

fn default_mem() -> u64 {
    512
}
fn default_cpus() -> f64 {
    1.0
}
fn default_pids() -> u64 {
    256
}
fn default_network() -> String {
    "none".to_string()
}

impl TaskSpec {
    pub fn load(dir: &Path) -> Result<Self> {
        let toml_path = dir.join("task.toml");
        let raw = std::fs::read_to_string(&toml_path)
            .with_context(|| format!("reading {}", toml_path.display()))?;
        let mut spec: TaskSpec =
            toml::from_str(&raw).with_context(|| format!("parsing {}", toml_path.display()))?;
        spec.dir = dir.to_path_buf();
        Ok(spec)
    }

    pub fn workspace_dir(&self) -> PathBuf {
        self.dir.join("workspace")
    }
    pub fn oracle_dir(&self) -> PathBuf {
        self.dir.join("oracle")
    }
    pub fn solution_dir(&self) -> PathBuf {
        self.dir.join("solution")
    }
}
