//! Agents. Deterministic stubs (noop/solve) + the real model-driven loop. All
//! implement one trait and depend only on ports, so each is testable with fakes.
//! The model agent drives the typed tool grammar (`tools`) through a mediation
//! `Policy`.

use anyhow::Result;
use serde_json::Value;
use std::collections::BTreeSet;
use std::path::Path;

use crate::capability::{self, CapabilityRegistry, LexicalSelector, Selector};
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

/// The real model-driven agent: drives the tool grammar (read / str_replace /
/// create / bash / run_tests / submit) through the mediation policy, bounded by
/// turn and token budgets.
pub struct ModelAgent {
    provider: Box<dyn ModelProvider>,
    policy: Box<dyn Policy>,
    max_turns: u32,
    max_tokens: u64,
    context_window: u64,
    sub_turns: u32,
}

impl ModelAgent {
    pub fn new(provider: Box<dyn ModelProvider>, policy: Box<dyn Policy>) -> Self {
        Self {
            provider,
            policy,
            max_turns: env_parse("GLAVE_MAX_TURNS", 20),
            max_tokens: env_parse("GLAVE_MAX_TOKENS", 120_000),
            context_window: env_parse("GLAVE_CONTEXT_WINDOW", 128_000),
            sub_turns: env_parse("GLAVE_SUB_TURNS", 6),
        }
    }

    /// Run a read-only research subagent in its own context and return its
    /// summary. Shares the runtime (read access) but cannot edit — only
    /// `read`/`grep`/`glob`. Bounded by `sub_turns`.
    fn run_subagent(
        &self,
        rt: &mut dyn WorkspaceRuntime,
        task: &TaskSpec,
        subtask: &str,
    ) -> Result<String> {
        let mut messages = vec![
            Message::new("system", subagent_system()),
            Message::new("user", subtask.to_string()),
        ];
        let mut viewed = BTreeSet::new();
        for _ in 0..self.sub_turns {
            let c = self.provider.complete(&messages)?;
            messages.push(Message::new("assistant", c.content.clone()));
            match tools::parse(&c.content) {
                Some(call) if is_readonly(&call) => {
                    let obs = tools::execute(&call, rt, task, &mut viewed)?;
                    messages.push(Message::new("user", obs));
                }
                // No tool (or an edit/other attempt): the content is the answer.
                _ => return Ok(c.content.trim().to_string()),
            }
        }
        Ok("(subagent reached its turn budget without concluding)".to_string())
    }
}

/// Tools a research subagent may use — strictly read-only.
fn is_readonly(call: &ToolCall) -> bool {
    matches!(
        call,
        ToolCall::Read { .. } | ToolCall::Grep { .. } | ToolCall::Glob { .. }
    )
}

fn subagent_system() -> String {
    "You are a research subagent. Investigate the question using <read .../>, \
     <grep .../>, and <glob .../> ONLY — you cannot edit files or run tests. When \
     you have the answer, reply with a concise summary and NO tool tag."
        .to_string()
}

/// Stub for an elided tool observation. Stable text so compaction is idempotent,
/// and it reminds the model the content is recoverable.
const ELIDED: &str = "[earlier tool output elided to save context — re-read/grep if you need it]";

/// Reclaim context by eliding the OLDEST tool observations (oldest first), only
/// until the estimated size is back under `target_tokens`. The system prompt,
/// the task, and every model turn are never touched — only tool outputs, which
/// are recoverable (re-read / re-grep / re-run). Least destructive: nothing is
/// elided while under budget, and recent observations are preserved.
fn compact_to_fit(messages: &mut [Message], current_tokens: u64, target_tokens: u64) {
    let mut over = current_tokens.saturating_sub(target_tokens);
    if over == 0 {
        return;
    }
    for m in messages.iter_mut().skip(2) {
        if over == 0 {
            break;
        }
        if m.role == "user" && m.content != ELIDED {
            let freed = u64::try_from(m.content.len()).unwrap_or(u64::MAX) / 4; // ~chars per token
            m.content = ELIDED.to_string();
            over = over.saturating_sub(freed);
        }
    }
}

