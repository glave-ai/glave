//! Agents. Deterministic stubs (noop/solve) + the real model-driven loop. All
//! implement one trait and depend only on ports, so each is testable with fakes.
//! The model agent drives the typed tool grammar (`tools`) through a mediation
//! `Policy`.

use anyhow::Result;
use std::collections::BTreeSet;
use std::path::Path;

use crate::event::RunEvent;
use crate::policy::{Decision, Policy};
use crate::ports::{Message, ModelProvider, TraceSink, WorkspaceRuntime};
use crate::task::TaskSpec;
use crate::tools::{self, ToolCall};

pub trait Agent {
    fn act(
        &mut self,
        rt: &mut dyn WorkspaceRuntime,
        task: &TaskSpec,
        run_id: &str,
        trace: &mut dyn TraceSink,
    ) -> Result<()>;
}

/// Makes no changes — the buggy workspace stays UNSOLVED.
pub struct NoopAgent;

impl Agent for NoopAgent {
    fn act(
        &mut self,
        _rt: &mut dyn WorkspaceRuntime,
        _task: &TaskSpec,
        run_id: &str,
        trace: &mut dyn TraceSink,
    ) -> Result<()> {
        trace.emit(RunEvent::agent_action(
            run_id,
            "noop",
            None,
            Some("stub made no changes".to_string()),
        ))?;
        trace.emit(RunEvent::cost_usage_zero(run_id))?;
        Ok(())
    }
}

/// Applies the task's reference solution — becomes SOLVED.
pub struct SolveAgent;

impl Agent for SolveAgent {
    fn act(
        &mut self,
        rt: &mut dyn WorkspaceRuntime,
        task: &TaskSpec,
        run_id: &str,
        trace: &mut dyn TraceSink,
    ) -> Result<()> {
        let sol = task.solution_dir();
        let mut files = Vec::new();
        collect(&sol, &sol, &mut files)?;
        files.sort();
        for rel in files {
            let contents = std::fs::read_to_string(sol.join(&rel))?;
            rt.write_file(&rel, &contents)?;
            trace.emit(RunEvent::agent_action(
                run_id,
                "write_file",
                Some(rel.clone()),
                Some("applied reference solution".to_string()),
            ))?;
        }
        trace.emit(RunEvent::cost_usage_zero(run_id))?;
        Ok(())
    }
}

/// The real model-driven agent: drives the tool grammar (view / str_replace /
/// create / bash / run_tests / submit) through the mediation policy, bounded by
/// turn and token budgets.
pub struct ModelAgent {
    provider: Box<dyn ModelProvider>,
    policy: Box<dyn Policy>,
    max_turns: u32,
    max_tokens: u64,
}

impl ModelAgent {
    pub fn new(provider: Box<dyn ModelProvider>, policy: Box<dyn Policy>) -> Self {
        Self {
            provider,
            policy,
            max_turns: 14,
            max_tokens: 120_000,
        }
    }
}

impl Agent for ModelAgent {
    fn act(
        &mut self,
        rt: &mut dyn WorkspaceRuntime,
        task: &TaskSpec,
        run_id: &str,
        trace: &mut dyn TraceSink,
    ) -> Result<()> {
        let mut messages = vec![
            Message::new("system", system_prompt()),
            Message::new("user", initial_user(&*rt, task)?),
        ];
        let mut viewed = BTreeSet::new();
        let mut total_prompt = 0u64;
        let mut total_completion = 0u64;
        let mut total_cost = 0.0f64;

        for turn in 1..=self.max_turns {
            let c = self.provider.complete(&messages)?;
            total_prompt += c.prompt_tokens;
            total_completion += c.completion_tokens;
            total_cost += c.cost_usd;
            trace.emit(RunEvent::model_turn(run_id, turn, &c))?;
            messages.push(Message::new("assistant", c.content.clone()));

            match tools::parse(&c.content) {
                None => messages.push(Message::new("user", help_text())),
                Some(ToolCall::Submit) => {
                    trace.emit(RunEvent::agent_action(run_id, "submit", None, None))?;
                    break;
                }
                Some(call) => match self.policy.decide(&call) {
                    // Autonomous eval: Ask is treated as Deny — zero HIL.
                    Decision::Deny(reason) | Decision::Ask(reason) => {
                        trace.emit(RunEvent::agent_action(
                            run_id,
                            "denied",
                            call.path(),
                            Some(reason.clone()),
                        ))?;
                        messages.push(Message::new("user", format!("DENIED: {reason}")));
                    }
                    Decision::Allow => {
                        let obs = tools::execute(&call, rt, task, &mut viewed)?;
                        trace.emit(RunEvent::agent_action(run_id, call.name(), call.path(), None))?;
                        messages.push(Message::new("user", obs));
                    }
                },
            }

            if total_prompt + total_completion >= self.max_tokens {
                trace.emit(RunEvent::agent_action(
                    run_id,
                    "budget_stop",
                    None,
                    Some(format!("token budget {} reached", self.max_tokens)),
                ))?;
                break;
            }
        }

        trace.emit(RunEvent::cost_usage_total(
            run_id,
            self.provider.model_id(),
            total_prompt,
            total_completion,
            total_cost,
        ))?;
        Ok(())
    }
}

