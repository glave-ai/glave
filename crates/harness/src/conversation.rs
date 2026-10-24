//! Rebuilding the conversation a model sees from a thread's events. The log alone is enough
//! to resume a thread: nothing the model needs lives anywhere else. A compaction restarts the
//! conversation from its summary; the events before it stay in the log.

use glave_event::{Body, Event, Role, ToolCallRequested};
use glave_ports::{Message, ToolCall};

/// The conversation recorded in `events`, in order, from the latest compaction on.
pub fn conversation(events: &[Event]) -> Vec<Message> {
    let mut messages: Vec<Message> = Vec::new();
    for event in events {
        follow(&mut messages, event);
    }
    messages
}

/// Takes one event into the conversation built so far.
pub(crate) fn follow(messages: &mut Vec<Message>, event: &Event) {
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
        Body::ToolCallRequested(requested) => call(messages, requested),
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
        | Body::CommandStarted(_)
        | Body::CommandEnded(_)
        | Body::ThreadArchived(_)
        | Body::ThreadUnarchived(_)
        | Body::ThreadDeleted(_)
        | Body::ThreadRenamed(_)
        | Body::ModelChosen(_)
        | Body::ApprovalModeChosen(_)
        | Body::EffortChosen(_)
        | Body::HookRan(_)
        | Body::TaskStarted(_)
        | Body::TaskEnded(_)
        | Body::ScheduleCreated(_)
        | Body::ScheduleDeleted(_)
        | Body::GatewayToolCalled(_)
        | Body::MemorySaved(_)
        | Body::MemoryEdited(_)
        | Body::MemoryForgotten(_)
        | Body::MemoryRestored(_)
        | Body::MemoryTidied(_)
        | Body::ProviderConnected(_)
        | Body::ProviderRemoved(_)
        | Body::CredentialConnected(_)
        | Body::CredentialRemoved(_)
        | Body::ProjectFolderSet(_)
        | Body::ProjectRemoved(_)
        | Body::TrustDecided(_)
        | Body::BrowserSignedIn(_)
        | Body::BrowserUsed(_)
        | Body::BrowserSignedOut(_)
        | Body::CatalogRefreshed(_)
        | Body::AcmeRegistered(_)
        | Body::CertificateIssued(_)
        | Body::PasskeyAdded(_)
        | Body::PasskeyUsed(_)
        | Body::PasskeyRemoved(_)
        | Body::AwakeSet(_)
        | Body::LanguageSet(_)
        | Body::GroupingSet(_)
        | Body::Unknown(_) => {}
    }
}

/// Takes a tool call the model asked for into the conversation: with the rest of its
/// response's calls, or as an answer of its own.
fn call(messages: &mut Vec<Message>, requested: &ToolCallRequested) {
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
