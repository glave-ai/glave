//! Where a thread stands as what its turns made is told, read from its log whenever the thread
//! is, so nothing of it lasts only in memory: the turn open now and whether it has called a tool
//! that can change files, and where the last look-over stopped while a command still ran
//! (its mark), which the next starts from.

use glave_event::{At, Body, Event, RawEvent, Seq};

use crate::tools::Tool;

/// The turn open on a thread: where its `turn.started` is, and whether it has called a tool
/// that can change files.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct TurnOpen {
    pub(crate) turn: Seq,
    pub(crate) at: At,
    pub(crate) works: bool,
}

/// Where the last look-over stopped, a command of the thread's still running in the background:
/// the turn that looked, and the time on the computer's clock the next look-over starts from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Mark {
    pub(crate) turn: Seq,
    pub(crate) since_ns: u64,
}

/// Whether a call of the tool `name` can change files in the thread's folder: glave's own
/// `write`, `edit` or `bash`, or a worker unless the thread works on its person's own computer
/// (`at_home`), which may sleep, and which the worker wakes with its own tools.
pub(crate) fn changes_files(name: &str, at_home: bool) -> bool {
    matches!(
        Tool::named(name),
        Some(Tool::Write | Tool::Edit | Tool::Bash)
    ) || (!at_home && name.trim().eq_ignore_ascii_case(crate::subagent::AGENT))
}

/// The turn `events` open, its `turn.started` the last of them: nothing called yet.
pub(crate) fn opened_by(events: &[RawEvent]) -> Option<TurnOpen> {
    events.last().map(|event| TurnOpen {
        turn: event.seq,
        at: event.at,
        works: false,
    })
}

/// Where a thread read from `events` stands: the turn open now, and the mark the last
/// look-over left. A turn that worked there and ended, or a look-over while the thread was idle,
/// leaves a mark, or none when it looked up to its end; a turn that only talked looked at
/// nothing, and leaves the mark as it was.
pub(crate) fn from_log(events: &[Event], at_home: bool) -> (Option<TurnOpen>, Option<Mark>) {
    let (mut open, mut mark) = (None, None);
    for event in events {
        let (turn, at) = (event.header.seq, event.header.at);
        match &event.body {
            Body::TurnStarted(_) => {
                open = Some(TurnOpen {
                    turn,
                    at,
                    works: false,
                });
            }
            Body::ToolCallRequested(call) => {
                let works = changes_files(&call.tool, at_home);
                if let Some(open) = open.as_mut() {
                    open.works |= works;
                }
            }
            Body::TurnEnded(ended) => {
                if let Some(ended_turn) = open.take().filter(|open| open.works) {
                    mark = ended.made_until_ns.map(|since_ns| Mark {
                        turn: ended_turn.turn,
                        since_ns,
                    });
                }
            }
            Body::TurnMade(made) => {
                mark = made.made_until_ns.map(|since_ns| Mark {
                    turn: made.turn,
                    since_ns,
                });
            }
            _ => {}
        }
    }
    (open, mark)
}
