//! Project trust (`specs/policy.md`, issue #83). A repository can bring things that run
//! commands: MCP servers in its `.mcp.json`, and settings that loosen approvals or the sandbox.
//! glave asks once per project, only when there is something to ask about, remembers the answer
//! in `trust.toml` in its folder, and asks again only about what changes. Policy can trust
//! projects without a question or forbid their servers. With no one to ask, they stay off.

mod ask;
mod assess;
mod project;
mod record;
#[cfg(test)]
mod tests;

use std::collections::BTreeMap;
use std::path::Path;

pub use assess::{Answer, Outcome, assess};
pub use project::Project;
pub use record::Record;

use crate::policy::Constraints;

/// Who can be asked.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Asking {
    /// The person at the terminal.
    Person,
    /// No one: a script, a pipe, the session server.
    NoOne,
    /// No one is asked and nothing is saved; `glave policy explain` only shows.
    Explaining,
}

/// What `project` does under the record in `home`, asking the person if that is needed and
/// possible, and keeping the answer.
pub async fn resolve(
    home: &Path,
    project: Project,
    constraints: &Constraints,
    asking: Asking,
) -> anyhow::Result<Outcome> {
    let mut record = Record::load(home).await?;
    let assessment = assess(project, &record, constraints);
    let answer = match asking {
        Asking::Explaining => Answer::Explaining,
        Asking::NoOne => Answer::NoOne,
        Asking::Person if assessment.undecided().is_empty() => Answer::NoOne,
        Asking::Person => match ask::ask(ask::question(&assessment)).await? {
            Some(true) => Answer::Yes,
            Some(false) => Answer::No,
            None => Answer::NoOne,
        },
    };
    let project = assessment.project.clone();
    let outcome = assessment.apply(answer);
    if let Some(decisions) = &outcome.decisions {
        record.remember(&project, decisions.clone());
        record.save(home).await?;
    }
    Ok(outcome)
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
