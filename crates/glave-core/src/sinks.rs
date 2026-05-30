//! Default light TraceSink adapter: append-only JSONL on disk.

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
