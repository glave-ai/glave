//! A service whose tool asks the person things, for tests of what a turn does when it does.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex, PoisonError};

use glave_event::{Arguments, ServiceField, ServiceFieldKind};
use serde_json::json;

use crate::model::ToolSpec;
use crate::tools::{
    BoxFuture, Called, Cancellation, Effect, ServiceAsks, ServiceGiven, ToolError, ToolInfo,
    ToolOutput, ToolSource,
};

/// The one tool of an [`AskingTool`], as the model calls it.
pub const ASKING_TOOL: &str = "mcp__github__file";

/// A tool that asks, in turn, each of what it was given, on behalf of the service GitHub, for one
/// required text field, `note`; then answers "filed". With no one to ask, it answers at once.
#[derive(Default)]
pub struct AskingTool {
    asks: Mutex<VecDeque<String>>,
    given: Mutex<Vec<Option<ServiceGiven>>>,
    unasked: Mutex<u32>,
}

impl AskingTool {
    /// One that asks each of `asks`, in turn.
    pub fn asking(asks: &[&str]) -> Arc<Self> {
        let tool = Self::default();
        *tool.asks.lock().unwrap_or_else(PoisonError::into_inner) =
            asks.iter().map(|ask| (*ask).to_owned()).collect();
        Arc::new(tool)
    }

    /// What each call was given, where the person could be asked.
    pub fn given(&self) -> Vec<Option<ServiceGiven>> {
        self.given
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// How many calls came with no one to ask.
    pub fn unasked(&self) -> u32 {
        *self.unasked.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl ToolSource for AskingTool {
    fn tools(&self) -> BoxFuture<'_, Result<Vec<ToolInfo>, ToolError>> {
        let info = ToolInfo {
            spec: ToolSpec {
                name: ASKING_TOOL.to_owned(),
                description: "Files an issue.".to_owned(),
                parameters: json!({"type": "object"}),
            },
            effect: Effect::Runs,
            parallel: false,
        };
        Box::pin(std::future::ready(Ok(vec![info])))
    }

    fn call<'a>(
        &'a self,
        _name: &'a str,
        _arguments: &'a Arguments,
        _cancel: &'a dyn Cancellation,
    ) -> BoxFuture<'a, ToolOutput> {
        let mut unasked = self.unasked.lock().unwrap_or_else(PoisonError::into_inner);
        *unasked = unasked.saturating_add(1);
        Box::pin(std::future::ready(ToolOutput::text(false, "filed unasked")))
    }

    fn call_asking<'a>(
        &'a self,
        _name: &'a str,
        _arguments: &'a Arguments,
        given: Option<ServiceGiven>,
        _cancel: &'a dyn Cancellation,
    ) -> BoxFuture<'a, Called> {
        self.given
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(given);
        let next = self
            .asks
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front();
        Box::pin(std::future::ready(match next {
            Some(message) => Called::Asks(ServiceAsks {
                service: "GitHub".to_owned(),
                message,
                fields: vec![ServiceField {
                    name: "note".to_owned(),
                    label: "Note".to_owned(),
                    description: None,
                    required: true,
                    kind: ServiceFieldKind::Text { default: None },
                }],
                state: json!({"round": "kept"}),
            }),
            None => Called::Done(ToolOutput::text(false, "filed")),
        }))
    }
}
