//! What a thread changed in files (`specs/session.md`, "Changes"), read from its log: each
//! `write` or `edit` call that finished without an error is a change, as its arguments asked
//! for it, which the tool applied exactly; an `agent` call's subagent, which works in its
//! parent's folder, changes files in the turn that made the call.

use glave_event::{Arguments, Body, CallId, RawEvent, Seq, ThreadId, decode};
use glave_harness::Tool;
use glave_ports::{BlobStore, CallKind, EventLog, ModelProvider};

use crate::protocol::{
    ApiError, ChangeKind, ChangedFile, DiffLine, FileChange, LineKind, Reply, ThreadChanges,
    TurnChanges,
};
use crate::server::Server;
use crate::words::Sentence;

impl<P, L, B> Server<P, L, B>
where
    P: ModelProvider + 'static,
    L: EventLog + 'static,
    B: BlobStore + 'static,
{
    /// What `thread` changed in files, its subagents' changes with its own.
    pub(crate) async fn changes(&self, thread: ThreadId) -> Result<Reply, ApiError> {
        let mut reading = Reading::default();
        self.read_all(thread, |event| reading.read(event)).await?;
        let mut changes = Vec::with_capacity(reading.found.len());
        for found in reading.found {
            match found {
                Found::Made(change) => changes.push(change),
                Found::Agent { turn, call } => {
                    changes.extend(self.subagent_changes(thread, &call, turn).await?);
                }
            }
        }
        Ok(Reply::Changes {
            changes: ThreadChanges {
                files: files_of(changes.iter()),
                turns: turns_of(&changes),
                note: Sentence::ChangesLeftOut.into(),
            },
        })
    }

    /// The changes of the subagent `parent`'s `call` started, as `parent`'s `turn`'s: none when
    /// it started none. A subagent starts none of its own.
    async fn subagent_changes(
        &self,
        parent: ThreadId,
        call: &CallId,
        turn: Option<Seq>,
    ) -> Result<Vec<Made>, ApiError> {
        let scope = self.scope_of(parent).await?;
        let child = self.inner.harness.child_of(&scope, parent, call).await;
        let Some(child) = child.map_err(|error| ApiError::from(&error))? else {
            return Ok(Vec::new());
        };
        let mut reading = Reading::default();
        self.read_all(child.thread, |event| reading.read(event))
            .await?;
        let changes = reading.found.into_iter().filter_map(|found| match found {
            Found::Made(change) => Some(Made { turn, ..change }),
            Found::Agent { .. } => None,
        });
        Ok(changes.collect())
    }
}

/// One change to a file, in the turn that made it.
struct Made {
    turn: Option<Seq>,
    path: String,
    change: FileChange,
}

/// What a thread's log holds that changed files, in order: a change, or a subagent, whose
/// changes are its parent's.
enum Found {
    Made(Made),
    Agent { turn: Option<Seq>, call: CallId },
}

/// A thread's log read so far: what changed files, the calls that will once they finish
/// without an error, and the turn it is in.
#[derive(Default)]
struct Reading {
    found: Vec<Found>,
    asked: Vec<(CallId, String, FileChange)>,
    turn: Option<Seq>,
}

impl Reading {
    /// Reads the next event of the log.
    fn read(&mut self, event: RawEvent) {
        if !matches!(
            event.kind.as_str(),
            "turn.started" | "tool.call.requested" | "tool.call.completed"
        ) {
            return;
        }
        let seq = event.seq;
        let Ok(event) = decode(event) else {
            return;
        };
        match event.body {
            Body::TurnStarted(_) => self.turn = Some(seq),
            Body::ToolCallRequested(requested)
                if CallKind::of(&requested.tool) == CallKind::Agent =>
            {
                self.found.push(Found::Agent {
                    turn: self.turn,
                    call: requested.call_id,
                });
            }
            Body::ToolCallRequested(requested) => {
                if let Some((path, change)) = asked(&requested.tool, &requested.arguments) {
                    self.asked.push((requested.call_id, path, change));
                }
            }
            Body::ToolCallCompleted(done) => {
                let Some(at) = self
                    .asked
                    .iter()
                    .position(|(call, ..)| *call == done.call_id)
                else {
                    return;
                };
                let (_, path, change) = self.asked.swap_remove(at);
                if !done.is_error {
                    let turn = self.turn;
                    self.found.push(Found::Made(Made { turn, path, change }));
                }
            }
            _ => {}
        }
    }
}

