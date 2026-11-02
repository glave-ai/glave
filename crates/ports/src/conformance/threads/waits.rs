//! An open turn's wait, as every store's views must keep it: an approval asked and not decided,
//! what a service asks not answered, the model's questions not answered, and `agent` calls, whose
//! subagents' threads are found by the call that started each.

use glave_event::{
    ApprovalDecided, ApprovalRequested, Arguments, BlobRef, CallId, Outcome, ParentCall, Part,
    ServiceAnswered, ServiceAsked, ServiceOutcome, Sha256, ToolCallCompleted, ToolCallRequested,
    ToolKind, TurnResult, Verdict,
};

use super::{Writer, started, turn};
use crate::conformance::{Verdict as Checked, ensure, thread};
use crate::log::EventLog;
use crate::scope::Scope;
use crate::view::{Standing, ThreadQuery, Unfinished, Wait, WaitKind};

fn call(id: &str) -> Result<CallId, String> {
    CallId::new(id).map_err(|e| e.to_string())
}

fn requested(id: &str, tool: &str) -> Result<ToolCallRequested, String> {
    Ok(ToolCallRequested {
        call_id: call(id)?,
        provider_call_id: None,
        tool: tool.into(),
        arguments: Arguments::Json(serde_json::Map::new()),
        summary: String::new(),
        kind: ToolKind::Other,
    })
}

fn completed(id: &str) -> Result<ToolCallCompleted, String> {
    Ok(ToolCallCompleted {
        call_id: call(id)?,
        is_error: false,
        output: vec![Part::Text { text: "ok".into() }],
        structured: None,
        untrusted: false,
    })
}

/// The wait the view of `thread` in `here` shows.
async fn wait_of<L: EventLog>(
    log: &L,
    here: &Scope,
    thread: glave_event::ThreadId,
) -> Result<Option<Wait>, String> {
    let view = log
        .thread_view(here, thread)
        .await
        .map_err(|e| e.to_string())?;
    let view = view.ok_or("a thread has a view")?;
    Ok(view.turn.and_then(|turn| turn.wait))
}

/// An open turn waits at its first call without a result when that call is an approval asked
/// and not decided, the model's questions, or subagents' work; its wait clears with the next
/// event that settles it, and a subagent's thread is found by the call that started it.
pub(super) async fn waits<L: EventLog>(log: &L, here: &Scope) -> Checked {
    let (parent, child) = (thread(17)?, thread(18)?);
    let mut writer = Writer::new(parent);
    let mut events = vec![writer.next(40, &started()?)?];
    let mut opened = turn(&mut writer, 41)?;
    // The turn stays open: its end is never written, and the next event takes its place.
    opened.pop();
    writer.seq = writer.seq.saturating_sub(1);
    events.extend(opened);
    events.push(writer.next(44, &requested("call_6", "bash")?)?);
    log.append(here, &events).await.map_err(|e| e.to_string())?;
    ensure(
        wait_of(log, here, parent).await?.is_none(),
        "a call running waits for no one",
    )?;
    approval(log, here, &mut writer, parent).await?;
    service(log, here, &mut writer, parent).await?;
    let question = writer.next(48, &requested("call_9", "question")?)?;
    let asked_at = question.seq;
    log.append(here, &[question])
        .await
        .map_err(|e| e.to_string())?;
    ensure(
        wait_of(log, here, parent)
            .await?
            .is_some_and(|wait| wait.kind == WaitKind::Question && wait.seq == asked_at),
        "the model's questions must be the turn's wait",
    )?;
    let answered = writer.next(49, &completed("call_9")?)?;
    log.append(here, &[answered])
        .await
        .map_err(|e| e.to_string())?;
    let agents = [
        writer.next(50, &requested("call_11", "agent")?)?,
        writer.next(50, &requested("call_12", "agent")?)?,
    ];
    let first = agents.first().map(|event| event.seq);
    log.append(here, &agents).await.map_err(|e| e.to_string())?;
    ensure(
        wait_of(log, here, parent).await?.is_some_and(|wait| {
            wait.kind == WaitKind::Agent
                && Some(wait.seq) == first
                && wait.call.as_str() == "call_11"
        }),
        "subagents' work must be the turn's wait, at its first call",
    )?;
    children(log, here, parent, child).await?;
    let ended = writer.next(
        52,
        &TurnResult {
            outcome: Outcome::Interrupted,
            ..TurnResult::begun()
        },
    )?;
    log.append(here, &[ended])
        .await
        .map_err(|e| e.to_string())?;
    let view = log
        .thread_view(here, parent)
        .await
        .map_err(|e| e.to_string())?;
    ensure(
        view.is_some_and(|view| view.turn.is_none()),
        "a turn that ended must wait for nothing",
    )
}

