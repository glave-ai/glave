//! Sessions — the event-sourced conversation record.
//!
//! A session is a UUID + an append-only log of [`SessionEvent`]s. The log is the
//! source of truth; the model transcript is REBUILT by replaying it (never stored
//! as a per-turn snapshot), so storage is O(messages), not O(turns²) — we log
//! events, never whole requests. A fork is a *pointer*: `(parent, index)` + the
//! fork's own events, so a shared prefix lives once and is recomposed on load.
//!
//! Observability (cost, tokens, timings) is the separate [`crate::event::RunEvent`]
//! trace; this log carries only what's needed to resume the conversation.

use std::fs::{create_dir_all, File, OpenOptions};
use std::io::{BufRead, BufReader, BufWriter, Write};
use std::path::PathBuf;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::ports::{Message, Workspace};

/// One appended fact in a session's log. Replaying these in order reconstructs
/// the conversation transcript.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SessionEvent {
    /// Session header — written once, first. Records the fork origin, if any.
    Started {
        #[serde(skip_serializing_if = "Option::is_none")]
        forked_from: Option<ForkPoint>,
    },
    /// A message appended to the conversation — the resume-bearing record.
    Message(Message),
}

/// Where a forked session branched from: the parent's id and how many of the
/// parent's messages it inherited.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ForkPoint {
    pub parent: String,
    pub at: usize,
}

/// An in-memory session: its id, optional fork origin, and the full transcript
/// (inherited prefix + own messages). Pure — no I/O; persistence is
/// [`SessionStore`].
#[derive(Debug, Clone)]
pub struct Session {
    id: String,
    forked_from: Option<ForkPoint>,
    messages: Vec<Message>,
    /// How many leading messages were inherited from a parent (so never
    /// re-persisted — the fork stores a pointer, not a copy of the prefix).
    inherited: usize,
}

impl Session {
    /// A fresh, empty session.
    pub fn new(id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            forked_from: None,
            messages: Vec::new(),
            inherited: 0,
        }
    }

    #[must_use]
    pub fn id(&self) -> &str {
        &self.id
    }

    #[must_use]
    pub fn forked_from(&self) -> Option<&ForkPoint> {
        self.forked_from.as_ref()
    }

    /// Append a message to the conversation.
    pub fn push(&mut self, message: Message) {
        self.messages.push(message);
    }

    /// The full transcript so far — the array handed to the model on the next
    /// turn, and what a round-trip must reproduce.
    #[must_use]
    pub fn transcript(&self) -> &[Message] {
        &self.messages
    }

    /// Fork at `at` messages: a new session inheriting the first `at` of this
    /// session's transcript by POINTER (recorded as the origin, never
    /// re-persisted). `at` is clamped to the transcript length.
    #[must_use]
    pub fn fork(&self, new_id: impl Into<String>, at: usize) -> Self {
        let at = at.min(self.messages.len());
        let inherited: Vec<Message> = self.messages.iter().take(at).cloned().collect();
        Self {
            id: new_id.into(),
            forked_from: Some(ForkPoint {
                parent: self.id.clone(),
                at,
            }),
            messages: inherited,
            inherited: at,
        }
    }

    /// The messages this session OWNS (not inherited) — the only ones persisted.
    fn own_messages(&self) -> impl Iterator<Item = &Message> {
        self.messages.iter().skip(self.inherited)
    }
}

/// File-backed store: one append-only `<id>.jsonl` per session under `root`.
pub struct SessionStore {
    root: PathBuf,
}

impl SessionStore {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    /// The default session root: `~/.glave/sessions`. Callers scope by project by
    /// joining the project id onto this.
    pub fn default_root() -> Result<PathBuf> {
        let home = std::env::var_os("HOME").context("HOME is not set")?;
        Ok(PathBuf::from(home).join(".glave").join("sessions"))
    }

    fn path(&self, id: &str) -> PathBuf {
        self.root.join(format!("{id}.jsonl"))
    }