/// One-shot agent (the "bare" baseline): a single model call, no tools, no
/// iteration. Shows the failing test + current files, asks for corrected file(s)
/// as <create> blocks, applies them. Measures the raw model.
pub struct OneShotAgent {
    provider: Box<dyn ModelProvider>,
}

impl OneShotAgent {
    pub fn new(provider: Box<dyn ModelProvider>) -> Self {
        Self { provider }
    }
}

impl Agent for OneShotAgent {
    fn act(
        &mut self,
        rt: &mut dyn WorkspaceRuntime,
        task: &TaskSpec,
        run_id: &str,
        trace: &mut dyn TraceSink,
    ) -> Result<()> {
        let messages = vec![
            Message::new("system", oneshot_system()),
            Message::new("user", oneshot_user(&*rt, task)?),
        ];
        let c = self.provider.complete(&messages)?;
        trace.emit(RunEvent::model_turn(run_id, 1, &c))?;

        let creates = tools::parse_creates(&c.content);
        if creates.is_empty() {
            trace.emit(RunEvent::agent_action(
                run_id,
                "no_edit",
                None,
                Some("no <create> block in response".to_string()),
            ))?;
        } else {
            for (path, content) in creates {
                rt.write_file(&path, &content)?;
                trace.emit(RunEvent::agent_action(run_id, "create", Some(path), None))?;
            }
        }

        trace.emit(RunEvent::cost_usage_total(
            run_id,
            self.provider.model_id(),
            c.prompt_tokens,
            c.completion_tokens,
            c.cost_usd,
        ))?;
        Ok(())
    }
}

fn oneshot_system() -> String {
    "You are given a failing test and the current source files. In ONE reply, \
     output the corrected version of every file you need to change, each as:\n\
     <create path=\"FILE\">\\nFULL NEW FILE CONTENT\\n</create>\n\
     Output nothing else. You have no tools and no second turn."
        .to_string()
}

fn oneshot_user(rt: &dyn WorkspaceRuntime, task: &TaskSpec) -> Result<String> {
    let mut out = String::new();
    out.push_str(&format!(
        "TASK: {} ({} / {})\n\nA test is FAILING and defines the required behavior.\n\n",
        task.archetype, task.language, task.difficulty
    ));
    let mut specs = Vec::new();
    let _ = collect(&task.oracle_dir(), &task.oracle_dir(), &mut specs);
    specs.sort();
    for rel in specs {
        if let Ok(src) = std::fs::read_to_string(task.oracle_dir().join(&rel)) {
            out.push_str(&format!("--- test: {rel} ---\n{src}\n\n"));
        }
    }
    out.push_str("Current files:\n\n");
    let mut files = rt.list_files()?;
    files.sort();
    for f in &files {
        if let Ok(src) = rt.read_file(f) {
            out.push_str(&format!("--- {f} ---\n{src}\n\n"));
        }
    }
    out.push_str("Output the corrected file(s) as <create> block(s), nothing else.");
    Ok(out)
}

fn system_prompt() -> String {
    "You are a coding agent. Fix the workspace so the failing test passes without \
     breaking the previously-passing tests. Respond with EXACTLY ONE action per \
     message, using these tags:\n\
     - <view path=\"FILE\"/> or <view path=\"FILE\" range=\"START-END\"/> — read a file (with line numbers)\n\
     - <str_replace path=\"FILE\"><search>\\nEXACT EXISTING TEXT\\n</search><replace>\\nNEW TEXT\\n</replace></str_replace> — replace an exact, unique snippet (view the file first)\n\
     - <create path=\"FILE\">\\nFULL CONTENT\\n</create> — create or overwrite a file\n\
     - <bash>COMMAND</bash> — run a shell command (build, delete files, …)\n\
     - <grep pattern=\"REGEX\"/> or <grep pattern=\"REGEX\" glob=\"*.go\"/> — search file contents (git-aware)\n\
     - <glob pattern=\"GLOB\"/> — find files by name (git-aware)\n\
     - <run_tests/> — run the tests and see the result\n\
     - <submit/> — when the failing test passes\n\
     Only edit files within the workspace. View a file before editing it. Keep changes minimal."
        .to_string()
}

