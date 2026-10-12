//! `todo`: a checklist the model keeps for work with several steps, as Claude Code, Codex and
//! OpenCode offer one. The model sends the whole list each time; the list lives in the call
//! itself, so the log keeps it and any client can show it. It changes nothing, so it never asks.

use std::future::ready;

use glave_event::Arguments;
use glave_ports::{
    BoxFuture, Cancellation, Effect, ToolError, ToolInfo, ToolOutput, ToolSource, ToolSpec,
};
use serde::Deserialize;
use serde_json::json;

/// The tool's name.
pub const TODO: &str = "todo";

/// The todo tool, as a tool source.
#[derive(Debug, Default, Clone, Copy)]
pub struct Todo;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct List {
    todos: Vec<Item>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Item {
    content: String,
    status: Status,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Status {
    Pending,
    InProgress,
    Completed,
}

fn spec() -> ToolSpec {
    ToolSpec {
        name: TODO.to_owned(),
        description: "Keep a checklist for work with several steps, so you and the person can \
                      see the plan and progress. Send the whole list each time. Mark one item \
                      in_progress while you work on it and mark items completed as soon as they \
                      are done. Skip it for simple tasks."
            .to_owned(),
        parameters: json!({"type": "object", "properties": {
            "todos": {"type": "array", "items": {"type": "object", "properties": {
                "content": {"type": "string", "description": "What to do, briefly"},
                "status": {"type": "string", "enum": ["pending", "in_progress", "completed"]}
            }, "required": ["content", "status"]}}
        }, "required": ["todos"]}),
    }
}

fn parse(arguments: &Arguments) -> Result<List, String> {
    let Arguments::Json(arguments) = arguments else {
        return Err("The arguments were not valid JSON.".to_owned());
    };
    serde_json::from_value(serde_json::Value::Object(arguments.clone()))
        .map_err(|err| format!("The list was not understood: {err}."))
}

/// How far along the list is: `2/5 done, now: run the tests`.
fn progress(list: &List) -> String {
    let done = list
        .todos
        .iter()
        .filter(|item| item.status == Status::Completed)
        .count();
    let total = list.todos.len();
    match list
        .todos
        .iter()
        .find(|item| item.status == Status::InProgress)
    {
        Some(item) => format!("{done}/{total} done, now: {}", item.content),
        None => format!("{done}/{total} done"),
    }
}

impl ToolSource for Todo {
    fn tools(&self) -> BoxFuture<'_, Result<Vec<ToolInfo>, ToolError>> {
        let info = ToolInfo {
            spec: spec(),
            effect: Effect::Reads,
            parallel: false,
        };
        Box::pin(ready(Ok(vec![info])))
    }

    fn describe(&self, name: &str, arguments: &Arguments) -> String {
        match parse(arguments) {
            Ok(list) => format!("{name} {}", progress(&list)),
            Err(_) => glave_ports::describe_call(name, arguments),
        }
    }

    fn call<'a>(
        &'a self,
        name: &'a str,
        arguments: &'a Arguments,
        _cancel: &'a dyn Cancellation,
    ) -> BoxFuture<'a, ToolOutput> {
        let output = if name == TODO {
            match parse(arguments) {
                Ok(list) => {
                    ToolOutput::text(false, format!("Todo list updated: {}.", progress(&list)))
                }
                Err(err) => ToolOutput::text(true, err),
            }
        } else {
            ToolOutput::text(true, format!("There is no tool named {name:?}."))
        };
        Box::pin(ready(output))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use glave_ports::{NeverCancelled, block_on, conformance};

    fn list(value: serde_json::Value) -> Arguments {
        match value {
            serde_json::Value::Object(map) => Arguments::Json(map),
            other => panic!("not an object: {other}"),
        }
    }

    #[test]
    fn passes_the_tool_source_conformance_suite() {
        assert_eq!(block_on!(conformance::tool_source(&Todo)), Ok(()));
    }

    #[test]
    fn an_update_says_how_far_along_the_list_is() {
        let todos = list(json!({"todos": [
            {"content": "read the code", "status": "completed"},
            {"content": "fix the bug", "status": "in_progress"},
            {"content": "run the tests", "status": "pending"}
        ]}));
        let output = block_on!(Todo.call(TODO, &todos, &NeverCancelled));
        assert_eq!(
            output,
            ToolOutput::text(false, "Todo list updated: 1/3 done, now: fix the bug.")
        );
        assert_eq!(
            Todo.describe(TODO, &todos),
            "todo 1/3 done, now: fix the bug"
        );
    }

    #[test]
    fn a_list_it_cannot_read_is_an_error_that_says_why() {
        let odd = list(json!({"todos": [{"content": "x", "status": "maybe"}]}));
        let output = block_on!(Todo.call(TODO, &odd, &NeverCancelled));
        assert!(output.is_error);
        assert!(
            format!("{output:?}").contains("unknown variant `maybe`"),
            "{output:?}"
        );
    }
}
