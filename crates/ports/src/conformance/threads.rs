//! The threads view suite: what every event log's views must show, kept in step with appends
//! and equal to what a rebuild from the log makes.

use glave_event::{
    At, ComputerKind, ComputerRef, Cost, Currency, EventId, Header, MessageAdded, ModelRef,
    ModelResponded, Money, Opened, Origin, OriginKind, Outcome, Part, Payload, Principal,
    PrincipalId, Producer, RawEvent, Role, Seq, StopReason, ThreadArchived, ThreadCompacted,
    ThreadId, ThreadRenamed, ThreadStarted, TokenCounts, TurnResult, TurnStarted, Usage,
    WorkspaceBound, encode,
};

use super::{Verdict, ensure, scope, thread, uuid_v7};

mod waits;
use crate::blob::digest;
use crate::log::EventLog;
use crate::scope::Scope;
use crate::view::{Standing, ThreadQuery, ThreadView};

/// Builds a thread's events, numbering them and giving each its own second.
pub(super) struct Writer {
    thread: ThreadId,
    seq: u64,
}

impl Writer {
    pub(super) fn new(thread: ThreadId) -> Self {
        Self { thread, seq: 0 }
    }

    pub(super) fn next<P: Payload>(
        &mut self,
        second: u64,
        payload: &P,
    ) -> Result<RawEvent, String> {
        self.seq = self.seq.saturating_add(1);
        let header = Header {
            id: EventId::try_from(uuid_v7(10_000 + second * 100 + self.seq))
                .map_err(|e| e.to_string())?,
            thread: self.thread,
            seq: Seq::new(self.seq).map_err(|e| e.to_string())?,
            at: at(second)?,
        };
        encode(header, payload).map_err(|e| e.to_string())
    }
}

fn at(second: u64) -> Result<At, String> {
    At::try_from(format!(
        "2026-10-07T21:{:02}:{:02}.000Z",
        second / 60,
        second % 60
    ))
    .map_err(|e| e.to_string())
}

pub(super) fn person() -> Result<Principal, String> {
    PrincipalId::try_from(uuid_v7(9_000))
        .map(Principal::person)
        .map_err(|e| e.to_string())
}

pub(super) fn started() -> Result<ThreadStarted, String> {
    Ok(ThreadStarted {
        producer: Producer {
            name: "glave".into(),
            version: "0.0.0".into(),
        },
        fork: None,
        parent: None,
        for_: Some(person()?),
        agent: None,
        origin: Origin::of(OriginKind::Prompt),
    })
}

fn said(text: &str) -> MessageAdded {
    MessageAdded {
        role: Role::User,
        parts: vec![Part::Text { text: text.into() }],
        author: None,
        thought: None,
    }
}

fn model() -> ModelRef {
    ModelRef {
        provider: "test".into(),
        name: "model".into(),
    }
}

fn responded() -> Result<ModelResponded, String> {
    let counts = TokenCounts {
        input: 100,
        cache_read: 0,
        cache_write: 0,
        output: 10,
        reasoning: 0,
    };
    Ok(ModelResponded {
        model: model(),
        response_id: None,
        stop: StopReason::EndTurn,
        provider_stop: None,
        usage: Usage::new(counts, false, None).map_err(|e| e.to_string())?,
        cost: Some(Cost {
            amount: Money {
                micros: 5_000,
                currency: Currency::USD,
            },
            price_list: "test".into(),
        }),
        route: None,
        target: None,
        service: None,
        reasoning_stripped: false,
        upstream: None,
        effort: None,
    })
}