/// The change a call of `tool` with `arguments` asks for, and the path of the file it changes:
/// a `write`'s or an `edit`'s that names a file ([`Tool::changes`]), else none.
fn asked(tool: &str, arguments: &Arguments) -> Option<(String, FileChange)> {
    let path = Tool::changes(tool, arguments)?;
    let Arguments::Json(map) = arguments else {
        return None;
    };
    let text = |name: &str| {
        map.get(name)
            .and_then(serde_json::Value::as_str)
            .unwrap_or("")
    };
    let change = match Tool::named(tool)? {
        Tool::Write => FileChange {
            kind: ChangeKind::Wrote,
            lines: marked(LineKind::Added, &lines_of(text("content"))),
        },
        Tool::Edit => FileChange {
            kind: match map.get("replace_all").and_then(serde_json::Value::as_bool) {
                Some(true) => ChangeKind::EditedEvery,
                _ => ChangeKind::Edited,
            },
            lines: edited(text("old_string"), text("new_string")),
        },
        _ => return None,
    };
    Some((path.to_owned(), change))
}

/// The lines of `text`: a last newline ends the last line rather than starting another.
fn lines_of(text: &str) -> Vec<&str> {
    if text.is_empty() {
        return Vec::new();
    }
    let text = text.strip_suffix('\n').unwrap_or(text);
    text.split('\n')
        .map(|line| line.strip_suffix('\r').unwrap_or(line))
        .collect()
}

/// `lines`, each one `kind`.
fn marked(kind: LineKind, lines: &[&str]) -> Vec<DiffLine> {
    lines
        .iter()
        .map(|text| DiffLine {
            kind,
            text: (*text).to_owned(),
        })
        .collect()
}

/// The lines of an edit that swapped `before` for `after`: those both have at the start and at
/// the end kept, the rest taken out and put in.
fn edited(before: &str, after: &str) -> Vec<DiffLine> {
    let (old, new) = (lines_of(before), lines_of(after));
    let start = old.iter().zip(&new).take_while(|(a, b)| a == b).count();
    let (first, old) = old.split_at_checked(start).unwrap_or((&[], &old));
    let new = new.get(start..).unwrap_or_default();
    let end = old
        .iter()
        .rev()
        .zip(new.iter().rev())
        .take_while(|(a, b)| a == b)
        .count();
    let (taken, last) = old
        .split_at_checked(old.len().saturating_sub(end))
        .unwrap_or((old, &[]));
    let put = new.get(..new.len().saturating_sub(end)).unwrap_or_default();
    [
        marked(LineKind::Same, first),
        marked(LineKind::Removed, taken),
        marked(LineKind::Added, put),
        marked(LineKind::Same, last),
    ]
    .concat()
}

/// `changes` by file, in the order each was first changed, with the lines they added and took
/// out.
fn files_of<'a>(changes: impl Iterator<Item = &'a Made>) -> Vec<ChangedFile> {
    let mut files: Vec<ChangedFile> = Vec::new();
    for Made { path, change, .. } in changes {
        let count = |kind: LineKind| {
            let lines = change.lines.iter().filter(|line| line.kind == kind).count();
            u64::try_from(lines).unwrap_or(u64::MAX)
        };
        let (added, removed) = (count(LineKind::Added), count(LineKind::Removed));
        match files.iter_mut().find(|file| file.path == *path) {
            Some(file) => {
                file.added = file.added.saturating_add(added);
                file.removed = file.removed.saturating_add(removed);
                file.changes.push(change.clone());
            }
            None => files.push(ChangedFile {
                path: path.clone(),
                added,
                removed,
                changes: vec![change.clone()],
            }),
        }
    }
    files
}

/// The files each turn that changed any changed, in order.
fn turns_of(changes: &[Made]) -> Vec<TurnChanges> {
    let mut turns: Vec<Seq> = Vec::new();
    for turn in changes.iter().filter_map(|change| change.turn) {
        if !turns.contains(&turn) {
            turns.push(turn);
        }
    }
    let of = |turn: Seq| {
        changes
            .iter()
            .filter(move |change| change.turn == Some(turn))
    };
    turns
        .into_iter()
        .map(|turn| TurnChanges {
            turn,
            files: files_of(of(turn)),
        })
        .collect()
}

#[cfg(test)]
mod tests;