    /// Write a session from scratch: the `Started` header (carrying any fork
    /// origin) plus its OWN messages. Truncates an existing file.
    pub fn save(&self, session: &Session) -> Result<()> {
        create_dir_all(&self.root)
            .with_context(|| format!("creating session dir {}", self.root.display()))?;
        let path = self.path(&session.id);
        let file = File::create(&path).with_context(|| format!("creating {}", path.display()))?;
        let mut writer = BufWriter::new(file);
        write_event(
            &mut writer,
            &SessionEvent::Started {
                forked_from: session.forked_from.clone(),
            },
        )?;
        for message in session.own_messages() {
            write_event(&mut writer, &SessionEvent::Message(message.clone()))?;
        }
        writer.flush().context("flushing session")
    }

    /// Append one message to a session's log on disk (incremental, durable).
    pub fn append_message(&self, id: &str, message: &Message) -> Result<()> {
        create_dir_all(&self.root)
            .with_context(|| format!("creating session dir {}", self.root.display()))?;
        let path = self.path(id);
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .with_context(|| format!("opening {}", path.display()))?;
        let mut writer = BufWriter::new(file);
        write_event(&mut writer, &SessionEvent::Message(message.clone()))?;
        writer.flush().context("flushing session append")
    }

    /// Load a session and replay it into its full transcript. A fork recursively
    /// composes its parent's first `at` messages, then its own — the prefix is
    /// recomputed, never read from the fork's own file.
    pub fn load(&self, id: &str) -> Result<Session> {
        let path = self.path(id);
        let file = File::open(&path).with_context(|| format!("opening {}", path.display()))?;
        let mut forked_from: Option<ForkPoint> = None;
        let mut own: Vec<Message> = Vec::new();
        for line in BufReader::new(file).lines() {
            let line = line.context("reading session line")?;
            if line.trim().is_empty() {
                continue;
            }
            match serde_json::from_str::<SessionEvent>(&line)
                .with_context(|| format!("parsing session event in {}", path.display()))?
            {
                SessionEvent::Started { forked_from: f } => forked_from = f,
                SessionEvent::Message(m) => own.push(m),
            }
        }

        let (mut messages, inherited) = match &forked_from {
            Some(fork) => {
                let parent = self.load(&fork.parent)?;
                let at = fork.at.min(parent.messages.len());
                let prefix: Vec<Message> = parent.messages.iter().take(at).cloned().collect();
                (prefix, at)
            }
            None => (Vec::new(), 0),
        };
        messages.extend(own);

        Ok(Session {
            id: id.to_string(),
            forked_from,
            messages,
            inherited,
        })
    }
}

/// The seam between the session loop and whatever drives a turn — a model agent
/// in the product, a scripted fake in tests. Given the transcript so far and the
/// workspace, it returns the messages to append (the assistant turn + any tool
/// results). It never grades and never sees a `TaskSpec`.
pub trait Turn {
    fn respond(
        &mut self,
        transcript: &[Message],
        workspace: &mut dyn Workspace,
    ) -> Result<Vec<Message>>;
}

/// One step of the session loop: record the user's message, produce a turn over
/// the workspace, then append + persist everything (the user message first, so a
/// crash mid-turn still leaves the prompt recorded). Returns the messages the
/// turn produced, for the caller to display.
pub fn step(
    session: &mut Session,
    store: &SessionStore,
    workspace: &mut dyn Workspace,
    turn: &mut dyn Turn,
    user_input: &str,
) -> Result<Vec<Message>> {
    let user = Message::user(user_input);
    session.push(user.clone());
    store.append_message(session.id(), &user)?;

    let replies = turn.respond(session.transcript(), workspace)?;
    for message in &replies {
        session.push(message.clone());
        store.append_message(session.id(), message)?;
    }
    Ok(replies)
}

