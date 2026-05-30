//! The task-spec contract — the most important schema in the system.
//!
//! A task lives in a directory:
//!   task.toml      the spec (this module)
//!   workspace/     the code the agent may edit   (mounted read-write)
//!   oracle/        the tests that judge it        (mounted read-only)
//!   solution/      reference fix — for the stub agent + validation ONLY.

use anyhow::{Context, Result};
use serde::Deserialize;
use std::path::{Path, PathBuf};

#[derive(Debug, Deserialize)]
#[allow(dead_code)] // data contract: fields consumed across later phases
pub struct TaskSpec {
    pub id: String,
    pub language: String,
    pub archetype: String,
    pub difficulty: String,

    /// Globs (relative to workspace/) the agent is permitted to modify.
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
#[allow(dead_code)] // provenance/calibration consumed by the eval scorer later
pub struct Provenance {
    #[serde(default)]
    pub source: String,
    #[serde(default)]
    pub contamination: String,
    #[serde(default)]
    pub calibration: bool,
}

#[derive(Debug, Deserialize)]
pub struct Image {
    pub base: String,
    #[serde(default = "default_workdir")]
    pub workdir: String,
}

fn default_workdir() -> String {
    "/work".to_string()
}

#[derive(Debug, Deserialize)]
pub struct Oracle {
    /// Base test command run in the container; selected node ids are appended.
    pub command: String,
    pub fail_to_pass: Vec<String>,
    #[serde(default)]
    pub pass_to_pass: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)] // measured_peak_mb consumed by the parallel scheduler later
pub struct Resource {
    #[serde(default = "default_mem")]
    pub mem_mb: u64,
    #[serde(default = "default_cpus")]
    pub cpus: f64,
    #[serde(default = "default_pids")]
    pub pids: u64,
    #[serde(default = "default_network")]
    pub network: String,
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
    /// Parse a spec from TOML text (no directory context).
    pub fn from_toml(s: &str) -> Result<Self> {
        toml::from_str(s).context("parsing task toml")
    }

    /// Load a spec from a task directory.
    pub fn load(dir: &Path) -> Result<Self> {
        let toml_path = dir.join("task.toml");
        let raw = std::fs::read_to_string(&toml_path)
            .with_context(|| format!("reading {}", toml_path.display()))?;
        let mut spec = Self::from_toml(&raw)
            .with_context(|| format!("parsing {}", toml_path.display()))?;
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