/// A whole turn: a message from the person, then the turn and one response.
fn turn(writer: &mut Writer, from: u64) -> Result<Vec<RawEvent>, String> {
    let blob = glave_event::BlobRef {
        sha256: digest(b"config"),
        bytes: 6,
        media_type: "application/json".into(),
    };
    Ok(vec![
        writer.next(from, &said("  \n  Fix the login test  \nIt fails on CI."))?,
        writer.next(
            from + 1,
            &TurnStarted {
                model: model(),
                config: blob,
                trace: None,
                auth: None,
            },
        )?,
        writer.next(from + 2, &responded()?)?,
        writer.next(
            from + 3,
            &TurnResult {
                outcome: Outcome::Completed,
                ..TurnResult::begun()
            },
        )?,
    ])
}

async fn list<L: EventLog>(
    log: &L,
    scope: &Scope,
    standing: Standing,
    limit: usize,
) -> Result<Vec<ThreadView>, String> {
    let query = ThreadQuery {
        standing,
        limit,
        after: None,
        open: false,
        parent: None,
    };
    log.thread_views(scope, &query)
        .await
        .map_err(|e| e.to_string())
}

fn ids(views: &[ThreadView]) -> Vec<ThreadId> {
    views.iter().map(|view| view.thread).collect()
}

/// Whether `view` is thread `a` as the suite wrote it: one whole turn.
fn worked(view: &ThreadView) -> bool {
    view.title_seq == Seq::new(2).ok()
        && view.turns == 1
        && view.outcome == Some(Outcome::Completed)
        && view.tokens.input == 100
        && view.tokens.output == 10
        && view.cost.is_some_and(|cost| cost.micros == 5_000)
        && view.for_ == person().ok()
        && at(0).is_ok_and(|start| view.created_at == start)
        && at(4).is_ok_and(|end| view.updated_at == end)
        && view.last_seq.get() == 5
}

/// Every [`EventLog`] keeps a view of each thread as it is appended: listed by standing, most
/// recently written first, in pages after a cursor, inside its scope; ledgers (no
/// `thread.started`) have none; and rebuilding a view from the log gives the same view.
pub async fn thread_views<L: EventLog>(log: &L) -> Verdict {
    let (here, elsewhere) = (scope(11), scope(12));
    let (a, b, c, ledger) = (thread(11)?, thread(12)?, thread(13)?, thread(14)?);
    let mut first = Writer::new(a);
    let mut events = vec![first.next(0, &started()?)?];
    events.extend(turn(&mut first, 1)?);
    log.append(&here, &events)
        .await
        .map_err(|e| e.to_string())?;
    let mut second = Writer::new(b);
    let mut third = Writer::new(c);
    let appends = [
        (b, vec![second.next(5, &started()?)?]),
        (b, vec![second.next(6, &said("Second"))?]),
        (c, vec![third.next(7, &started()?)?]),
        (
            ledger,
            vec![Writer::new(ledger).next(
                8,
                &TurnResult {
                    outcome: Outcome::Failed,
                    ..TurnResult::begun()
                },
            )?],
        ),
    ];
    for (_, events) in appends {
        log.append(&here, &events)
            .await
            .map_err(|e| e.to_string())?;
    }

    let active = list(log, &here, Standing::Active, 10).await?;
    ensure(
        ids(&active) == [c, b, a],
        "views must list most recently written first, and only threads",
    )?;
    ensure(
        active.iter().any(|view| view.thread == a && worked(view)),
        "a view must hold the thread's start, title event, turns, usage and cost",
    )?;
    paging(log, &here, &active).await?;

    let archived = vec![second.next(9, &ThreadArchived { by: None })?];
    log.append(&here, &archived)
        .await
        .map_err(|e| e.to_string())?;
    let renamed = vec![first.next(
        10,
        &ThreadRenamed {
            title: "Login".into(),
            by: None,
        },
    )?];
    log.append(&here, &renamed)
        .await
        .map_err(|e| e.to_string())?;
    ensure(
        ids(&list(log, &here, Standing::Active, 10).await?) == [a, c],
        "an archived thread must leave the active views, and a rename must update a view",
    )?;
    ensure(
        ids(&list(log, &here, Standing::Archived, 10).await?) == [b],
        "an archived thread must be listed with the archived views",
    )?;
    let renamed = log.thread_view(&here, a).await.map_err(|e| e.to_string())?;
    ensure(
        renamed.as_ref().and_then(|view| view.title_seq) == Seq::new(6).ok(),
        "a rename must become the thread's title event",
    )?;

    isolation(log, &here, &elsewhere, a, ledger).await?;
    rebuilding(log, &here, &[a, b, c]).await?;
    opening_messages(log, &here, thread(15)?).await?;
    open_turns(log, &here, thread(16)?).await?;
    where_and_how_full(log, &here, thread(19)?).await?;
    waits::waits(log, &here).await
}