fn write_event(writer: &mut impl Write, event: &SessionEvent) -> Result<()> {
    let line = serde_json::to_string(event).context("serializing session event")?;
    writer.write_all(line.as_bytes())?;
    writer.write_all(b"\n")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ports::{Message, ToolCallRequest};

    /// A fresh, unique temp root per test (tests run in parallel).
    fn tmp_root(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("glave-sess-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    fn contents(s: &Session) -> Vec<String> {
        s.transcript().iter().map(|m| m.content.clone()).collect()
    }

    #[test]
    fn save_then_load_round_trips_the_transcript() -> Result<()> {
        let root = tmp_root("roundtrip");
        let store = SessionStore::new(&root);

        let mut s = Session::new("s1");
        s.push(Message::system("you are a coding agent"));
        s.push(Message::user("fix the bug"));
        s.push(Message::assistant_with_tool_calls(
            "",
            vec![ToolCallRequest {
                id: "c1".to_string(),
                name: "read".to_string(),
                arguments: r#"{"path":"x"}"#.to_string(),
            }],
        ));
        s.push(Message::tool_result("c1", "file contents"));
        store.save(&s)?;

        let loaded = store.load("s1")?;
        // Exact transcript reproduced — including tool_calls and tool_call_id.
        assert_eq!(loaded.transcript(), s.transcript());

        let _ = std::fs::remove_dir_all(&root);
        Ok(())
    }

    #[test]
    fn incremental_appends_reload_in_order() -> Result<()> {
        let root = tmp_root("incremental");
        let store = SessionStore::new(&root);

        store.save(&Session::new("s2"))?; // header only
        store.append_message("s2", &Message::user("one"))?;
        store.append_message("s2", &Message::assistant("two"))?;

        let loaded = store.load("s2")?;
        assert_eq!(contents(&loaded), ["one", "two"]);

        let _ = std::fs::remove_dir_all(&root);
        Ok(())
    }

    #[test]
    fn fork_replays_parent_prefix_plus_own() -> Result<()> {
        let root = tmp_root("fork");
        let store = SessionStore::new(&root);

        let mut parent = Session::new("parent");
        for w in ["m0", "m1", "m2", "m3"] {
            parent.push(Message::user(w));
        }
        store.save(&parent)?;

        let fork = parent.fork("fork", 2);
        store.save(&fork)?; // header (forked_from) + zero own messages
        store.append_message("fork", &Message::user("m2b"))?;

        let loaded = store.load("fork")?;
        // parent[0..2] + own.
        assert_eq!(contents(&loaded), ["m0", "m1", "m2b"]);
        // Parent untouched.
        assert_eq!(store.load("parent")?.transcript().len(), 4);

        // Pointer, not copy: the fork's file holds only the header + its 1 own
        // message — never the 2 inherited from the parent.
        let fork_lines = std::fs::read_to_string(root.join("fork.jsonl"))?
            .lines()
            .filter(|l| !l.trim().is_empty())
            .count();
        assert_eq!(fork_lines, 2);

        let _ = std::fs::remove_dir_all(&root);
        Ok(())
    }

    #[test]
    fn fork_at_beyond_len_clamps() {
        let mut parent = Session::new("p");
        parent.push(Message::user("only"));
        let fork = parent.fork("f", 99);
        assert_eq!(fork.transcript().len(), 1);
        assert_eq!(fork.forked_from().map(|f| f.at), Some(1));
    }

    /// A deterministic turn: reads the workspace + echoes the last user message,
    /// so the loop is exercised with zero model calls.
    struct ScriptedTurn;
    impl Turn for ScriptedTurn {
        fn respond(
            &mut self,
            transcript: &[Message],
            workspace: &mut dyn Workspace,
        ) -> Result<Vec<Message>> {
            let files = workspace.list_files()?.len();
            let last = transcript.last().map_or_else(String::new, |m| m.content.clone());
            Ok(vec![Message::assistant(format!("ack:{last}:files={files}"))])
        }
    }

    #[test]
    fn step_records_user_then_turn_and_persists() -> Result<()> {
        use crate::testkit::InMemoryRuntime;
        let root = tmp_root("loop");
        let store = SessionStore::new(&root);
        let mut ws = InMemoryRuntime::new().with_file("a.txt", "x");
        let mut session = Session::new("loop1");
        store.save(&session)?;

        let replies = step(&mut session, &store, &mut ws, &mut ScriptedTurn, "hello")?;
        assert_eq!(replies.len(), 1);

        // Reload: the user message and the produced turn persisted, in order.
        let loaded = store.load("loop1")?;
        assert_eq!(contents(&loaded), ["hello", "ack:hello:files=1"]);

        let _ = std::fs::remove_dir_all(&root);
        Ok(())
    }
}
