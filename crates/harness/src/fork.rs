//! Forking a thread (`specs/session.md`, "Forking a thread"): a new thread in the same project
//! whose history is a copy of another's up to a point between its turns, so a turn that went
//! wrong is tried again, or an earlier message sent again changed, as Claude Code's
//! `--fork-session` and Codex's fork copy a session. The other thread is only read. Each copy
//! keeps what its event said and when, so the new thread's model calls open with the prefix
//! its copies' last turn recorded and the provider's cache still matches (`specs/invariants.md`,
//! 27). Files are never rewound: `thread.forked`, after the copies, says where the new thread
//! works on them.

use std::collections::BTreeSet;

use glave_event::{
    Body, Event, EventId, ForkFiles, ForkPoint, Header, Origin, OriginKind, RawEvent, Seq, Sha256,
    ThreadForked, ThreadId, decode,
};
use glave_ports::{BlobStore, Caller, EventLog, ModelProvider, Scope};

use crate::approval::Approver;
use crate::bind::Place;
use crate::harness::{Harness, HarnessError, Invalid, Thread};

/// What a fork is made from: the thread, the point between its turns it holds up to and
/// including (its latest when absent), and what the person is told of its files.
pub struct Fork {
    pub from: ThreadId,
    pub at: Option<Seq>,
    /// The sentence `thread.forked` records, in English, for how the fork stands as to files.
    pub told: fn(ForkFiles) -> String,
}

impl<P, L, B, A> Harness<P, L, B, A>
where
    P: ModelProvider,
    L: EventLog,
    B: BlobStore,
    A: Approver,
{
    /// Forks `fork.from` into a new thread for `caller`, working at `place`: written in one
    /// batch, its start and where it works, the copies, then `thread.forked`. Every blob the
    /// copies name is referred to by the new thread first, so it lives as long as the copies
    /// do. Refused ([`Invalid::ForkPoint`]) at a point within a turn or past the end, and for
    /// a subagent's thread ([`Invalid::ForkSubagent`]).
    pub async fn fork_thread(
        &self,
        scope: Scope,
        caller: &Caller,
        fork: Fork,
        place: Place,
    ) -> Result<Thread, HarnessError> {
        let events = Box::pin(self.from(&scope, fork.from, None)).await?;
        let at = between_turns(&events, fork.at)?;
        let origin = Origin {
            kind: OriginKind::Fork,
            reference: Some(fork.from.to_string()),
            by: None,
        };
        let (mut thread, mut staged) = self
            .staged_start(scope, None, origin, caller, Some(place))
            .await?;
        let mut history = decoded(&staged)?;
        let files = if root_of(&history) == root_of(&events) {
            ForkFiles::Same
        } else {
            ForkFiles::Other
        };
        let copies: Vec<Event> = events
            .into_iter()
            .take_while(|event| event.header.seq <= at)
            .collect();
        let commands = commands_used(&copies);
        let mut named = BTreeSet::new();
        for copy in copies.into_iter().filter(copied) {
            let header = Header {
                id: EventId::new(self.clock.now(), self.entropy.bytes()?)?,
                thread: thread.id,
                seq: Self::seq_after(&thread, &staged)?,
                at: copy.header.at,
            };
            let copy = Event { header, ..copy };
            let raw = copy.to_raw()?;
            named.extend(blobs_named(&raw));
            staged.push(raw);
            history.push(copy);
        }
        let forked = ThreadForked {
            from: ForkPoint {
                thread: fork.from,
                seq: at,
            },
            files,
            told: (fork.told)(files),
            commands,
        };
        let seam = staged.len();
        self.stage(&thread, &mut staged, &forked)?;
        history.extend(decoded(staged.get(seam..).unwrap_or_default())?);
        self.refer_all(&scope, thread.id, &named).await?;
        if let Err(error) = self.commit(&mut thread, staged).await {
            self.blobs.release(&scope, thread.id).await?;
            return Err(error);
        }
        self.carry(&mut thread, &history, commands).await;
        Ok(thread)
    }

    /// Makes thread `id` refer to each blob in `named`: one gone already is left, as its copy
    /// still says what it was. When the store fails, what was referred to is let go again.
    async fn refer_all(
        &self,
        scope: &Scope,
        id: ThreadId,
        named: &BTreeSet<Sha256>,
    ) -> Result<(), HarnessError> {
        for sha256 in named {
            if let Err(error) = self.blobs.refer(scope, sha256, id).await {
                self.blobs.release(scope, id).await?;
                return Err(error.into());
            }
        }
        Ok(())
    }

    /// Takes into `thread`, just started, what its history says of how it goes on, as reading
    /// it again from its log would; its commands are numbered after the `commands` its copies
    /// started.
    async fn carry(&self, thread: &mut Thread, history: &[Event], commands: u32) {
        thread.messages = crate::conversation::conversation(history);
        self.take_history(thread, history).await;
        thread.commands.numbered_after(commands);
    }
}