/// A view says where its thread works, its latest `workspace.bound`, and what its conversation
/// last sent and got back: the latest call of its own, never the judge's, none once it was
/// compacted; and whether any call's usage was estimated.
async fn where_and_how_full<L: EventLog>(log: &L, here: &Scope, id: ThreadId) -> Verdict {
    let mut writer = Writer::new(id);
    let bound = WorkspaceBound {
        workspace: uuid_v7(17),
        computer: ComputerRef {
            id: uuid_v7(18),
            kind: ComputerKind::Local,
        },
        opened: Opened::Folder {
            path: "/work".into(),
        },
        root: "/work".into(),
    };
    let estimated = ModelResponded {
        usage: Usage::new(
            TokenCounts {
                input: 2_000,
                output: 50,
                ..TokenCounts::default()
            },
            true,
            None,
        )
        .map_err(|e| e.to_string())?,
        route: Some("main".into()),
        ..responded()?
    };
    let judged = ModelResponded {
        route: Some("judge".into()),
        ..responded()?
    };
    let events = vec![
        writer.next(40, &started()?)?,
        writer.next(41, &bound)?,
        writer.next(42, &estimated)?,
        writer.next(43, &judged)?,
    ];
    log.append(here, &events).await.map_err(|e| e.to_string())?;
    let view = log.thread_view(here, id).await.map_err(|e| e.to_string())?;
    ensure(
        view.as_ref().is_some_and(|view| {
            view.workspace_seq == Seq::new(2).ok()
                && view.estimated
                && view
                    .context
                    .as_ref()
                    .is_some_and(|last| last.tokens == 2_050 && last.model == "model")
        }),
        "a view must say where its thread works, that a usage was estimated, and what its own \
         latest call sent and got back, not the judge's",
    )?;
    let compacted = ThreadCompacted {
        through: Seq::new(4).map_err(|e| e.to_string())?,
        parts: Vec::new(),
    };
    let more = vec![writer.next(44, &compacted)?];
    log.append(here, &more).await.map_err(|e| e.to_string())?;
    let view = log.thread_view(here, id).await.map_err(|e| e.to_string())?;
    ensure(
        view.is_some_and(|view| view.context.is_none() && view.workspace_seq.is_some()),
        "a compacted thread's view must say nothing of its context until its next call",
    )
}

/// A turn started and not ended is open: the view says where it started, and only such threads
/// are listed when the query asks for open turns, until the turn ends.
async fn open_turns<L: EventLog>(log: &L, here: &Scope, id: ThreadId) -> Verdict {
    let mut writer = Writer::new(id);
    let mut events = vec![writer.next(30, &started()?)?];
    events.extend(turn(&mut writer, 31)?);
    let ended = events.pop().ok_or("a turn has an end")?;
    log.append(here, &events).await.map_err(|e| e.to_string())?;
    let query = ThreadQuery {
        standing: Standing::Active,
        limit: 10,
        after: None,
        open: true,
        parent: None,
    };
    let open = log
        .thread_views(here, &query)
        .await
        .map_err(|e| e.to_string())?;
    ensure(
        ids(&open) == [id]
            && open
                .first()
                .and_then(|view| view.turn.as_ref())
                .is_some_and(|turn| turn.started.get() == 3),
        "a turn not yet ended must be open, from its start, and alone in a listing of open turns",
    )?;
    log.append(here, &[ended])
        .await
        .map_err(|e| e.to_string())?;
    let closed = log
        .thread_views(here, &query)
        .await
        .map_err(|e| e.to_string())?;
    let view = log.thread_view(here, id).await.map_err(|e| e.to_string())?;
    ensure(
        closed.is_empty() && view.is_some_and(|view| view.turn.is_none()),
        "a turn that ended must no longer be open",
    )
}

