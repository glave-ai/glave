//! A thread's turns in steps (`specs/session.md`, "Steps"), read from its log: each run of tool
//! calls with nothing said between them, told in a line by the kinds of work its calls did, and
//! each turn's reply, its last words.

use std::collections::BTreeSet;

use glave_event::{Arguments, Body, CallId, RawEvent, Role, Seq, ThreadId, ToolKind};
use glave_ports::{BlobStore, CallKind, EventLog, ModelProvider};

use crate::protocol::{ApiError, CallRun, Reply, ThreadSteps, Told};
use crate::server::Server;
use crate::sources::{said, turn_event};
use crate::words::Sentence;

impl<P, L, B> Server<P, L, B>
where
    P: ModelProvider + 'static,
    L: EventLog + 'static,
    B: BlobStore + 'static,
{
    /// `thread`'s turns in steps, read from its log.
    pub(crate) async fn steps(&self, thread: ThreadId) -> Result<Reply, ApiError> {
        let mut reading = Reading::default();
        self.read_all(thread, |event| reading.read(event)).await?;
        Ok(Reply::Steps {
            steps: reading.done(),
        })
    }
}

/// A thread's log read so far, in order.
#[derive(Default)]
struct Reading {
    runs: Vec<Run>,
    /// Whether the last run still takes calls: nothing has been said since its last.
    open: bool,
    replies: Vec<Seq>,
    /// The latest words of the model's in the turn it is in.
    words: Option<Seq>,
}

/// One run of calls, as read so far.
struct Run {
    first: Seq,
    calls: Vec<Call>,
}

/// One call of a run.
struct Call {
    id: CallId,
    kind: ToolKind,
    /// The file it names, for the kinds counted by file.
    path: Option<String>,
    /// Whether it has a result, and whether that is a failure.
    ended: Option<bool>,
}

impl Reading {
    /// Reads the next event of the log.
    fn read(&mut self, event: RawEvent) {
        let Some((seq, body)) = turn_event(event) else {
            return;
        };
        match body {
            Body::TurnStarted(_) => {
                self.open = false;
                self.words = None;
            }
            Body::TurnEnded(_) => {
                self.open = false;
                self.replies.extend(self.words.take());
            }
            // Questions are put to the person between runs, never in one.
            Body::ToolCallRequested(call) if CallKind::of(&call.tool) == CallKind::Question => {
                self.open = false;
            }
            // What a service asks stands between runs too, as what the person answers in place.
            Body::ServiceAsked(_) => self.open = false,
            Body::ToolCallRequested(call) => {
                let made = Call {
                    path: path_of(call.kind, &call.arguments),
                    id: call.call_id,
                    kind: call.kind,
                    ended: None,
                };
                match self.runs.last_mut() {
                    Some(run) if self.open => run.calls.push(made),
                    _ => self.runs.push(Run {
                        first: seq,
                        calls: vec![made],
                    }),
                }
                self.open = true;
            }
            Body::ToolCallCompleted(done) => {
                let found = self
                    .runs
                    .iter_mut()
                    .rev()
                    .find_map(|run| run.calls.iter_mut().find(|call| call.id == done.call_id));
                if let Some(call) = found {
                    call.ended = Some(done.is_error);
                }
            }
            Body::MessageAdded(message) if said(&message.parts) => {
                self.open = false;
                if message.role == Role::Assistant {
                    self.words = Some(seq);
                }
            }
            _ => {}
        }
    }

    /// Every run, told, and the replies of the turns that ended.
    fn done(self) -> ThreadSteps {
        ThreadSteps {
            runs: self.runs.into_iter().map(Run::told).collect(),
            replies: self.replies,
        }
    }
}

impl Run {
    /// Its calls, and what they did in a line.
    fn told(self) -> CallRun {
        let sentence = sentence(&self.calls);
        CallRun {
            first: self.first,
            calls: self.calls.into_iter().map(|call| call.id).collect(),
            sentence,
        }
    }
}

/// The clause that tells how many calls of a kind a run made, given how it stands.
type Clause = fn(String, u64) -> Sentence;

/// The kinds a run is told by, in the order its line names them, and the clause of each.
const KINDS: [(ToolKind, Clause); 6] = [
    (ToolKind::Command, |how, count| Sentence::RanCommands {
        how,
        count,
    }),
    (ToolKind::Read, |how, count| Sentence::ReadFiles {
        how,
        count,
    }),
    (ToolKind::Search, |how, count| Sentence::RanSearches {
        how,
        count,
    }),
    (ToolKind::Edit, |how, count| Sentence::EditedFiles {
        how,
        count,
    }),
    (ToolKind::Web, |how, count| Sentence::UsedWeb { how, count }),
    (ToolKind::Other, |how, count| Sentence::UsedTools {
        how,
        count,
    }),
];

/// What `calls` did, in a line: a clause for each kind of work, in the present while one of
/// them runs, then how many failed, when any did.
fn sentence(calls: &[Call]) -> Told {
    let now = calls.iter().any(|call| call.ended.is_none());
    let mut said = Vec::new();
    for (kind, clause) in KINDS {
        let count = counted(calls.iter().filter(|call| call.kind == kind));
        if count == 0 {
            continue;
        }
        let how = match (said.is_empty(), now) {
            (true, true) => "first-now",
            (false, true) => "then-now",
            (true, false) => "first-done",
            (false, false) => "then-done",
        };
        said.push(clause(how.to_owned(), count));
    }
    let failed = calls.iter().filter(|call| call.ended == Some(true)).count();
    if failed > 0 {
        said.push(Sentence::CallsFailed {
            count: u64::try_from(failed).unwrap_or(u64::MAX),
        });
    }
    Told::joined(said)
}

/// How many of `calls` there are: by the files they name, each once, where they name one.
fn counted<'a>(calls: impl Iterator<Item = &'a Call>) -> u64 {
    let mut files = BTreeSet::new();
    let mut count: u64 = 0;
    for call in calls {
        let first = call.path.as_ref().is_none_or(|path| files.insert(path));
        if first {
            count = count.saturating_add(1);
        }
    }
    count
}

/// The file a call of `kind` names, for the kinds counted by file: a read and an edit.
fn path_of(kind: ToolKind, arguments: &Arguments) -> Option<String> {
    if !matches!(kind, ToolKind::Read | ToolKind::Edit) {
        return None;
    }
    match arguments {
        Arguments::Json(map) => map.get("path")?.as_str().map(str::to_owned),
        Arguments::Malformed(_) => None,
    }
}

#[cfg(test)]
mod tests;