/// The point `at` of a thread's `events` when it is between two of its turns (before the
/// first, or after one ended and before the next began), else the latest such point when
/// `at` is absent. A point within a turn would leave a call without its result, or an
/// approval or question waiting, which no other thread can answer.
fn between_turns(events: &[Event], at: Option<Seq>) -> Result<Seq, Invalid> {
    let started = events.first().map(|event| &event.body);
    if matches!(started, Some(Body::ThreadStarted(started)) if started.parent.is_some()) {
        return Err(Invalid::ForkSubagent);
    }
    let mut open = false;
    let mut latest = None;
    for event in events {
        match event.body {
            Body::TurnStarted(_) => open = true,
            Body::TurnEnded(_) => open = false,
            _ => {}
        }
        let seq = event.header.seq;
        if at == Some(seq) {
            return if open {
                Err(Invalid::ForkPoint)
            } else {
                Ok(seq)
            };
        }
        if !open {
            latest = Some(seq);
        }
    }
    match at {
        Some(_) => Err(Invalid::ForkPoint),
        None => latest.ok_or(Invalid::ForkPoint),
    }
}

/// Whether a fork copies `event` of the thread it comes from: everything but what is the
/// other's own, which the fork has its own of or none: its start, where it works, whether it
/// is shown, its commands in the background (whose ids would reach the other's), and where it
/// was itself forked, which told of another folder. All but its start are glave's own records,
/// which call ids do not count, so the fork's next calls are numbered as the other's would be.
fn copied(event: &Event) -> bool {
    !matches!(
        event.body,
        Body::ThreadStarted(_)
            | Body::WorkspaceBound(_)
            | Body::WorkspaceRemoved(_)
            | Body::ThreadArchived(_)
            | Body::ThreadUnarchived(_)
            | Body::ThreadDeleted(_)
            | Body::CommandStarted(_)
            | Body::CommandEnded(_)
            | Body::ThreadForked(_)
    )
}

/// How many numbers `events` gave commands in the background, which the model's history names
/// them by: one for each started, after those a fork's start says came before them.
fn commands_used(events: &[Event]) -> u32 {
    events.iter().fold(0, |used, event| match &event.body {
        Body::ThreadForked(forked) => used.max(forked.commands),
        Body::CommandStarted(_) => used.saturating_add(1),
        _ => used,
    })
}

/// Where the latest `workspace.bound` of `events` says the thread works, if anywhere.
fn root_of(events: &[Event]) -> Option<&str> {
    events.iter().rev().find_map(|event| match &event.body {
        Body::WorkspaceBound(bound) => Some(bound.root.as_str()),
        _ => None,
    })
}

/// Every blob `event` names, as written: a reference's `sha256` (a turn's configuration, a
/// file, what a service needs to go on) and the line the gate leaves where it kept something
/// (`blob:<SHA-256>`, `specs/invariants.md`, 26), in its text or its structured result alike.
fn blobs_named(event: &RawEvent) -> Vec<Sha256> {
    let data = event.data.get();
    ["\"sha256\":\"", "blob:"]
        .into_iter()
        .flat_map(|mark| data.match_indices(mark).map(move |(at, _)| at + mark.len()))
        .filter_map(|start| data.get(start..start.saturating_add(64)))
        .filter_map(|hex| Sha256::new(hex).ok())
        .collect()
}

/// `raw`, decoded.
fn decoded(raw: &[RawEvent]) -> Result<Vec<Event>, HarnessError> {
    raw.iter()
        .map(|event| decode(event.clone()).map_err(|e| HarnessError::Event(e.to_string())))
        .collect()
}

#[cfg(test)]
mod tests;