/// A thread's first message opens with the project's instructions, its last part being what
/// the person wrote: the title is from that part, and a message whose last part is blank
/// gives none, however much text comes before it.
async fn opening_messages<L: EventLog>(log: &L, here: &Scope, id: ThreadId) -> Verdict {
    let mut writer = Writer::new(id);
    let parts = |prompt: &str| MessageAdded {
        role: Role::User,
        parts: vec![
            Part::Text {
                text: "# AGENTS.md instructions for /repo\n\n<INSTRUCTIONS>…</INSTRUCTIONS>".into(),
            },
            Part::Text {
                text: prompt.into(),
            },
        ],
        author: None,
        thought: None,
    };
    let events = vec![
        writer.next(20, &started()?)?,
        writer.next(21, &parts("   "))?,
        writer.next(22, &parts("Deploy the site"))?,
    ];
    log.append(here, &events).await.map_err(|e| e.to_string())?;
    let view = log.thread_view(here, id).await.map_err(|e| e.to_string())?;
    ensure(
        view.as_ref().and_then(|view| view.title_seq) == Seq::new(3).ok(),
        "a message titles its thread by its last text part, not the instructions before it",
    )?;
    ensure(
        events.get(2).and_then(crate::view::title_of).as_deref() == Some("Deploy the site"),
        "the title is the first line of the message's last text part",
    )
}

/// Pages follow each other through the cursor of the last view.
async fn paging<L: EventLog>(log: &L, here: &Scope, all: &[ThreadView]) -> Verdict {
    let page = list(log, here, Standing::Active, 2).await?;
    ensure(
        ids(&page) == ids(all.get(..2).unwrap_or_default()),
        "a page must stop at its limit",
    )?;
    let query = ThreadQuery {
        standing: Standing::Active,
        limit: 10,
        after: page.last().map(ThreadView::cursor),
        open: false,
        parent: None,
    };
    let rest = log
        .thread_views(here, &query)
        .await
        .map_err(|e| e.to_string())?;
    ensure(
        ids(&rest) == ids(all.get(2..).unwrap_or_default()),
        "the next page must start right after the cursor",
    )
}

async fn isolation<L: EventLog>(
    log: &L,
    here: &Scope,
    elsewhere: &Scope,
    a: ThreadId,
    ledger: ThreadId,
) -> Verdict {
    ensure(
        log.thread_view(here, ledger)
            .await
            .map_err(|e| e.to_string())?
            .is_none(),
        "a stream without thread.started must have no view",
    )?;
    ensure(
        log.thread_view(elsewhere, a)
            .await
            .map_err(|e| e.to_string())?
            .is_none()
            && list(log, elsewhere, Standing::Active, 10).await?.is_empty(),
        "views must not cross scopes",
    )
}

async fn rebuilding<L: EventLog>(log: &L, here: &Scope, threads: &[ThreadId]) -> Verdict {
    for thread in threads {
        let kept = log
            .thread_view(here, *thread)
            .await
            .map_err(|e| e.to_string())?;
        let rebuilt = log
            .rebuild_thread_view(here, *thread)
            .await
            .map_err(|e| e.to_string())?;
        ensure(
            kept.is_some() && rebuilt == kept,
            "a view rebuilt from the log must equal the one kept by appends",
        )?;
        let stored = log
            .thread_view(here, *thread)
            .await
            .map_err(|e| e.to_string())?;
        ensure(stored == rebuilt, "a rebuilt view must be stored")?;
    }
    Ok(())
}