fn help_text() -> String {
    "No valid action found. Respond with exactly one of: <view .../>, \
     <str_replace ...>, <create ...>, <bash>...</bash>, <run_tests/>, <submit/>."
        .to_string()
}

fn initial_user(rt: &dyn WorkspaceRuntime, task: &TaskSpec) -> Result<String> {
    let mut out = String::new();
    out.push_str(&format!(
        "TASK: {} ({} / {})\n\nA test is currently FAILING and defines the required behavior.\n\n",
        task.archetype, task.language, task.difficulty
    ));

    let mut specs = Vec::new();
    let _ = collect(&task.oracle_dir(), &task.oracle_dir(), &mut specs);
    specs.sort();
    for rel in specs {
        if let Ok(src) = std::fs::read_to_string(task.oracle_dir().join(&rel)) {
            out.push_str(&format!("--- test: {rel} ---\n{src}\n\n"));
        }
    }

    out.push_str("Workspace files (use <view> to read them):\n");
    let mut files = rt.list_files()?;
    files.sort();
    for f in &files {
        out.push_str(&format!("- {f}\n"));
    }

    out.push_str("\nBegin. View the relevant file, fix it, run the tests, then submit.");
    Ok(out)
}

fn collect(root: &Path, dir: &Path, out: &mut Vec<String>) -> Result<()> {
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let p = entry.path();
        if p.is_dir() {
            collect(root, &p, out)?;
        } else if let Ok(rel) = p.strip_prefix(root) {
            out.push(rel.to_string_lossy().replace('\\', "/"));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::policy::AutonomousPolicy;
    use crate::sinks::InMemorySink;
    use crate::testkit::{InMemoryProvider, InMemoryRuntime};

    fn count_action(events: &[RunEvent], action: &str) -> usize {
        events
            .iter()
            .filter(|e| matches!(e, RunEvent::AgentAction { action: a, .. } if a == action))
            .count()
    }

    #[test]
    fn model_agent_creates_then_submits() -> Result<()> {
        let provider = Box::new(InMemoryProvider::new(vec![
            "<create path=\"solver.py\">\nFIXED\n</create>",
            "<submit/>",
        ]));
        let policy = Box::new(AutonomousPolicy::new(&[]));
        let mut agent = ModelAgent::new(provider, policy);
        let mut rt = InMemoryRuntime::new().with_file("solver.py", "BUGGY");
        let mut sink = InMemorySink::default();
        let task = TaskSpec::from_toml(
            "id = \"t\"\nlanguage = \"python\"\narchetype = \"a\"\ndifficulty = \"easy\"\n\
             [image]\nbase = \"x\"\n[oracle]\nfail_to_pass = \"true\"\n",
        )?;

        agent.act(&mut rt, &task, "run-test", &mut sink)?;

        assert_eq!(rt.file("solver.py").map(String::as_str), Some("FIXED"));
        assert_eq!(count_action(&sink.events, "create"), 1);
        assert_eq!(count_action(&sink.events, "submit"), 1);
        Ok(())
    }

    #[test]
    fn edit_outside_editable_surface_is_denied() -> Result<()> {
        // str_replace on a file outside the editable surface must be denied.
        let provider = Box::new(InMemoryProvider::new(vec![
            "<str_replace path=\"secret.py\"><search>\na\n</search><replace>\nb\n</replace></str_replace>",
            "<submit/>",
        ]));
        let policy = Box::new(AutonomousPolicy::new(&["solver.py".to_string()]));
        let mut agent = ModelAgent::new(provider, policy);
        let mut rt = InMemoryRuntime::new().with_file("secret.py", "a");
        let mut sink = InMemorySink::default();
        let task = TaskSpec::from_toml(
            "id = \"t\"\nlanguage = \"python\"\narchetype = \"a\"\ndifficulty = \"easy\"\n\
             [image]\nbase = \"x\"\n[oracle]\nfail_to_pass = \"true\"\n",
        )?;

        agent.act(&mut rt, &task, "run-test", &mut sink)?;

        assert_eq!(rt.file("secret.py").map(String::as_str), Some("a")); // unchanged
        assert_eq!(count_action(&sink.events, "denied"), 1);
        Ok(())
    }
}
