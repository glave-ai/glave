//! TraceSink adapters: append-only JSONL on disk, and an in-memory collector
//! used by the scorer to read back a run's verdict and cost.

use anyhow::{Context, Result};
use std::fs::{create_dir_all, File, OpenOptions};
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};

use crate::event::RunEvent;
use crate::ports::TraceSink;

pub struct JsonlSink {
    writer: BufWriter<File>,
    path: PathBuf,
}

impl JsonlSink {
    pub fn create(dir: &Path, run_id: &str) -> Result<Self> {
        create_dir_all(dir)?;
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

    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl TraceSink for JsonlSink {
    #[allow(clippy::needless_pass_by_value)] // the sink takes ownership of the event
    fn emit(&mut self, event: RunEvent) -> Result<()> {
        let line = serde_json::to_string(&event)?;
        self.writer.write_all(line.as_bytes())?;
        self.writer.write_all(b"\n")?;
        self.writer.flush()?;
        Ok(())
    }
}

/// Collects events in memory and reports a run's outcome + cost. Used by the
/// scorer (and by tests).
#[derive(Default)]
pub struct InMemorySink {
    pub events: Vec<RunEvent>,
}

impl InMemorySink {
    /// Whether the run's oracle reported solved.
    pub fn solved(&self) -> bool {
        self.events
            .iter()
            .rev()
            .find_map(|e| match e {
                RunEvent::OracleResult { solved, .. } => Some(*solved),
                _ => None,
            })
            .unwrap_or(false)
    }

    /// Total USD cost across the run (sum of cost.usage totals).
    pub fn cost_usd(&self) -> f64 {
        self.events
            .iter()
            .filter_map(|e| match e {
                RunEvent::CostUsage { cost_usd, .. } => Some(*cost_usd),
                _ => None,
            })
            .sum()
    }

    /// Total tokens across the run.
    pub fn tokens(&self) -> u64 {
        self.events
            .iter()
            .filter_map(|e| match e {
                RunEvent::CostUsage {
                    prompt_tokens,
                    completion_tokens,
                    ..
                } => Some(prompt_tokens + completion_tokens),
                _ => None,
            })
            .sum()
    }
}

impl TraceSink for InMemorySink {
    #[allow(clippy::needless_pass_by_value)]
    fn emit(&mut self, event: RunEvent) -> Result<()> {
        self.events.push(event);
        Ok(())
    }
}
