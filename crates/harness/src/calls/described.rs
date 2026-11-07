//! A call as the person sees it: in a line, and by the kind of work it does, as the tool it
//! stands for says.

use glave_event::ToolKind;
use glave_ports::{BlobStore, EventLog, ModelProvider, ToolCall, describe_call};

use crate::approval::Approver;
use crate::harness::{Harness, Thread};
use crate::toolset::{Route, TOOL_SEARCH, ToolSet};

impl<P, L, B, A> Harness<P, L, B, A>
where
    P: ModelProvider,
    L: EventLog,
    B: BlobStore,
    A: Approver,
{
    /// `call` in a line, for the person (`bash: cargo test`), as `tools` offer it: what its
    /// `tool.call.requested` records, and what its live update and an approval of it say, each
    /// by the same description: its source's, or glave's own tool's.
    pub(crate) fn describe(&self, thread: &Thread, tools: &ToolSet, call: &ToolCall) -> String {
        let call = match tools.route(call) {
            Route::Run(call) => call,
            Route::Search { query, connector } => {
                let within = connector.map(|c| format!(" in {c}")).unwrap_or_default();
                return format!("{TOOL_SEARCH} {}{within}", query.unwrap_or_default());
            }
            Route::Refused(_) => call.clone(),
        };
        let (name, arguments) = (call.tool.as_str(), &call.arguments);
        if tools.is_plan(name) {
            return crate::plan::describe(arguments);
        } else if tools.is_memory(name) {
            return crate::memory::described(name, arguments);
        } else if tools.is_agent(name) {
            return crate::subagent::describe(arguments);
        }
        let described = tools.get(name).and_then(|(index, info)| {
            let source = self.source_for(thread, tools, index)?;
            Some(source.describe(&info.spec.name, arguments))
        });
        described.unwrap_or_else(|| describe_call(name, arguments))
    }

    /// What kind of work `call` does, as its source says: glave's own plan, memory and
    /// subagent tools, and a tool not offered, are `other`.
    pub(crate) fn kind(&self, thread: &Thread, tools: &ToolSet, call: &ToolCall) -> ToolKind {
        let Route::Run(call) = tools.route(call) else {
            return ToolKind::Other;
        };
        let name = call.tool.as_str();
        if tools.is_plan(name) || tools.is_memory(name) || tools.is_agent(name) {
            return ToolKind::Other;
        }
        tools
            .get(name)
            .and_then(|(index, info)| {
                Some(self.source_for(thread, tools, index)?.kind(&info.spec.name))
            })
            .unwrap_or_default()
    }
}
