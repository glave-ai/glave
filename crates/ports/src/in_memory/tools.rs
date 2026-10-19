//! Tools held in memory, for tests and for runs that keep nothing.

use std::sync::Mutex;

/// Tools held in memory: each answers every call with a fixed output, and every call is
/// remembered. The in-memory adapter of the tool source port.
pub struct InMemoryTools {
    tools: Vec<(crate::tools::ToolInfo, crate::tools::ToolOutput)>,
    calls: Mutex<Vec<(String, glave_event::Arguments)>>,
}

impl InMemoryTools {
    pub fn new(tools: Vec<(crate::tools::ToolInfo, crate::tools::ToolOutput)>) -> Self {
        Self {
            tools,
            calls: Mutex::new(Vec::new()),
        }
    }

    /// Every call so far, in order: the tool's name and its arguments.
    pub fn calls(&self) -> Vec<(String, glave_event::Arguments)> {
        self.calls
            .lock()
            .map(|calls| calls.clone())
            .unwrap_or_default()
    }
}

impl crate::tools::ToolSource for InMemoryTools {
    fn tools(
        &self,
    ) -> crate::tools::BoxFuture<'_, Result<Vec<crate::tools::ToolInfo>, crate::tools::ToolError>>
    {
        let tools = self.tools.iter().map(|(info, _)| info.clone()).collect();
        Box::pin(std::future::ready(Ok(tools)))
    }

    fn call<'a>(
        &'a self,
        name: &'a str,
        arguments: &'a glave_event::Arguments,
        _cancel: &'a dyn crate::tools::Cancellation,
    ) -> crate::tools::BoxFuture<'a, crate::tools::ToolOutput> {
        let answer = self
            .tools
            .iter()
            .find(|(info, _)| info.spec.name == name)
            .map(|(_, output)| output.clone());
        if answer.is_some()
            && let Ok(mut calls) = self.calls.lock()
        {
            calls.push((name.to_owned(), arguments.clone()));
        }
        let output = answer.unwrap_or_else(|| {
            crate::tools::ToolOutput::text(true, format!("There is no tool named {name:?}."))
        });
        Box::pin(std::future::ready(output))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{block_on, conformance};

    fn tool(name: &str) -> (crate::tools::ToolInfo, crate::tools::ToolOutput) {
        let info = crate::tools::ToolInfo {
            spec: crate::model::ToolSpec {
                name: name.into(),
                description: "A tool.".into(),
                parameters: serde_json::json!({"type": "object"}),
            },
            effect: crate::tools::Effect::Reads,
            parallel: true,
        };
        (info, crate::tools::ToolOutput::text(false, "ok"))
    }

    #[test]
    fn in_memory_tools_pass_the_tool_source_conformance_suite() {
        let tools = InMemoryTools::new(vec![tool("lookup"), tool("mcp__docs__search")]);
        assert_eq!(block_on!(conformance::tool_source(&tools)), Ok(()));
    }

    #[test]
    fn the_tool_source_suite_catches_bad_sources() {
        let twice = InMemoryTools::new(vec![tool("lookup"), tool("lookup")]);
        assert!(block_on!(conformance::tool_source(&twice)).is_err());
        let projectd = InMemoryTools::new(vec![tool("look up")]);
        assert!(block_on!(conformance::tool_source(&projectd)).is_err());
    }
}
