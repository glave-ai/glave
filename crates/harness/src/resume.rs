//! Continuing a recorded thread from the log alone, which reads and never writes; and closing
//! what a turn left open when the glave running it stopped without ending it, which only the
//! holder of the thread's lease does (`specs/one-server.md`, "After a restart").

use std::collections::{HashMap, HashSet};

use glave_event::{Body, CallId, Event, Outcome, Part, Seq, ThreadId, TurnEnded, Verdict, decode};
use glave_ports::{BlobStore, CallKind, EventLog, ModelProvider, Scope, ToolCall, ToolOutput};

use crate::approval::Approver;
use crate::conversation::conversation;
use crate::harness::{Harness, HarnessError, Thread};
use crate::waits::Opened;

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
    /// Continues thread `id`: rebuilds the conversation from its events, recording nothing. A
    /// turn a glave left open stays as the log has it ([`Thread::left_open`]) until
    /// [`Harness::repair`] closes it, which work on the thread does first; one that waits
    /// ([`Thread::waits`]) goes on once its answer comes ([`Harness::go_on`]).
    pub async fn resume_thread(&self, scope: Scope, id: ThreadId) -> Result<Thread, HarnessError> {
        self.resume_thread_in(scope, id, None).await
    }

    /// Continues thread `id` as [`Harness::resume_thread`] does, where its project's folder is
    /// now `folder`: a thread whose own folder isn't there anymore works there from now on,
    /// which its next turn records (`workspace.bound`).
    pub async fn resume_thread_in(
        &self,
        scope: Scope,
        id: ThreadId,
        folder: Option<&str>,
    ) -> Result<Thread, HarnessError> {
        let events = self.load(&scope, id).await?;
        let last = events.last().ok_or(HarnessError::NotFound(id))?;
        let next_seq = last
            .header
            .seq
            .next()
            .ok_or_else(|| HarnessError::Event(format!("thread {id} is full")))?;
        let (workspace, bound, moved) = self.reopen(scope, id, &events, folder).await?;
        let for_ = for_of(&events).or(self.actors.person);
        let instructions = self
            .instructions_for(workspace.as_deref(), bound.as_ref(), for_)
            .await;
        let project = bound
            .as_ref()
            .filter(|_| workspace.is_some())
            .and_then(|bound| crate::project_folders::project_of(&bound.opened));
        let computers = workspace.as_ref().unwrap_or(&self.workspace).computers();
        let commands = crate::background::Commands::rebuilt(&events, computers);
        let mut thread = Thread {
            scope,
            id,
            next_seq,
            messages: conversation(&events),
            allowed: allowed_for_thread(&events),
            measured: None,
            request: latest_request(&events),
            asked: asked(&events),
            turns: turns_begun(&events),
            above: above(&events),
            goal: crate::goal::latest(&events).map(crate::goal::Pursuit::of),
            workspace,
            unnumbered: crate::bind::own_records(&events),
            note: None,
            standing: crate::standing::standing_after(glave_ports::Standing::Active, &events),
            removed: false,
            untrusted: read_untrusted(&events),
            read_secret: read_secret(&events),
            memory: crate::memory::MemoryState::default(),
            for_,
            model: crate::chosen::chosen_in(&events),
            permissions: crate::chosen::permissions_in(&events),
            effort: crate::chosen::effort_in(&events),
            instructions,
            project,
            brought: std::sync::Arc::default(),
            defaults: self.defaults(),
            inbox: crate::inbox::Inbox::default(),
            commands: std::sync::Arc::new(commands),
            opened: self.opened(&scope, id, &events).await?,
            rebound: bound.filter(|_| moved),
        };
        self.rebuild_memory(&mut thread, &events).await;
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

    /// Closes what a turn left open in thread `id`, as its log has it now, when it does not
    /// wait (one answered and not gone on with is a crash's leftover here): each call with no
    /// result gets the interrupted result, but a subagent's call its
    /// subagent's report, the subagent's own turn closed first; then the turn ends
    /// `interrupted`. Records nothing when nothing was left open, and says whether it
    /// recorded. Only for the holder of the thread's lease, which keeps any other glave from
    /// running the turn meanwhile; a thread held in memory is read again after.
    pub async fn repair(&self, scope: Scope, id: ThreadId) -> Result<bool, HarnessError> {
        Box::pin(self.repair_now(scope, id)).await
    }

    async fn repair_now(&self, scope: Scope, id: ThreadId) -> Result<bool, HarnessError> {
        let events = self.load(&scope, id).await?;
        if matches!(
            self.opened(&scope, id, &events).await?,
            None | Some(Opened::Waits)
        ) {
            return Ok(false);
        }
        let next = events
            .last()
            .and_then(|last| last.header.seq.next())
            .ok_or_else(|| HarnessError::Event(format!("thread {id} is full")))?;
        let open = crate::waits::open_turn(&events);
        let mut staged = Vec::new();
        for call in dangling_calls(&events) {
            let output = match self.child_of(&scope, id, &call.call_id).await? {
                Some(child) if CallKind::of(&call.tool) == CallKind::Agent => {
                    self.repair(scope, child.thread).await?;
                    self.reported(&scope, child.thread).await?.0
                }
                _ => ToolOutput::text(true, INTERRUPTED),
            };
            let completed = crate::calls::completed(call.call_id, output, None);
            self.stage_next(id, next, &mut staged, &completed)?;
        }
        if open.is_some() {
            let ended = TurnEnded {
                outcome: Outcome::Interrupted,
            };
            self.stage_next(id, next, &mut staged, &ended)?;
        }
        self.log.append(&scope, &staged).await?;
        Ok(true)
    }

    /// Closes what a turn left open in `thread`, as [`Harness::repair`] does, and reads its
    /// conversation again after: before any work on it, which only the holder of its lease
    /// runs. A turn that waits is left to go on ([`Harness::go_on`]).
    pub(crate) async fn close_left_open(&self, thread: &mut Thread) -> Result<(), HarnessError> {
        if !matches!(thread.opened, Some(Opened::LeftOpen(_))) {
            return Ok(());
        }
        self.repair(thread.scope, thread.id).await?;
        let events = self.load(&thread.scope, thread.id).await?;
        let last = events.last().ok_or(HarnessError::NotFound(thread.id))?;
        thread.next_seq = last
            .header
            .seq
            .next()
            .ok_or_else(|| HarnessError::Event(format!("thread {} is full", thread.id)))?;
        thread.messages = conversation(&events);
        thread.opened = self.opened(&thread.scope, thread.id, &events).await?;
        Ok(())
    }
}

