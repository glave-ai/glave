//! In-memory adapters for deterministic, I/O-free tests. Gated behind the
//! `test-util` feature (and available in this crate's own tests).

use anyhow::{anyhow, Result};
use std::cell::RefCell;
use std::collections::{BTreeMap, VecDeque};

use crate::ports::{Completion, ExecResult, Message, ModelProvider, WorkspaceRuntime};

/// Returns scripted completions in order — deterministic, zero network, zero cost.
pub struct InMemoryProvider {
    scripted: RefCell<VecDeque<String>>,
    model: String,
}

impl InMemoryProvider {
    pub fn new(responses: Vec<&str>) -> Self {
        Self {
            scripted: RefCell::new(responses.into_iter().map(String::from).collect()),
            model: "inmemory".to_string(),
        }
    }
}

impl ModelProvider for InMemoryProvider {
    fn complete(&self, _messages: &[Message]) -> Result<Completion> {
        let content = self.scripted.borrow_mut().pop_front().unwrap_or_default();
        Ok(Completion {
            content,
            model: self.model.clone(),
            ..Completion::default()
        })
    }
    fn model_id(&self) -> &str {
        &self.model
    }
}

/// In-memory workspace: a map of files; `run()` returns a configurable exit code.
pub struct InMemoryRuntime {
    files: BTreeMap<String, String>,
    run_exit: i32,
}

impl InMemoryRuntime {
    pub fn new() -> Self {
        Self {
            files: BTreeMap::new(),
            run_exit: 0,
        }
    }
    #[must_use]
    pub fn with_file(mut self, rel: &str, contents: &str) -> Self {
        self.files.insert(rel.to_string(), contents.to_string());
        self
    }
    pub fn file(&self, rel: &str) -> Option<&String> {
        self.files.get(rel)
    }
    pub fn set_run_exit(&mut self, code: i32) {
        self.run_exit = code;
    }
}

impl Default for InMemoryRuntime {
    fn default() -> Self {
        Self::new()
    }
}

impl WorkspaceRuntime for InMemoryRuntime {
    fn read_file(&self, rel: &str) -> Result<String> {
        self.files
            .get(rel)
            .cloned()
            .ok_or_else(|| anyhow!("no such file: {rel}"))
    }
    fn write_file(&mut self, rel: &str, contents: &str) -> Result<()> {
        self.files.insert(rel.to_string(), contents.to_string());
        Ok(())
    }
    fn list_files(&self) -> Result<Vec<String>> {
        Ok(self.files.keys().cloned().collect())
    }
    fn run(&self, _cmd: &str) -> Result<ExecResult> {
        Ok(ExecResult {
            exit_code: self.run_exit,
            stdout: String::new(),
            stderr: String::new(),
        })
    }
}

