//! A task's connected services, when it has a `connected.json` (#247): every connector of
//! `evals/connectors`, beside the tasks' folder, each a service someone connected, together
//! 1,787 tools of real size (GitHub's MCP server, Atlassian's, GitHub's REST API, and smaller
//! ones), more than a model's list holds, so the harness lists each connector as one tool and
//! the model searches. A tool answers with the first of the task's answers for it whose `when`
//! its arguments hold, and anything else finds nothing. In-process, since what is measured is
//! the harness's listing and search, not the protocol.

use std::collections::BTreeMap;
use std::future::ready;
use std::path::Path;
use std::sync::Arc;

use glave_event::Arguments;
use glave_ports::{
    BoxFuture, Cancellation, Effect, ToolError, ToolInfo, ToolOutput, ToolSource, ToolSpec,
};
use serde::Deserialize;

use crate::error::EvalError;
use crate::task::Task;

/// What a tool with no answer for its arguments says.
const NOTHING: &str = "No records match.";

/// One connector, as a file of `evals/connectors` holds it.
#[derive(Deserialize)]
struct Connector {
    /// What it says of itself, as a server's instructions do.
    description: String,
    tools: Vec<ToolSpec>,
}

/// One answer of a task's: its text, when the call's arguments hold `when`.
#[derive(Clone, Deserialize)]
struct Answer {
    when: String,
    text: String,
}

/// A connector's tools, answering as the task says.
struct Service {
    connector: Connector,
    answers: BTreeMap<String, Vec<Answer>>,
}

impl ToolSource for Service {
    fn tools(&self) -> BoxFuture<'_, Result<Vec<ToolInfo>, ToolError>> {
        let tools = self.connector.tools.iter().cloned().map(|spec| {
            let reads = reads(&spec.name);
            ToolInfo {
                spec,
                effect: if reads { Effect::Reads } else { Effect::Runs },
                parallel: reads,
            }
        });
        Box::pin(ready(Ok(tools.collect())))
    }

    fn connected(&self) -> bool {
        true
    }

    fn instructions(&self) -> Option<String> {
        Some(self.connector.description.clone())
    }

    fn call<'a>(
        &'a self,
        name: &'a str,
        arguments: &'a Arguments,
        _cancel: &'a dyn Cancellation,
    ) -> BoxFuture<'a, ToolOutput> {
        let given = match arguments {
            Arguments::Json(map) => serde_json::Value::Object(map.clone()).to_string(),
            Arguments::Malformed(raw) => raw.clone(),
        }
        .to_lowercase();
        let answered = (self.answers.get(name).into_iter().flatten())
            .find(|answer| given.contains(&answer.when.to_lowercase()))
            .map_or(NOTHING, |answer| answer.text.as_str());
        Box::pin(ready(ToolOutput::text(false, answered)))
    }
}

/// Words a tool's name holds when it only reads, and words it holds when it may change
/// something.
const READS: [&str; 13] = [
    "get", "list", "search", "read", "find", "query", "retrieve", "describe", "fetch", "lookup",
    "view", "show", "count",
];
const CHANGES: [&str; 40] = [
    "create",
    "update",
    "delete",
    "add",
    "edit",
    "remove",
    "set",
    "send",
    "post",
    "upload",
    "share",
    "move",
    "transition",
    "merge",
    "close",
    "reopen",
    "assign",
    "mute",
    "resolve",
    "acknowledge",
    "tag",
    "log",
    "deactivate",
    "reply",
    "write",
    "run",
    "cancel",
    "publish",
    "put",
    "rename",
    "lock",
    "approve",
    "dismiss",
    "invite",
    "transfer",
    "rerun",
    "trigger",
    "dispatch",
    "submit",
    "refund",
];

/// Whether the tool `name` says it only reads (MCP's `readOnlyHint`), as a well-behaved
/// server's read tools do: the stand-ins carry no hints of their own, so a name holding a word
/// of [`READS`] and none of [`CHANGES`] says so (`list_channels`, `getJiraIssue`, `issue_read`).
fn reads(name: &str) -> bool {
    let short = name.rsplit("__").next().unwrap_or(name);
    let mut words = vec![String::new()];
    let mut lower = false;
    for c in short.chars() {
        if !c.is_alphanumeric() || (lower && c.is_uppercase()) {
            words.push(String::new());
        }
        if let Some(word) = words.last_mut().filter(|_| c.is_alphanumeric()) {
            word.extend(c.to_lowercase());
        }
        lower = c.is_lowercase();
    }
    let holds = |list: &[&str]| words.iter().any(|word| list.contains(&word.as_str()));
    holds(&READS) && !holds(&CHANGES)
}