/// The calls in `events` with no result, in order.
fn dangling_calls(events: &[Event]) -> Vec<ToolCall> {
    let completed: HashSet<&CallId> = events
        .iter()
        .filter_map(|event| match &event.body {
            Body::ToolCallCompleted(done) => Some(&done.call_id),
            _ => None,
        })
        .collect();
    events
        .iter()
        .filter_map(|event| match &event.body {
            Body::ToolCallRequested(call) if !completed.contains(&call.call_id) => Some(ToolCall {
                call_id: call.call_id.clone(),
                tool: call.tool.clone(),
                arguments: call.arguments.clone(),
            }),
            _ => None,
        })
        .collect()
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

/// How many turns `events` begin.
fn turns_begun(events: &[Event]) -> u64 {
    let begun = events
        .iter()
        .filter(|event| matches!(event.body, Body::TurnStarted(_)))
        .count();
    u64::try_from(begun).unwrap_or(u64::MAX)
}

/// The person a thread is for, as `thread.started` records it; `None` when it records none,
/// which means the local person.
fn for_of(events: &[Event]) -> Option<glave_event::PrincipalId> {
    events.iter().find_map(|event| match &event.body {
        Body::ThreadStarted(started) => started
            .for_
            .as_ref()
            .filter(|principal| principal.kind == glave_event::PrincipalKind::Person)
            .and_then(|principal| principal.id),
        _ => None,
    })
}

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

/// Whether the thread has read untrusted content: any call's output marked so.
fn read_untrusted(events: &[Event]) -> bool {
    events.iter().any(|event| match &event.body {
        Body::ToolCallCompleted(done) => done.untrusted,
        _ => false,
    })
}

/// Whether the model has been shown a file that usually holds secrets: an approval for one
/// that was allowed.
fn read_secret(events: &[Event]) -> bool {
    let mut asked: HashSet<&CallId> = HashSet::new();
    events.iter().any(|event| match &event.body {
        Body::ApprovalRequested(requested) => {
            if requested.secret {
                asked.insert(&requested.call_id);
            }
            false
        }
        Body::ApprovalDecided(decided) => {
            asked.contains(&decided.call_id)
                && matches!(
                    decided.decision,
                    Verdict::AllowOnce | Verdict::AllowForThread
                )
        }
        _ => false,
    })
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
