//! Flat-file JSONL trace. Every event carries the run id so the two halves of a
//! run (this runner half now; the gateway/model half in slice 2) join cleanly.
//! Append-only, one event per line.

use anyhow::{Context, Result};
use serde::Serialize;
use std::fs::{create_dir_all, File, OpenOptions};
use std::io::{BufWriter, Write};
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::oracle::Verdict;
use crate::task::TaskSpec;

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

#[derive(Serialize)]
#[serde(tag = "event")]
pub enum RunEvent {
    #[serde(rename = "run.started")]
    RunStarted {
        run_id: String,
        ts_ms: u64,
        task_id: String,
        language: String,
        archetype: String,
        difficulty: String,
        agent: String,
        harness_sha: String,
    },
    #[serde(rename = "agent.action")]
    AgentAction {
        run_id: String,
        ts_ms: u64,
        action: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        path: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        detail: Option<String>,
    },
    #[serde(rename = "cost.usage")]
    CostUsage {
        run_id: String,
        ts_ms: u64,
        #[serde(skip_serializing_if = "Option::is_none")]
        model: Option<String>,
        prompt_tokens: u64,
        completion_tokens: u64,
        cost_usd: f64,
    },
    #[serde(rename = "oracle.result")]
    OracleResult {
        run_id: String,
        ts_ms: u64,
        fail_to_pass_ok: bool,
        pass_to_pass_ok: bool,
        solved: bool,
        f2p_exit: i32,
        #[serde(skip_serializing_if = "Option::is_none")]
        p2p_exit: Option<i32>,
    },
    #[serde(rename = "run.completed")]
    RunCompleted {
        run_id: String,
        ts_ms: u64,
        solved: bool,
        duration_ms: u64,
    },
}

impl RunEvent {
    pub fn run_started(run_id: &str, task: &TaskSpec, agent: &str) -> Self {
        RunEvent::RunStarted {
            run_id: run_id.to_string(),
            ts_ms: now_ms(),
            task_id: task.id.clone(),
            language: task.language.clone(),
            archetype: task.archetype.clone(),
            difficulty: task.difficulty.clone(),
            agent: agent.to_string(),
            harness_sha: harness_sha(),
        }
    }

    pub fn agent_action(
        run_id: &str,
        action: &str,
        path: Option<String>,
        detail: Option<String>,
    ) -> Self {
        RunEvent::AgentAction {
            run_id: run_id.to_string(),
            ts_ms: now_ms(),
            action: action.to_string(),
            path,
            detail,
        }
    }

    pub fn cost_usage_zero(run_id: &str) -> Self {
        RunEvent::CostUsage {
            run_id: run_id.to_string(),
            ts_ms: now_ms(),
            model: None,
            prompt_tokens: 0,
            completion_tokens: 0,
            cost_usd: 0.0,
        }
    }

    pub fn oracle_result(run_id: &str, v: &Verdict) -> Self {
        RunEvent::OracleResult {
            run_id: run_id.to_string(),
            ts_ms: now_ms(),
            fail_to_pass_ok: v.fail_to_pass_ok,
            pass_to_pass_ok: v.pass_to_pass_ok,
            solved: v.solved,
            f2p_exit: v.f2p_exit,
            p2p_exit: v.p2p_exit,
        }
    }

    pub fn run_completed(run_id: &str, solved: bool, duration_ms: u64) -> Self {
        RunEvent::RunCompleted {
            run_id: run_id.to_string(),
            ts_ms: now_ms(),
            solved,
            duration_ms,
        }
    }
}

fn harness_sha() -> String {
    std::process::Command::new("git")
        .args(["rev-parse", "--short", "HEAD"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_else(|| "unknown".to_string())
}

pub struct Trace {
    writer: BufWriter<File>,
    path: PathBuf,
}

impl Trace {
    pub fn new(run_id: &str) -> Result<Self> {
        let dir = PathBuf::from("traces");
        create_dir_all(&dir)?;
        let path = dir.join(format!("{run_id}.jsonl"));
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .with_context(|| format!("opening trace {}", path.display()))?;
        Ok(Self {
            writer: BufWriter::new(file),
            path,
        })
    }

    pub fn emit(&mut self, ev: RunEvent) -> Result<()> {
        let line = serde_json::to_string(&ev)?;
        self.writer.write_all(line.as_bytes())?;
        self.writer.write_all(b"\n")?;
        self.writer.flush()?;
        Ok(())
    }

    pub fn path(&self) -> &PathBuf {
        &self.path
    }
}
