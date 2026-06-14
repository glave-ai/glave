//! In-memory adapters for deterministic, I/O-free tests. Gated behind the
//! `test-util` feature (and available in this crate's own tests).

use anyhow::{anyhow, Result};
use std::cell::RefCell;
use std::collections::{BTreeMap, VecDeque};

use crate::ports::{
    Completion, ExecResult, Grader, Message, ModelProvider, ToolCallRequest, Workspace,
};

/// Returns scripted completions in order — deterministic, zero network, zero cost.
pub struct InMemoryProvider {
    scripted: RefCell<VecDeque<Completion>>,
    model: String,
}

impl InMemoryProvider {
    /// Script plain-text completions (no tool calls).
    pub fn new(responses: Vec<&str>) -> Self {
        let scripted = responses
            .into_iter()
            .map(|s| Completion {
                content: s.to_string(),
                model: "inmemory".to_string(),
                ..Completion::default()
            })
            .collect();
        Self {
            scripted: RefCell::new(scripted),
            model: "inmemory".to_string(),
        }
    }

    /// Script native tool-calling completions: each turn is a list of
    /// `(tool_name, json_args)`. Tool-call ids are auto-assigned and unique.
    pub fn with_tool_calls(turns: Vec<Vec<(&str, &str)>>) -> Self {
        let mut scripted = VecDeque::new();
        for (ti, calls) in turns.into_iter().enumerate() {
            let tool_calls = calls
                .into_iter()
                .enumerate()
                .map(|(ci, (name, args))| ToolCallRequest {
                    id: format!("c{ti}_{ci}"),
                    name: name.to_string(),
                    arguments: args.to_string(),
                })
                .collect();
            scripted.push_back(Completion {
                tool_calls,
                model: "inmemory".to_string(),
                ..Completion::default()
            });
        }
        Self {
            scripted: RefCell::new(scripted),
            model: "inmemory".to_string(),
        }
    }
}

impl ModelProvider for InMemoryProvider {
    fn complete(&self, _messages: &[Message]) -> Result<Completion> {
        Ok(self.scripted.borrow_mut().pop_front().unwrap_or_default())
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

impl Workspace for InMemoryRuntime {
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

    // Simplified fake: substring match (the real runtime uses regex via ripgrep).
    fn grep(&self, pattern: &str, file_glob: Option<&str>) -> Result<Vec<String>> {
        let glob = file_glob.and_then(|g| glob::Pattern::new(g).ok());
        let mut out = Vec::new();
        for (path, content) in &self.files {
            if glob.as_ref().is_some_and(|g| !g.matches(path)) {
                continue;
            }
            for (i, line) in content.lines().enumerate() {
                if line.contains(pattern) {
                    out.push(format!("{path}:{}:{line}", i + 1));
                }
            }
        }
        Ok(out)
    }

    fn find(&self, glob: &str) -> Result<Vec<String>> {
        let g = glob::Pattern::new(glob).map_err(|e| anyhow!("invalid glob: {e}"))?;
        Ok(self
            .files
            .keys()
            .filter(|p| g.matches(p))
            .cloned()
            .collect())
    }
}

/// The eval-side grader for the fake: grading just runs the command (the held
/// tests are scripted via `set_run_exit`). Kept off `Workspace` so the fake mirrors
/// the real seal — only the eval path, holding an `EvalRuntime`, can grade.
impl Grader for InMemoryRuntime {
    fn grade(&self, cmd: &str) -> Result<ExecResult> {
        self.run(cmd)
    }
}
