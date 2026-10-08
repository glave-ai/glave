//! Continuing a recorded thread from the log alone, closing anything a crash left open.

use std::collections::HashSet;

use glave_event::{
    Body, CallId, Event, Outcome, Part, Seq, ThreadId, ToolCallCompleted, TurnEnded, decode,
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
            allowed: HashSet::new(),
        };
        self.repair(&mut thread, &events).await?;
        Ok(thread)
    }

    async fn load(&self, scope: &Scope, id: ThreadId) -> Result<Vec<Event>, HarnessError> {
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
