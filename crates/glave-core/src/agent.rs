//! Agents. Deterministic stubs (noop/solve) + the real model-driven loop. All
//! implement one trait and depend only on ports, so each is testable with fakes.

use anyhow::Result;
use std::collections::BTreeSet;
use std::path::Path;

use crate::event::RunEvent;
use crate::ports::{Message, ModelProvider, TraceSink, WorkspaceRuntime};
use crate::task::TaskSpec;

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

/// The real model-driven agent: read -> edit -> run-tests -> submit over a tiny
/// XML action grammar, bounded by turn and token budgets.
pub struct ModelAgent {
    provider: Box<dyn ModelProvider>,
    max_turns: u32,
    max_tokens: u64,
}

impl ModelAgent {
    pub fn new(provider: Box<dyn ModelProvider>) -> Self {
        Self {
            provider,
            max_turns: 10,
            max_tokens: 80_000,
        }
    }
}

enum Action {
    Read(String),
    Write { path: String, content: String },
    RunTests,
    Submit,
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

            match parse_action(&c.content) {
                Some(Action::Submit) => {
                    trace.emit(RunEvent::agent_action(run_id, "submit", None, None))?;
                    break;
                }
                Some(Action::Read(path)) => {
                    trace.emit(RunEvent::agent_action(run_id, "read", Some(path.clone()), None))?;
                    let obs = match rt.read_file(&path) {
                        Ok(s) => format!("<file path=\"{path}\">\n{s}\n</file>"),
                        Err(e) => format!("ERROR reading {path}: {e}"),
                    };
                    messages.push(Message::new("user", obs));
                }
                Some(Action::Write { path, content }) => {
                    rt.write_file(&path, &content)?;
                    trace.emit(RunEvent::agent_action(
                        run_id,
                        "write_file",
                        Some(path.clone()),
                        None,
                    ))?;
                    messages.push(Message::new(
                        "user",
                        format!("wrote {path} ({} bytes)", content.len()),
                    ));
                }
                Some(Action::RunTests) => {
                    trace.emit(RunEvent::agent_action(run_id, "run_tests", None, None))?;
                    let obs = run_tests(&*rt, task)?;
                    messages.push(Message::new("user", obs));
                }
                None => {
                    messages.push(Message::new(
                        "user",
                        "No valid action found. Respond with exactly one of: \
                         <read path=\"...\"/>, <write path=\"...\">CONTENT</write>, \
                         <run_tests/>, <submit/>.",
                    ));
                }
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

fn system_prompt() -> String {
    "You are a coding agent. Fix the workspace code so the failing test passes \
     without breaking the previously-passing tests. Respond with EXACTLY ONE \
     action per message, using these tags:\n\
     - <read path=\"FILE\"/> to read a workspace file\n\
     - <write path=\"FILE\">FULL NEW FILE CONTENT</write> to overwrite a file\n\
     - <run_tests/> to run the tests and see the result\n\
     - <submit/> when you believe the failing test now passes\n\
     Only edit files in the workspace. Do not edit the tests. Keep changes minimal."
        .to_string()
}

fn initial_user(rt: &dyn WorkspaceRuntime, task: &TaskSpec) -> Result<String> {
    let mut out = String::new();
    out.push_str(&format!(
        "TASK: {} ({} / {})\n\nA test is currently FAILING and defines the required behavior.\n\n",
        task.archetype, task.language, task.difficulty
    ));

    let mut seen = BTreeSet::new();
    for node in &task.oracle.fail_to_pass {
        let file = node.split("::").next().unwrap_or(node);
        let base = file.trim_start_matches("/oracle/");
        if !seen.insert(base.to_string()) {
            continue;
        }
        if let Ok(src) = std::fs::read_to_string(task.oracle_dir().join(base)) {
            out.push_str(&format!("--- failing test: {base} ---\n{src}\n\n"));
        }
    }

    out.push_str("Editable workspace files:\n\n");
    for f in rt.list_files()? {
        if let Ok(src) = rt.read_file(&f) {
            out.push_str(&format!("--- {f} ---\n{src}\n\n"));
        }
    }

    out.push_str("Respond with one action. Call <submit/> once the failing test passes.");
    Ok(out)
}

fn run_tests(rt: &dyn WorkspaceRuntime, task: &TaskSpec) -> Result<String> {
    let v = crate::oracle::evaluate(rt, task)?;
    Ok(format!(
        "TEST RESULTS\nFAIL_TO_PASS: {}\nPASS_TO_PASS: {}\n--- output ---\n{}",
        if v.fail_to_pass_ok { "PASS" } else { "FAIL" },
        if v.pass_to_pass_ok { "PASS" } else { "FAIL" },
        v.f2p_output.chars().take(2000).collect::<String>()
    ))
}

fn parse_action(s: &str) -> Option<Action> {
    let kind = [
        ("write", s.find("<write")),
        ("read", s.find("<read")),
        ("run_tests", s.find("<run_tests")),
        ("submit", s.find("<submit")),
    ]
    .into_iter()
    .filter_map(|(k, i)| i.map(|i| (k, i)))
    .min_by_key(|(_, i)| *i)?
    .0;

    match kind {
        "write" => parse_write(s),
        "read" => {
            let start = s.find("<read")?;
            extract_attr(s.get(start..)?, "path").map(Action::Read)
        }
        "run_tests" => Some(Action::RunTests),
        "submit" => Some(Action::Submit),
        _ => None,
    }
}

fn parse_write(s: &str) -> Option<Action> {
    let start = s.find("<write")?;
    let rest = s.get(start..)?;
    let path = extract_attr(rest, "path")?;
    let open_end = rest.find('>')?.checked_add(1)?;
    let close = rest.find("</write>")?;
    if close < open_end {
        return None;
    }
    let body = rest.get(open_end..close)?;
    let content = body.strip_prefix('\n').unwrap_or(body).to_string();
    Some(Action::Write { path, content })
}

fn extract_attr(s: &str, attr: &str) -> Option<String> {
    let key = format!("{attr}=\"");
    let start = s.find(&key)?.checked_add(key.len())?;
    let rest = s.get(start..)?;
    let end = rest.find('"')?;
    Some(rest.get(..end)?.to_string())
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
    use crate::testkit::{InMemoryProvider, InMemoryRuntime, InMemorySink};

    fn count_action(events: &[RunEvent], action: &str) -> usize {
        events
            .iter()
            .filter(|e| matches!(e, RunEvent::AgentAction { action: a, .. } if a == action))
            .count()
    }

    #[test]
    fn model_agent_writes_then_submits() -> Result<()> {
        let provider = Box::new(InMemoryProvider::new(vec![
            "<write path=\"solver.py\">FIXED</write>",
            "<submit/>",
        ]));
        let mut agent = ModelAgent::new(provider);
        let mut rt = InMemoryRuntime::new().with_file("solver.py", "BUGGY");
        let mut sink = InMemorySink::default();
        let task = TaskSpec::from_toml(
            "id = \"t\"\nlanguage = \"python\"\narchetype = \"a\"\ndifficulty = \"easy\"\n\
             [image]\nbase = \"x\"\n[oracle]\ncommand = \"true\"\nfail_to_pass = []\n",
        )?;

        agent.act(&mut rt, &task, "run-test", &mut sink)?;

        assert_eq!(rt.file("solver.py").map(String::as_str), Some("FIXED"));
        assert_eq!(count_action(&sink.events, "write_file"), 1);
        assert_eq!(count_action(&sink.events, "submit"), 1);
        Ok(())
    }
}
