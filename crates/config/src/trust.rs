//! Project trust (`specs/policy.md`, issue #83). A repository can bring things that run
//! commands: MCP servers in its `.mcp.json`, and settings that loosen approvals or the sandbox.
//! glave asks once per project, only when there is something to ask about, in the terminal or
//! in an app (issue #250), remembers the answer in the store's trust ledger (kept by the caller
//! from what [`Outcome::decisions`] says), and asks again only about what changes. Policy can trust projects without a question or forbid their
//! servers. Until someone answers, they stay off.

mod assess;
mod project;
mod protect;
mod record;
#[cfg(test)]
mod tests;

use std::collections::BTreeMap;

pub use assess::{Answer, Assessment, Outcome, assess, assess_there};
pub use project::{Plain, Project, remote};
pub use protect::protections;
pub use record::{Decision, Entry, Record};

use crate::policy::Constraints;

/// Who can be asked. The terminal asks through the person's server, as an app does
/// (`specs/one-server.md`, "The CLI as a client").
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Asking {
    /// No one: a script, a pipe.
    NoOne,
    /// No one here, but the person can answer later from an app: the session server.
    Later,
    /// No one is asked and nothing is saved; `glave policy explain` only shows.
    Explaining,
}

/// What `project` does under `record`, as `asking` leaves it: nothing is asked here. The
/// answer to keep, when there is one, is the outcome's `decisions`.
pub fn resolve(
    record: &Record,
    project: Project,
    constraints: &Constraints,
    asking: Asking,
) -> Outcome {
    let assessment = assess(project, record, constraints);
    let answer = match asking {
        Asking::Explaining => Answer::Explaining,
        Asking::NoOne => Answer::NoOne,
        Asking::Later => Answer::Later,
    };
    assessment.apply(answer)
}

/// The trusted project servers as one `.mcp.json`, ready to read like the person's own.
pub fn mcp_json(servers: &BTreeMap<String, String>) -> String {
    let servers: BTreeMap<&str, serde_json::Value> = servers
        .iter()
        .map(|(name, text)| {
            let value = serde_json::from_str(text).unwrap_or(serde_json::Value::Null);
            (name.as_str(), value)
        })
        .collect();
    serde_json::json!({ "mcpServers": servers }).to_string()
}

/// Trusts what `project` asks to be trusted, as the person's answer to the ask named `asked`
/// ([`Assessment::ask_id`]), under `record`: the entry to keep, exactly as the terminal keeps
/// a yes. None, trusting nothing, when the project no longer asks exactly that: it changed
/// since the person was shown it, or was answered already.
pub fn trust_asked(
    record: &Record,
    project: Project,
    constraints: &Constraints,
    asked: &str,
) -> Option<Entry> {
    trust_shown(assess(project, record, constraints), asked)
}

/// The same for a project on a person's own computer ([`assess_there`]).
pub fn trust_asked_there(
    record: &Record,
    project: Project,
    constraints: &Constraints,
    asked: &str,
) -> Option<Entry> {
    trust_shown(assess_there(project, record, constraints), asked)
}

/// The entry that keeps a yes to `asked`, when `assessment` still asks exactly that.
fn trust_shown(assessment: Assessment, asked: &str) -> Option<Entry> {
    if assessment.undecided().is_empty() || assessment.ask_id() != asked {
        return None;
    }
    let project = assessment.project.clone();
    let decisions = assessment.apply(Answer::Yes).decisions?;
    Some(Record::deciding(&project, decisions))
}
