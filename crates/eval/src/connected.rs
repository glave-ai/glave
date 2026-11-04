//! A task's crowd of connected tools, when it has a `connected.md`: a hundred tools of a made-up
//! workplace service, too many for the model's list, among them the one that answers with the
//! file's text, so the answer can only come from finding it (#247). In-process, since what is
//! measured is the harness's search and not the protocol.

use std::future::ready;
use std::sync::Arc;

use glave_event::Arguments;
use glave_ports::{
    BoxFuture, Cancellation, Effect, ToolError, ToolInfo, ToolOutput, ToolSource, ToolSpec,
};
use serde_json::json;

use crate::error::EvalError;
use crate::task::Task;

/// The one tool that answers.
const ANSWERS: &str = "mcp__workplace__lookup_on_call_engineer";

const VERBS: [&str; 10] = [
    "list", "get", "create", "update", "delete", "archive", "restore", "export", "share", "tag",
];
const NOUNS: [&str; 10] = [
    "invoice", "contract", "ticket", "customer", "expense", "vendor", "asset", "shipment",
    "campaign", "survey",
];

/// Workplace's tools, the one that answers somewhere among them.
struct Crowd {
    answer: String,
}

/// What a real service's definition says, near enough in length: GitHub's run about a hundred
/// tokens and more each.
fn spec(name: String, summary: &str) -> ToolSpec {
    ToolSpec {
        description: format!(
            "{summary}. Works on the records of the workplace service the person's company uses, \
             as the signed-in person may see them. Results come back newest first, a page at a \
             time; pass the cursor from one page to get the next. Dates are ISO 8601, in UTC \
             unless the record says otherwise. A record archived more than a year ago is not \
             included unless the filter asks for it. Fails with a message saying why when the \
             person has no access, and changes nothing when it fails."
        ),
        parameters: json!({"type": "object", "properties": {
            "id": {"type": "string", "description": "The record's id, when it is about one record"},
            "filter": {"type": "string", "description": "Which records, in the service's filter syntax"},
            "cursor": {"type": "string", "description": "Where the last page ended"},
            "limit": {"type": "integer", "description": "The most records in one page"}
        }}),
        name,
    }
}

fn specs() -> Vec<ToolSpec> {
    let crowd = VERBS.iter().flat_map(|verb| {
        NOUNS.iter().map(move |noun| {
            spec(
                format!("mcp__workplace__{verb}_{noun}"),
                &format!("{verb} a {noun}"),
            )
        })
    });
    let mut specs: Vec<ToolSpec> = crowd.collect();
    let middle = specs.len() / 2;
    let answers = spec(
        ANSWERS.to_owned(),
        "Find which engineer is on call for a service this week",
    );
    specs.insert(middle, answers);
    specs
}

impl ToolSource for Crowd {
    fn tools(&self) -> BoxFuture<'_, Result<Vec<ToolInfo>, ToolError>> {
        let tools = specs().into_iter().map(|spec| ToolInfo {
            spec,
            effect: Effect::Reads,
            parallel: true,
        });
        Box::pin(ready(Ok(tools.collect())))
    }

    fn connected(&self) -> bool {
        true
    }

    fn call<'a>(
        &'a self,
        name: &'a str,
        _arguments: &'a Arguments,
        _cancel: &'a dyn Cancellation,
    ) -> BoxFuture<'a, ToolOutput> {
        let text = if name == ANSWERS {
            self.answer.as_str()
        } else {
            "No records match."
        };
        Box::pin(ready(ToolOutput::text(false, text)))
    }
}

/// The crowd for `task`; none when it has no `connected.md`.
pub(crate) async fn connection(task: &Task) -> Result<Option<Arc<dyn ToolSource>>, EvalError> {
    let Some(file) = task.connected_answer() else {
        return Ok(None);
    };
    let answer = tokio::fs::read_to_string(&file)
        .await
        .map_err(EvalError::io("read", &file))?;
    Ok(Some(Arc::new(Crowd { answer })))
}

#[cfg(test)]
mod tests {
    use super::*;
    use glave_ports::NeverCancelled;

    #[test]
    fn the_crowd_is_past_a_tenth_of_the_usual_window_and_one_tool_answers() {
        let specs = specs();
        assert_eq!(specs.len(), 101);
        let tokens = serde_json::to_string(&specs).unwrap().len() / 4;
        assert!(
            tokens > 12_800,
            "{tokens} tokens: nothing would be deferred"
        );
        let crowd = Crowd {
            answer: "the answer".to_owned(),
        };
        let none = Arguments::Json(serde_json::Map::new());
        glave_ports::block_on!(async {
            let cancel = NeverCancelled;
            let answered = crowd.call(ANSWERS, &none, &cancel).await;
            assert_eq!(answered, ToolOutput::text(false, "the answer"));
            let other = crowd
                .call("mcp__workplace__get_ticket", &none, &cancel)
                .await;
            assert_eq!(other, ToolOutput::text(false, "No records match."));
        });
    }
}
