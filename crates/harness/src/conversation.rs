//! Rebuilding the conversation a model sees from a thread's events. The log alone is enough
//! to resume a thread: nothing the model needs lives anywhere else. A compaction restarts the
//! conversation from its summary; the events before it stay in the log.

use glave_event::{Body, Event, Role};
use glave_ports::{Message, ToolCall};

/// The conversation recorded in `events`, in order, from the latest compaction on.
pub fn conversation(events: &[Event]) -> Vec<Message> {
    let mut messages: Vec<Message> = Vec::new();
    for event in events {
        match &event.body {
            Body::MessageAdded(added) => messages.push(match added.role {
                Role::User => Message::User {
                    parts: added.parts.clone(),
                },
                Role::Assistant => Message::Assistant {
                    parts: added.parts.clone(),
                    tool_calls: Vec::new(),
                },
            }),
            Body::ToolCallRequested(requested) => {
                let call = ToolCall {
                    call_id: requested.call_id.clone(),
                    tool: requested.tool.clone(),
                    arguments: requested.arguments.clone(),
                };
                match messages.last_mut() {
                    Some(Message::Assistant { tool_calls, .. }) => tool_calls.push(call),
                    _ => messages.push(Message::Assistant {
                        parts: Vec::new(),
                        tool_calls: vec![call],
                    }),
                }
            }
            Body::ToolCallCompleted(completed) => messages.push(Message::Tool {
                call_id: completed.call_id.clone(),
                is_error: completed.is_error,
                output: completed.output.clone(),
            }),
            Body::ThreadCompacted(compacted) => {
                messages.clear();
                messages.push(Message::User {
                    parts: compacted.parts.clone(),
                });
            }
            Body::ThreadStarted(_)
            | Body::TurnStarted(_)
            | Body::ModelResponded(_)
            | Body::TurnEnded(_)
            | Body::ApprovalRequested(_)
            | Body::ApprovalDecided(_)
            | Body::GoalSet(_)
            | Body::GoalEnded(_)
            | Body::GoalResumed(_)
            | Body::WorkspaceBound(_)
            | Body::WorkspaceRemoved(_)
            | Body::HookRan(_)
            | Body::Unknown(_) => {}
        }
    }
    messages
}
