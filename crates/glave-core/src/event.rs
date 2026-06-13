//! The trace event schema — a compounding asset. Run-id-correlated, append-only,
//! one event per line when written by a JSONL sink.

use serde::Serialize;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::oracle::Verdict;
use crate::ports::Completion;
use crate::task::TaskSpec;

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
        .unwrap_or(0)
}

#[derive(Debug, Clone, Serialize)]
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
    #[serde(rename = "model.turn")]
    ModelTurn {
        run_id: String,
        ts_ms: u64,
        turn: u32,
        model: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        provider: Option<String>,
        prompt_tokens: u64,
        completion_tokens: u64,
        reasoning_tokens: u64,
        cost_usd: f64,
        content: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        reasoning: Option<String>,
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

    pub fn model_turn(run_id: &str, turn: u32, c: &Completion) -> Self {
        RunEvent::ModelTurn {
            run_id: run_id.to_string(),
            ts_ms: now_ms(),
            turn,
            model: c.model.clone(),
            provider: c.provider.clone(),
            prompt_tokens: c.prompt_tokens,
            completion_tokens: c.completion_tokens,
            reasoning_tokens: c.reasoning_tokens,
            cost_usd: c.cost_usd,
            content: c.content.clone(),
            reasoning: c.reasoning.clone(),
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

    pub fn cost_usage_total(
        run_id: &str,
        model: &str,
        prompt_tokens: u64,
        completion_tokens: u64,
        cost_usd: f64,
    ) -> Self {
        RunEvent::CostUsage {
            run_id: run_id.to_string(),
            ts_ms: now_ms(),
            model: Some(model.to_string()),
            prompt_tokens,
            completion_tokens,
            cost_usd,
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
        .map_or_else(
            || "unknown".to_string(),
            |o| String::from_utf8_lossy(&o.stdout).trim().to_string(),
        )
}
