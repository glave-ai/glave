//! An MCP server as a tool source: its tools, its notes to the model, each call as the person
//! reads it (a named account's by that account), and calls.

use glave_event::Arguments;
use glave_ports::{
    BoxFuture, Called, Cancellation, Naming, ServiceGiven, ToolError, ToolInfo, ToolOutput,
    ToolSource,
};

use super::{McpServer, State};
use crate::call::Answerer;

/// The most of a server's instructions the model is given.
const INSTRUCTION_CHARS: usize = 4_000;

impl ToolSource for McpServer {
    fn tools(&self) -> BoxFuture<'_, Result<Vec<ToolInfo>, ToolError>> {
        Box::pin(self.list())
    }

    fn connected(&self) -> bool {
        true
    }

    fn instructions(&self) -> Option<String> {
        let State::Ready(connection) = &*self.state.borrow() else {
            return None;
        };
        let text = connection.instructions()?;
        let total = text.chars().count();
        let text = if total > INSTRUCTION_CHARS {
            let kept: String = text.chars().take(INSTRUCTION_CHARS).collect();
            format!("{kept}\n[cut short: {total} characters]")
        } else {
            text
        };
        Some(format!("## {}\n\n{text}", self.name))
    }

    fn describe(&self, name: &str, arguments: &Arguments) -> String {
        match &self.account {
            Some(account) => glave_ports::describe_call(&format!("{name} ({account})"), arguments),
            None => glave_ports::describe_call(name, arguments),
        }
    }

    /// Every key of the call's arguments, its account named as in its line.
    fn describe_in_full(&self, name: &str, arguments: &Arguments) -> String {
        match &self.account {
            Some(account) => {
                glave_ports::describe_call_in_full(&format!("{name} ({account})"), arguments)
            }
            None => glave_ports::describe_call_in_full(name, arguments),
        }
    }

    fn naming(&self, name: &str) -> Naming {
        let State::Ready(connection) = &*self.state.borrow() else {
            return Naming::default();
        };
        Naming {
            title: connection.title_of(name),
            service: self.known_as.clone(),
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
