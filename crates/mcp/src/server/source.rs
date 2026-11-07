//! An MCP server as a tool source: its tools, its notes to the model, each call as the person
//! reads it (a web search by what it searched, a named account's by that account), and calls.

use glave_event::{Arguments, ToolKind};
use glave_ports::{
    BoxFuture, Called, Cancellation, ServiceGiven, ToolError, ToolInfo, ToolOutput, ToolSource,
};

use super::{McpServer, State};
use crate::call::Answerer;

/// The most of a server's instructions the model is given.
const INSTRUCTION_CHARS: usize = 4_000;

impl ToolSource for McpServer {
    fn tools(&self) -> BoxFuture<'_, Result<Vec<ToolInfo>, ToolError>> {
        Box::pin(self.list())
    }

    /// Every server but web search, which is one tool and how glave reaches the web.
    fn connected(&self) -> bool {
        self.search.is_none()
    }

    fn instructions(&self) -> Option<String> {
        let State::Ready(connection) = &*self.state.borrow() else {
            return None;
        };
        let own = connection.instructions().map(|text| {
            let total = text.chars().count();
            if total > INSTRUCTION_CHARS {
                let kept: String = text.chars().take(INSTRUCTION_CHARS).collect();
                format!("{kept}\n[cut short: {total} characters]")
            } else {
                text
            }
        });
        let searching = self.search.as_deref().map(|search| {
            format!(
                "To look something up on the web, search with {search} first, then read the \
                 pages that matter with web_fetch. This server offers its search alone, whatever \
                 its notes or its tool's description name."
            )
        });
        let notes: Vec<String> = own.into_iter().chain(searching).collect();
        (!notes.is_empty()).then(|| format!("## {}\n\n{}", self.name, notes.join("\n\n")))
    }

    fn describe(&self, name: &str, arguments: &Arguments) -> String {
        match (&self.search, &self.account) {
            (Some(search), _) if search == name => searched(arguments),
            (_, Some(account)) => {
                glave_ports::describe_call(&format!("{name} ({account})"), arguments)
            }
            (_, None) => glave_ports::describe_call(name, arguments),
        }
    }

    fn kind(&self, name: &str) -> ToolKind {
        match &self.search {
            Some(search) if search == name => ToolKind::Web,
            _ => ToolKind::Other,
        }
    }

    fn prompt<'a>(
        &'a self,
        name: &'a str,
        words: &'a str,
        cancel: &'a dyn Cancellation,
    ) -> BoxFuture<'a, Option<Result<String, String>>> {
        Box::pin(self.prompt_text(name, words, cancel))
    }

    fn call<'a>(
        &'a self,
        name: &'a str,
        arguments: &'a Arguments,
        cancel: &'a dyn Cancellation,
    ) -> BoxFuture<'a, ToolOutput> {
        Box::pin(async move {
            match self.run(name, arguments, Answerer::NoOne, cancel).await {
                Called::Done(output) => output,
                // With no one to answer, it never asks.
                Called::Asks(asks) => {
                    ToolOutput::text(true, format!("{} asked for more input.", asks.service))
                }
            }
        })
    }

    fn call_asking<'a>(
        &'a self,
        name: &'a str,
        arguments: &'a Arguments,
        given: Option<ServiceGiven>,
        cancel: &'a dyn Cancellation,
    ) -> BoxFuture<'a, Called> {
        Box::pin(self.run(name, arguments, Answerer::Person(given), cancel))
    }
}

/// The most of a search told to the person.
const SEARCHED_CHARS: usize = 120;

/// A web search's call as the person reads it: what was searched, its `query` or else its first
/// text, on one line.
fn searched(arguments: &Arguments) -> String {
    let asked = match arguments {
        Arguments::Json(map) => map
            .get("query")
            .or_else(|| map.values().find(|value| value.is_string()))
            .and_then(|value| value.as_str())
            .unwrap_or_default(),
        Arguments::Malformed(raw) => raw.as_str(),
    };
    let line = asked.lines().next().unwrap_or_default().trim();
    let mut short: String = line.chars().take(SEARCHED_CHARS).collect();
    if line.chars().count() > SEARCHED_CHARS {
        short.push('…');
    }
    format!("Searched the web: {short}")
}