/// The connected services for `task`, one per connector; none when it has no
/// `connected.json`.
pub(crate) async fn services(task: &Task) -> Result<Vec<Arc<dyn ToolSource>>, EvalError> {
    let Some(file) = task.connected_answers() else {
        return Ok(Vec::new());
    };
    let answers: BTreeMap<String, Vec<Answer>> = read_json(&file).await?;
    let folder = task
        .dir
        .parent()
        .and_then(Path::parent)
        .map(|evals| evals.join("connectors"));
    let folder = folder.ok_or_else(|| EvalError::Setup(format!("{}: no connectors", task.name)))?;
    let mut files = Vec::new();
    let mut entries = tokio::fs::read_dir(&folder)
        .await
        .map_err(EvalError::io("read", &folder))?;
    while let Some(entry) = entries
        .next_entry()
        .await
        .map_err(EvalError::io("read", &folder))?
    {
        files.push(entry.path());
    }
    files.retain(|path| path.extension().is_some_and(|ext| ext == "json"));
    files.sort();
    let mut services: Vec<Arc<dyn ToolSource>> = Vec::new();
    for path in files {
        let connectors: BTreeMap<String, Connector> = read_json(&path).await?;
        for connector in connectors.into_values() {
            let answers = (connector.tools.iter())
                .filter_map(|tool| Some((tool.name.clone(), answers.get(&tool.name)?.clone())))
                .collect();
            services.push(Arc::new(Service { connector, answers }));
        }
    }
    Ok(services)
}

/// The JSON in the file at `path`.
async fn read_json<T: for<'de> Deserialize<'de>>(path: &Path) -> Result<T, EvalError> {
    let text = tokio::fs::read_to_string(path)
        .await
        .map_err(EvalError::io("read", path))?;
    serde_json::from_str(&text)
        .map_err(|err| EvalError::Setup(format!("{}: {err}", path.display())))
}

#[cfg(test)]
mod tests {
    use super::*;
    use glave_ports::NeverCancelled;

    #[test]
    fn a_tool_answers_as_the_task_says_and_any_other_finds_nothing() {
        let spec = |name: &str| ToolSpec {
            name: name.into(),
            description: String::new(),
            parameters: serde_json::json!({"type": "object"}),
        };
        let answer = Answer {
            when: "OPS-311".into(),
            text: "In Progress".into(),
        };
        let service = Service {
            connector: Connector {
                description: "Jira".into(),
                tools: vec![spec("mcp__jira__get"), spec("mcp__jira__other")],
            },
            answers: BTreeMap::from([("mcp__jira__get".to_owned(), vec![answer])]),
        };
        let asked = |key: &str| {
            let mut map = serde_json::Map::new();
            map.insert("key".into(), key.into());
            Arguments::Json(map)
        };
        glave_ports::block_on!(async {
            let cancel = NeverCancelled;
            let found = service
                .call("mcp__jira__get", &asked("ops-311"), &cancel)
                .await;
            assert_eq!(found, ToolOutput::text(false, "In Progress"));
            let other = service
                .call("mcp__jira__get", &asked("OPS-1"), &cancel)
                .await;
            assert_eq!(other, ToolOutput::text(false, NOTHING));
            let none = service
                .call("mcp__jira__other", &asked("OPS-311"), &cancel)
                .await;
            assert_eq!(none, ToolOutput::text(false, NOTHING));
        });
    }

    #[test]
    fn a_stand_in_s_reads_say_they_only_read_and_the_rest_may_write() {
        for read in [
            "mcp__slack__list_channels",
            "mcp__gmail__search_messages",
            "mcp__atlassian__getJiraIssue",
            "mcp__github__issue_read",
            "mcp__github_rest__repos_get_content",
        ] {
            assert!(reads(read), "{read}");
        }
        for write in [
            "mcp__slack__post_message",
            "mcp__gmail__send_message",
            "mcp__atlassian__addOrEditJiraIssueComment",
            "mcp__github__issue_write",
            "mcp__snowflake__run_query",
            "mcp__figma__export_image",
        ] {
            assert!(!reads(write), "{write}");
        }
    }

    #[test]
    fn the_connectors_beside_the_tasks_are_more_than_a_model_s_list_holds() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../evals/connectors");
        let mut tools = 0;
        for entry in std::fs::read_dir(dir).unwrap() {
            let text = std::fs::read_to_string(entry.unwrap().path()).unwrap();
            let connectors: BTreeMap<String, Connector> = serde_json::from_str(&text).unwrap();
            tools += connectors.values().map(|c| c.tools.len()).sum::<usize>();
        }
        assert_eq!(tools, 1_787);
    }
}