/// An approval asked and not decided is the turn's wait, at its request; decided, its call
/// says so until it has a result.
async fn approval<L: EventLog>(
    log: &L,
    here: &Scope,
    writer: &mut Writer,
    parent: glave_event::ThreadId,
) -> Checked {
    let ask = ApprovalRequested {
        call_id: call("call_6")?,
        tool: "bash".into(),
        summary: "bash: make".into(),
        ask: "Run this command?".into(),
        detail: "make".into(),
        secret: false,
        caution: None,
        reason: None,
        outside_sandbox: None,
    };
    let asked = writer.next(45, &ask)?;
    let asked_at = asked.seq;
    log.append(here, &[asked])
        .await
        .map_err(|e| e.to_string())?;
    let waiting = wait_of(log, here, parent).await?;
    ensure(
        waiting.is_some_and(|wait| {
            wait.kind == WaitKind::Approval
                && wait.seq == asked_at
                && wait.call.as_str() == "call_6"
        }),
        "an approval asked and not decided must be the turn's wait, at its request",
    )?;
    let decided = ApprovalDecided {
        call_id: call("call_6")?,
        decision: Verdict::AllowOnce,
        by: None,
    };
    let settled = [
        writer.next(46, &decided)?,
        writer.next(47, &completed("call_6")?)?,
    ];
    log.append(here, &settled[..1])
        .await
        .map_err(|e| e.to_string())?;
    ensure(
        wait_of(log, here, parent).await?.is_none(),
        "a decided approval must no longer wait",
    )?;
    let view = log
        .thread_view(here, parent)
        .await
        .map_err(|e| e.to_string())?;
    let first = view
        .and_then(|view| view.turn)
        .and_then(|turn| turn.unfinished.into_iter().next());
    ensure(
        first.is_some_and(|call| call.decided && call.asked.is_none()),
        "a decided approval's call must say so until it has a result",
    )?;
    log.append(here, &settled[1..])
        .await
        .map_err(|e| e.to_string())?;
    Ok(())
}

/// What a service asks is the turn's wait, at its `service.asked`, even after an approval of the
/// same call was decided, which is what makes the call run; answered, the call says so until it
/// has a result, or until the service asks again.
async fn service<L: EventLog>(
    log: &L,
    here: &Scope,
    writer: &mut Writer,
    parent: glave_event::ThreadId,
) -> Checked {
    let ask = ApprovalRequested {
        call_id: call("call_7")?,
        tool: "mcp__github__file".into(),
        summary: "mcp__github__file {}".into(),
        ask: "Allow this?".into(),
        detail: "{}".into(),
        secret: false,
        caution: None,
        reason: None,
        outside_sandbox: None,
    };
    let decided = ApprovalDecided {
        call_id: call("call_7")?,
        decision: Verdict::AllowOnce,
        by: None,
    };
    let opened = [
        writer.next(47, &requested("call_7", "mcp__github__file")?)?,
        writer.next(47, &ask)?,
        writer.next(47, &decided)?,
    ];
    log.append(here, &opened).await.map_err(|e| e.to_string())?;
    for _round in 0..2 {
        service_round(log, here, writer, parent).await?;
    }
    let done = writer.next(47, &completed("call_7")?)?;
    log.append(here, &[done]).await.map_err(|e| e.to_string())?;
    Ok(())
}

