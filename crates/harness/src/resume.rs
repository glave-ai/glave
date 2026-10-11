//! Continuing a recorded thread from the log alone, closing anything a crash left open.

use std::collections::{HashMap, HashSet};

use glave_event::{
    Body, CallId, Event, Outcome, Part, Seq, ThreadId, ToolCallCompleted, TurnEnded, Verdict,
    decode,
};
use glave_ports::{BlobStore, EventLog, Message, ModelProvider, Scope};

use crate::approval::Approver;
use crate::conversation::conversation;
use crate::harness::{Harness, HarnessError, Thread};

/// Events read per page while loading a thread.
const PAGE: usize = 1_000;

const INTERRUPTED: &str = "[interrupted: glave stopped before this call finished]";

impl<P, L, B, A> Harness<P, L, B, A>
where
    P: ModelProvider,
    L: EventLog,
    B: BlobStore,
    A: Approver,
{
    /// Continues thread `id`: rebuilds the conversation from its events, and records an
    /// interrupted result for any tool call and an ending for any turn a crash left open, so
    /// the conversation is valid to send again.
    pub async fn resume_thread(&self, scope: Scope, id: ThreadId) -> Result<Thread, HarnessError> {
        let events = self.load(&scope, id).await?;
        let last = events.last().ok_or(HarnessError::NotFound(id))?;
        let next_seq = last
            .header
            .seq
            .next()
            .ok_or_else(|| HarnessError::Event(format!("thread {id} is full")))?;
        let mut thread = Thread {
            scope,
            id,
            next_seq,
            messages: conversation(&events),
            allowed: allowed_for_thread(&events),
            measured: None,
            request: latest_request(&events),
            asked: asked(&events),
            above: above(&events),
            goal: crate::goal::latest(&events).map(crate::goal::Pursuit::of),
            workspace: self.reopen(scope, id, &events).await?,
            unnumbered: crate::bind::own_records(&events),
            standing: crate::standing::Standing::of(&events),
        };
        self.repair(&mut thread, &events).await?;
        Ok(thread)
    }

    pub(crate) async fn load(
        &self,
        scope: &Scope,
        id: ThreadId,
    ) -> Result<Vec<Event>, HarnessError> {
        let mut events = Vec::new();
        let mut after: Option<Seq> = None;
        loop {
            let page = self.log.read(scope, id, after, PAGE).await?;
            let full = page.len() == PAGE;
            after = page.last().map(|event| event.seq).or(after);
            for raw in page {
                events.push(decode(raw).map_err(|e| HarnessError::Event(e.to_string()))?);
            }
            if !full {
                return Ok(events);
            }
        }
    }

    async fn repair(&self, thread: &mut Thread, events: &[Event]) -> Result<(), HarnessError> {
        let completed: HashSet<&CallId> = events
            .iter()
            .filter_map(|event| match &event.body {
                Body::ToolCallCompleted(done) => Some(&done.call_id),
                _ => None,
            })
            .collect();
        let dangling: Vec<CallId> = events
            .iter()
            .filter_map(|event| match &event.body {
                Body::ToolCallRequested(call) if !completed.contains(&call.call_id) => {
                    Some(call.call_id.clone())
                }
                _ => None,
            })
            .collect();
        let open_turn = events
            .iter()
            .rev()
            .find_map(|event| match event.body {
                Body::TurnStarted(_) => Some(true),
                Body::TurnEnded(_) => Some(false),
                _ => None,
            })
            .unwrap_or(false);
        if dangling.is_empty() && !open_turn {
            return Ok(());
        }
        let output = vec![Part::Text {
            text: INTERRUPTED.to_owned(),
        }];
        let mut staged = Vec::new();
        for call_id in &dangling {
            let completed = ToolCallCompleted {
                call_id: call_id.clone(),
                is_error: true,
                output: output.clone(),
                structured: None,
            };
            self.stage(thread, &mut staged, &completed)?;
        }
        if open_turn {
            let ended = TurnEnded {
                outcome: Outcome::Interrupted,
            };
            self.stage(thread, &mut staged, &ended)?;
        }
        self.commit(thread, staged).await?;
        for call_id in dangling {
            thread.messages.push(Message::Tool {
                call_id,
                is_error: true,
                output: output.clone(),
            });
        }
        Ok(())
    }
}

/// The person's own messages, oldest first: the last text of each message they added (a new
/// thread's first message opens with the project's instructions). A subagent's thread has
/// none of its own: its prompts came from the parent's agent.
fn asked(events: &[Event]) -> Vec<String> {
    if !above(events).is_empty() {
        return Vec::new();
    }
    events
        .iter()
        .filter_map(|event| match &event.body {
            Body::MessageAdded(added) if by_person(added) => {
                added.parts.iter().rev().find_map(|part| match part {
                    Part::Text { text } => Some(text.clone()),
                    _ => None,
                })
            }
            _ => None,
        })
        .collect()
}

/// Whether a call in another thread started this one.
/// The thread that started this one, if an `agent` call did. Subagents start no subagents of
/// their own, so there is never more than one.
fn above(events: &[Event]) -> Vec<ThreadId> {
    events
        .iter()
        .filter_map(|event| match &event.body {
            Body::ThreadStarted(started) => started.parent.as_ref().map(|parent| parent.thread),
            _ => None,
        })
        .take(1)
        .collect()
}

/// The text of the person's latest message, kept word for word through a compaction.
fn latest_request(events: &[Event]) -> Option<String> {
    events.iter().rev().find_map(|event| match &event.body {
        Body::MessageAdded(added) if by_person(added) => {
            let texts: Vec<&str> = added
                .parts
                .iter()
                .filter_map(|part| match part {
                    Part::Text { text } => Some(text.as_str()),
                    _ => None,
                })
                .collect();
            Some(texts.join("\n"))
        }
        _ => None,
    })
}

/// Whether a message is the person's own: not the model's, and not one glave wrote, such as
/// a goal's continuation.
fn by_person(added: &glave_event::MessageAdded) -> bool {
    added.role == glave_event::Role::User
        && added
            .author
            .as_ref()
            .is_none_or(|author| author.kind != glave_event::PrincipalKind::Glave)
}

/// The tools the person allowed for the rest of the thread, from its recorded approvals.
fn allowed_for_thread(events: &[Event]) -> HashSet<String> {
    let mut asked: HashMap<&CallId, &str> = HashMap::new();
    let mut allowed = HashSet::new();
    for event in events {
        match &event.body {
            Body::ApprovalRequested(requested) => {
                asked.insert(&requested.call_id, requested.tool.as_str());
            }
            Body::ApprovalDecided(decided) if decided.decision == Verdict::AllowForThread => {
                if let Some(tool) = asked.get(&decided.call_id) {
                    allowed.insert((*tool).to_owned());
                }
            }
            _ => {}
        }
    }
    allowed
}