/// Read a numeric tuning knob from the environment, falling back to a default.
fn env_parse<T: std::str::FromStr>(key: &str, default: T) -> T {
    std::env::var(key)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

impl Agent for ModelAgent {
    fn act(
        &mut self,
        rt: &mut dyn WorkspaceRuntime,
        task: &TaskSpec,
        run_id: &str,
        trace: &mut dyn TraceSink,
    ) -> Result<()> {
        let mut mcp = crate::mcp::McpHub::connect(&*rt);
        let mut messages = vec![
            Message::new("system", system_prompt()),
            Message::new("user", initial_user(&*rt, task)?),
        ];
        if !mcp.is_empty() {
            messages.push(Message::new("user", mcp.catalog()));
        }
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
                Some(ToolCall::Subagent { task: subtask }) => {
                    let summary = self.run_subagent(rt, task, &subtask)?;
                    trace.emit(RunEvent::agent_action(
                        run_id,
                        "subagent",
                        None,
                        Some(subtask.clone()),
                    ))?;
                    messages.push(Message::new("user", format!("SUBAGENT RESULT:\n{summary}")));
                }
                Some(ToolCall::Mcp { server, tool, args }) => {
                    let parsed: Value = serde_json::from_str(&args).unwrap_or(Value::Null);
                    let obs = mcp
                        .call(&server, &tool, &parsed)
                        .unwrap_or_else(|e| format!("MCP ERROR: {e}"));
                    trace.emit(RunEvent::agent_action(
                        run_id,
                        "mcp",
                        Some(format!("{server}/{tool}")),
                        None,
                    ))?;
                    messages.push(Message::new("user", obs));
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

            // Compaction: fire only as the real context (qwen ~128k) nears the
            // window. Below ~95% we keep the whole transcript — least destructive.
            if c.prompt_tokens >= self.context_window.saturating_mul(95) / 100 {
                let target = self.context_window.saturating_mul(70) / 100;
                compact_to_fit(&mut messages, c.prompt_tokens, target);
            }

            // Budget on completion (generated) tokens only. Cumulative prompt
            // counts re-sent history every turn — cheap under caching, and it
            // tripped the limit long before the model was actually done.
            if total_completion >= self.max_tokens {
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
        "TASK: {} ({} / {})\n\n",
        task.archetype, task.language, task.difficulty
    ));
    if let Some(guidance) = project_guidance(rt) {
        out.push_str(&format!("PROJECT NOTES (HOPE.md) — follow these:\n{guidance}\n\n"));
    }
    out.push_str(&spec_section(task));
    out.push_str("Current files:\n\n");
    let mut files = rt.list_files()?;
    files.sort();
    // One-shot dumps full contents; cap hard so a large repo can't blow context.
    for f in files.iter().take(25) {
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
     - <read path=\"FILE\"/> or <read path=\"FILE\" offset=\"N\" limit=\"M\"/> — read a file with line numbers (bounded by default; offset = first line, limit = max lines)\n\
     - <str_replace path=\"FILE\"><search>\\nEXACT EXISTING TEXT\\n</search><replace>\\nNEW TEXT\\n</replace></str_replace> — replace an exact, unique snippet (read the file first)\n\
     - <create path=\"FILE\">\\nFULL CONTENT\\n</create> — create or overwrite a file\n\
     - <bash>COMMAND</bash> — run a shell command (build, delete files, …)\n\
     - <grep pattern=\"REGEX\"/> or <grep pattern=\"REGEX\" glob=\"*.go\"/> — search file contents (git-aware)\n\
     - <glob pattern=\"GLOB\"/> — find files by name (git-aware)\n\
     - <run_tests/> — run the tests and see the result\n\
     - <subagent>QUESTION</subagent> — spawn a read-only research subagent to investigate; only its summary returns to you (keeps your context lean)\n\
     - <submit/> — when the failing test passes\n\
     Only edit files within the workspace. Read a file before editing it. Keep changes minimal."
        .to_string()
}

fn help_text() -> String {
    "No valid action found. Respond with exactly one of: <read .../>, \
     <str_replace ...>, <create ...>, <bash>...</bash>, <run_tests/>, <submit/>."
        .to_string()
}

fn initial_user(rt: &dyn WorkspaceRuntime, task: &TaskSpec) -> Result<String> {
    const FILE_CAP: usize = 60;
    let mut out = String::new();
    out.push_str(&format!(
        "TASK: {} ({} / {})\n\n",
        task.archetype, task.language, task.difficulty
    ));
    if let Some(guidance) = project_guidance(rt) {
        out.push_str(&format!("PROJECT NOTES (HOPE.md) — follow these:\n{guidance}\n\n"));
    }
    out.push_str(&spec_section(task));

    // Registry-backed skills: surface only those the selector finds relevant to
    // the task, not the whole catalog. No `.glave/skills/` -> no section.
    let skills = capability::load_skills(rt);
    if !skills.is_empty() {
        let mut registry = CapabilityRegistry::new();
        for s in skills {
            registry.register(s);
        }
        let query = format!("{} {} {}", task.statement, task.archetype, task.language);
        let picked = LexicalSelector.select(&registry, &query, 3);
        if !picked.is_empty() {
            out.push_str("RELEVANT SKILLS:\n");
            for cap in picked {
                out.push_str(&format!("## {} — {}\n{}\n\n", cap.name, cap.description, cap.body));
            }
        }
    }

    out.push_str("Workspace files (use <read> to read them, <grep>/<glob> to navigate):\n");
    let mut files = rt.list_files()?;
    files.sort();
    for f in files.iter().take(FILE_CAP) {
        out.push_str(&format!("- {f}\n"));
    }
    let extra = files.len().saturating_sub(FILE_CAP);
    if extra > 0 {
        out.push_str(&format!(
            "- … (+{extra} more files; use <glob>/<grep> to find what you need)\n"
        ));
    }

    out.push_str("\nBegin. Find the relevant file, fix it, run the tests, then submit.");
    Ok(out)
}

/// Project-level guidance the agent should honor — the `HOPE.md` convention,
/// our analog of `CLAUDE.md`. Read from the workspace root (or `.glave/HOPE.md`);
/// missing or empty yields `None`.
fn project_guidance(rt: &dyn WorkspaceRuntime) -> Option<String> {
    for path in ["HOPE.md", ".glave/HOPE.md"] {
        if let Ok(content) = rt.read_file(path) {
            let trimmed = content.trim();
            if !trimmed.is_empty() {
                return Some(trimmed.to_string());
            }
        }
    }
    None
}

/// The spec shown to the agent: the held issue statement for ingested tasks, or
/// the oracle test files for authored tasks.
fn spec_section(task: &TaskSpec) -> String {
    if task.statement.is_empty() {
        let mut out = String::from("A test is FAILING and defines the required behavior.\n\n");
        let mut specs = Vec::new();
        let _ = collect(&task.oracle_dir(), &task.oracle_dir(), &mut specs);
        specs.sort();
        for rel in specs {
            if let Ok(src) = std::fs::read_to_string(task.oracle_dir().join(&rel)) {
                out.push_str(&format!("--- test: {rel} ---\n{src}\n\n"));
            }
        }
        out
    } else {
        format!(
            "Fix the issue below. A held test verifies the fix; use <run_tests/> to check.\n\n--- issue ---\n{}\n\n",
            task.statement
        )
    }
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

    fn task_for(lang: &str) -> Result<TaskSpec> {
        TaskSpec::from_toml(&format!(
            "id = \"t\"\nlanguage = \"{lang}\"\narchetype = \"a\"\ndifficulty = \"easy\"\n\
             statement = \"do the thing\"\n[image]\nbase = \"x\"\n[oracle]\nfail_to_pass = \"true\"\n"
        ))
    }

    #[test]
    fn hope_guidance_injected_when_present() -> Result<()> {
        let rt = InMemoryRuntime::new()
            .with_file("HOPE.md", "Always run the linter.")
            .with_file("src/main.rs", "fn main() {}");
        let out = initial_user(&rt, &task_for("rust")?)?;
        assert!(out.contains("PROJECT NOTES (HOPE.md)"), "header present: {out}");
        assert!(out.contains("Always run the linter."), "content present: {out}");
        Ok(())
    }

    #[test]
    fn hope_guidance_absent_when_no_file() -> Result<()> {
        let rt = InMemoryRuntime::new().with_file("src/main.rs", "fn main() {}");
        let out = initial_user(&rt, &task_for("rust")?)?;
        assert!(!out.contains("PROJECT NOTES"), "no HOPE section: {out}");
        Ok(())
    }

    #[test]
    fn hope_guidance_empty_file_is_ignored() {
        let rt = InMemoryRuntime::new().with_file("HOPE.md", "   \n  \n");
        assert_eq!(project_guidance(&rt), None);
    }

    #[test]
    fn hope_guidance_falls_back_to_dot_glave() {
        let rt = InMemoryRuntime::new().with_file(".glave/HOPE.md", "nested guidance");
        assert_eq!(project_guidance(&rt).as_deref(), Some("nested guidance"));
    }

    #[test]
    fn hope_guidance_prefers_root_over_dot_glave() {
        let rt = InMemoryRuntime::new()
            .with_file("HOPE.md", "root")
            .with_file(".glave/HOPE.md", "nested");
        assert_eq!(project_guidance(&rt).as_deref(), Some("root"));
    }

    #[test]
    fn hope_guidance_reaches_oneshot_too() -> Result<()> {
        let rt = InMemoryRuntime::new().with_file("HOPE.md", "one-shot guidance");
        let out = oneshot_user(&rt, &task_for("python")?)?;
        assert!(out.contains("one-shot guidance"), "oneshot includes HOPE: {out}");
        Ok(())
    }

    fn statement_task(statement: &str) -> Result<TaskSpec> {
        TaskSpec::from_toml(&format!(
            "id = \"t\"\nlanguage = \"rust\"\narchetype = \"real-issue\"\ndifficulty = \"easy\"\n\
             statement = \"{statement}\"\n[image]\nbase = \"x\"\n[oracle]\nfail_to_pass = \"true\"\n"
        ))
    }

    #[test]
    fn relevant_skill_is_injected() -> Result<()> {
        let rt = InMemoryRuntime::new()
            .with_file(
                ".glave/skills/decode.md",
                "---\ndescription: fix integer decoding and sign extension bugs\n---\nCheck the shift width.",
            )
            .with_file("src/main.rs", "fn main() {}");
        let out = initial_user(&rt, &statement_task("integer decoding is wrong")?)?;
        assert!(out.contains("RELEVANT SKILLS"), "skills section: {out}");
        assert!(out.contains("Check the shift width."), "skill body injected: {out}");
        Ok(())
    }

    #[test]
    fn no_skills_dir_means_no_section() -> Result<()> {
        let rt = InMemoryRuntime::new().with_file("src/main.rs", "fn main() {}");
        let out = initial_user(&rt, &statement_task("integer decoding is wrong")?)?;
        assert!(!out.contains("RELEVANT SKILLS"), "no skills section: {out}");
        Ok(())
    }

    #[test]
    fn irrelevant_skill_is_not_injected() -> Result<()> {
        let rt = InMemoryRuntime::new()
            .with_file(
                ".glave/skills/deploy.md",
                "---\ndescription: deploy kubernetes manifests to production\n---\nkubectl apply.",
            )
            .with_file("src/main.rs", "fn main() {}");
        let out = initial_user(&rt, &statement_task("integer decoding is wrong")?)?;
        assert!(!out.contains("RELEVANT SKILLS"), "irrelevant skill dropped: {out}");
        Ok(())
    }

    /// [system, task, (assistant, sized-observation) × n]. Each observation is
    /// `obs_chars` long (~obs_chars/4 estimated tokens).
    fn sized_convo(n: usize, obs_chars: usize) -> Vec<Message> {
        let mut m = vec![Message::new("system", "S"), Message::new("user", "TASK")];
        for i in 0..n {
            m.push(Message::new("assistant", format!("act{i}")));
            m.push(Message::new("user", "x".repeat(obs_chars)));
        }
        m
    }

    fn obs_elided(m: &[Message]) -> Vec<bool> {
        m.iter()
            .filter(|x| x.role == "user")
            .skip(1) // skip the task message
            .map(|x| x.content == ELIDED)
            .collect()
    }

    #[test]
    fn compaction_noop_when_under_target() {
        let mut m = sized_convo(5, 400); // ~500 tok
        compact_to_fit(&mut m, 100, 200); // already under target
        assert_eq!(obs_elided(&m), [false, false, false, false, false]);
    }

    #[test]
    fn compaction_evicts_oldest_only_to_target() {
        // 5 observations of ~100 tokens each (~500 total); free 300 -> evict 3 oldest.
        let mut m = sized_convo(5, 400);
        compact_to_fit(&mut m, 500, 200);
        assert_eq!(obs_elided(&m), [true, true, true, false, false]);
    }

    #[test]
    fn compaction_preserves_system_task_and_model_turns() {
        let mut m = sized_convo(5, 400);
        compact_to_fit(&mut m, 500, 0); // evict everything possible
        assert_eq!(m.first().map(|x| x.content.as_str()), Some("S"));
        assert_eq!(m.get(1).map(|x| x.content.as_str()), Some("TASK"));
        let acts: Vec<&str> =
            m.iter().filter(|x| x.role == "assistant").map(|x| x.content.as_str()).collect();
        assert_eq!(acts, ["act0", "act1", "act2", "act3", "act4"]); // model turns untouched
    }

    #[test]
    fn is_readonly_allows_only_read_grep_glob() {
        assert!(is_readonly(&ToolCall::Read { path: "f".into(), offset: None, limit: None }));
        assert!(is_readonly(&ToolCall::Grep { pattern: "x".into(), glob: None }));
        assert!(is_readonly(&ToolCall::Glob { pattern: "*".into() }));
        assert!(!is_readonly(&ToolCall::Create { path: "f".into(), content: "c".into() }));
        assert!(!is_readonly(&ToolCall::RunTests));
    }

    #[test]
    fn subagent_runs_then_main_submits() -> Result<()> {
        // main spawns a subagent; sub reads then answers; main submits.
        let provider = Box::new(InMemoryProvider::new(vec![
            "<subagent>where is the bug?</subagent>",
            "<read path=\"src/main.rs\"/>",
            "The bug is on line 1.",
            "<submit/>",
        ]));
        let policy = Box::new(AutonomousPolicy::new(&["src/main.rs".to_string()]));
        let mut agent = ModelAgent::new(provider, policy);
        let mut rt = InMemoryRuntime::new().with_file("src/main.rs", "BUG");
        let mut sink = InMemorySink::default();
        let task = task_for("rust")?;

        agent.act(&mut rt, &task, "run-sub", &mut sink)?;

        assert_eq!(count_action(&sink.events, "subagent"), 1);
        assert_eq!(count_action(&sink.events, "submit"), 1);
        Ok(())
    }

    #[test]
    fn subagent_cannot_edit_the_workspace() -> Result<()> {
        // The subagent attempts a str_replace; it must not change the file.
        let provider = Box::new(InMemoryProvider::new(vec![
            "<subagent>fix it yourself</subagent>",
            "<str_replace path=\"src/main.rs\"><search>\nBUG\n</search><replace>\nFIXED\n</replace></str_replace>",
            "<submit/>",
        ]));
        let policy = Box::new(AutonomousPolicy::new(&["src/main.rs".to_string()]));
        let mut agent = ModelAgent::new(provider, policy);
        let mut rt = InMemoryRuntime::new().with_file("src/main.rs", "BUG");
        let mut sink = InMemorySink::default();
        let task = task_for("rust")?;

        agent.act(&mut rt, &task, "run-sub", &mut sink)?;

        assert_eq!(rt.file("src/main.rs").map(String::as_str), Some("BUG")); // untouched
        Ok(())
    }
}