/// The service of `call_7` asks, which is the turn's wait, and is answered, which ends it.
async fn service_round<L: EventLog>(
    log: &L,
    here: &Scope,
    writer: &mut Writer,
    parent: glave_event::ThreadId,
) -> Checked {
    let asks = ServiceAsked {
        call_id: call("call_7")?,
        service: "GitHub".into(),
        ask: "GitHub asks you, not the model".into(),
        note: "Your answer goes only to GitHub.".into(),
        message: "Which repository?".into(),
        fields: Vec::new(),
        state: BlobRef {
            sha256: Sha256::from_digest([7; 32]),
            bytes: 2,
            media_type: "application/json".into(),
        },
        adds: None,
    };
    let asked = writer.next(47, &asks)?;
    let asked_at = asked.seq;
    log.append(here, &[asked])
        .await
        .map_err(|e| e.to_string())?;
    ensure(
        wait_of(log, here, parent).await?.is_some_and(|wait| {
            wait.kind == WaitKind::Service && wait.seq == asked_at && wait.call.as_str() == "call_7"
        }),
        "what a service asks must be the turn's wait, after any approval of its call",
    )?;
    let first = first_unfinished(log, here, parent).await?;
    ensure(
        first.is_some_and(|call| !call.decided),
        "a call a service waits at must not read as answered",
    )?;
    let answer = ServiceAnswered {
        call_id: call("call_7")?,
        service: "GitHub".into(),
        outcome: ServiceOutcome::Declined,
        told: "You declined GitHub's question".into(),
        answers: None,
        by: None,
    };
    let answered = writer.next(47, &answer)?;
    log.append(here, &[answered])
        .await
        .map_err(|e| e.to_string())?;
    ensure(
        wait_of(log, here, parent).await?.is_none(),
        "what a service asked, answered, must no longer wait",
    )?;
    let first = first_unfinished(log, here, parent).await?;
    ensure(
        first.is_some_and(|call| call.decided && call.service.is_none()),
        "an answered service's call must say so until it has a result",
    )
}

/// The first call without a result of the turn open in `thread`.
async fn first_unfinished<L: EventLog>(
    log: &L,
    here: &Scope,
    thread: glave_event::ThreadId,
) -> Result<Option<Unfinished>, String> {
    let view = log
        .thread_view(here, thread)
        .await
        .map_err(|e| e.to_string())?;
    Ok(view
        .and_then(|view| view.turn)
        .and_then(|turn| turn.unfinished.into_iter().next()))
}

/// A subagent's thread is found by the call that started it, and by no other.
async fn children<L: EventLog>(
    log: &L,
    here: &Scope,
    parent: glave_event::ThreadId,
    child: glave_event::ThreadId,
) -> Checked {
    let mut writer = Writer::new(child);
    let mut begun = started()?;
    begun.parent = Some(ParentCall {
        thread: parent,
        call_id: call("call_11")?,
    });
    log.append(here, &[writer.next(51, &begun)?])
        .await
        .map_err(|e| e.to_string())?;
    let by = |call_id: &str| -> Result<ThreadQuery, String> {
        Ok(ThreadQuery {
            standing: Standing::Active,
            limit: 10,
            after: None,
            open: false,
            parent: Some(ParentCall {
                thread: parent,
                call_id: call(call_id)?,
            }),
        })
    };
    let found = log
        .thread_views(here, &by("call_11")?)
        .await
        .map_err(|e| e.to_string())?;
    ensure(
        found.len() == 1
            && found.first().is_some_and(|view| {
                view.thread == child
                    && view
                        .parent_call
                        .as_ref()
                        .is_some_and(|c| c.as_str() == "call_11")
            }),
        "a subagent's thread must be found by the call that started it",
    )?;
    let none = log
        .thread_views(here, &by("call_12")?)
        .await
        .map_err(|e| e.to_string())?;
    ensure(
        none.is_empty(),
        "no thread must be found for a call that started none",
    )
}
