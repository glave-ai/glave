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

use anyhow::{anyhow, bail, Context, Result};
use serde::{Deserialize, Serialize};

use crate::ports::{Message, Workspace};

/// Hard cap on fork-chain depth — a backstop against a pathological or corrupt
/// chain even when there's no outright cycle.
const MAX_FORK_DEPTH: usize = 128;

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
    /// origin) plus its OWN messages. Truncates an existing file. CONTRACT: call
    /// once to initialize a session (new or freshly forked), then use
    /// `append_message`. Calling `save` after out-of-band appends would truncate
    /// and lose them — the in-memory `Session` is treated as the sole authority.
    pub fn save(&self, session: &Session) -> Result<()> {
        safe_id(&session.id)?;
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
        safe_id(id)?;
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
        self.load_guarded(id, &mut Vec::new())
    }

    /// Replay with a `chain` of ancestor ids, so a fork cycle or runaway chain
    /// errors instead of recursing into a stack overflow.
    fn load_guarded(&self, id: &str, chain: &mut Vec<String>) -> Result<Session> {
        // Validate EVERY id — including a fork `parent` deserialized from a file
        // header — so a planted parent pointer can't escape the store root.
        safe_id(id)?;
        if chain.iter().any(|p| p == id) {
            bail!("session fork cycle at {id:?} ({})", chain.join(" -> "));
        }
        if chain.len() >= MAX_FORK_DEPTH {
            bail!("session fork chain too deep (>{MAX_FORK_DEPTH}) at {id:?}");
        }
        let path = self.path(id);
        let file = File::open(&path).with_context(|| format!("opening {}", path.display()))?;
        let mut forked_from: Option<ForkPoint> = None;
        let mut own: Vec<Message> = Vec::new();
        let mut seen_message = false;
        let mut lines = BufReader::new(file).lines().peekable();
        while let Some(line) = lines.next() {
            let line = line.context("reading session line")?;
            if line.trim().is_empty() {
                continue;
            }
            let event: SessionEvent = match serde_json::from_str(&line) {
                Ok(ev) => ev,
                Err(e) => {
                    // Tolerate a torn LAST line (a crash mid-append); a parse
                    // error anywhere earlier is real corruption, so surface it.
                    if lines.peek().is_none() {
                        eprintln!(
                            "WARN: dropping unparsable trailing line in {} ({e})",
                            path.display()
                        );
                        break;
                    }
                    return Err(anyhow!("parsing session event in {}: {e}", path.display()));
                }
            };
            match event {
                // The header is only meaningful as the FIRST record; ignore a
                // stray later one (corruption / concatenated files).
                SessionEvent::Started { forked_from: f } => {
                    if !seen_message && forked_from.is_none() {
                        forked_from = f;
                    }
                }
                SessionEvent::Message(m) => {
                    seen_message = true;
                    own.push(m);
                }
            }
        }

        let (mut messages, inherited) = match &forked_from {
            Some(fork) => {
                chain.push(id.to_string());
                let parent = self.load_guarded(&fork.parent, chain)?;
                chain.pop();
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
    // Persist THEN reflect in memory, so a failed write never leaves the
    // in-memory transcript ahead of disk (which would corrupt a later resume).
    let user = Message::user(user_input);
    store.append_message(session.id(), &user)?;
    session.push(user);

    let replies = turn.respond(session.transcript(), workspace)?;
    for message in &replies {
        store.append_message(session.id(), message)?;
        session.push(message.clone());
    }
    Ok(replies)
}

/// Reject a session id that isn't a single safe path component, so a
/// caller-supplied id (e.g. a `--resume` value — this store is public API) can't
/// escape `root` via `..`, a separator, or an absolute path.
fn safe_id(id: &str) -> Result<()> {
    use std::path::Component;
    let mut comps = std::path::Path::new(id).components();
    match (comps.next(), comps.next()) {
        (Some(Component::Normal(c)), None) if !c.is_empty() => Ok(()),
        _ => bail!("invalid session id {id:?}: must be a single path component"),
    }
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

    /// A turn that actually edits the workspace — proves the loop + a tool-using
    /// turn + persistence + resume + fork compose end to end (no model, no GPU).
    struct EditingTurn;
    impl Turn for EditingTurn {
        fn respond(
            &mut self,
            transcript: &[Message],
            workspace: &mut dyn Workspace,
        ) -> Result<Vec<Message>> {
            let last = transcript.last().map_or_else(String::new, |m| m.content.clone());
            workspace.write_file("notes.txt", &format!("did: {last}"))?;
            Ok(vec![Message::assistant(format!("wrote notes.txt for: {last}"))])
        }
    }

    #[test]
    fn end_to_end_edit_resume_fork() -> Result<()> {
        use crate::testkit::InMemoryRuntime;
        let root = tmp_root("e2e");
        let store = SessionStore::new(&root);
        let mut ws = InMemoryRuntime::new();

        // 1) a session that edits the workspace.
        let mut session = Session::new("e2e");
        store.save(&session)?;
        step(&mut session, &store, &mut ws, &mut EditingTurn, "first task")?;
        assert_eq!(
            ws.file("notes.txt").map(String::as_str),
            Some("did: first task")
        );

        // 2) resume from disk — the transcript is intact.
        let resumed = store.load("e2e")?;
        assert_eq!(
            contents(&resumed),
            ["first task", "wrote notes.txt for: first task"]
        );

        // 3) fork at the user message (index 1), diverge, and persist.
        let mut forked = resumed.fork("e2e-fork", 1);
        store.save(&forked)?;
        step(&mut forked, &store, &mut ws, &mut EditingTurn, "other task")?;
        assert_eq!(
            contents(&store.load("e2e-fork")?),
            ["first task", "other task", "wrote notes.txt for: other task"]
        );
        // The parent's transcript is untouched by the fork.
        assert_eq!(
            contents(&store.load("e2e")?),
            ["first task", "wrote notes.txt for: first task"]
        );

        let _ = std::fs::remove_dir_all(&root);
        Ok(())
    }

    #[test]
    fn rejects_unsafe_session_ids() {
        let store = SessionStore::new(std::env::temp_dir().join("glave-safeid"));
        for bad in ["../escape", "a/b", "/abs", "..", ".", ""] {
            assert!(store.load(bad).is_err(), "load must reject {bad:?}");
            assert!(
                store.append_message(bad, &Message::user("x")).is_err(),
                "append must reject {bad:?}"
            );
        }
    }

    #[test]
    fn load_detects_fork_cycle() -> Result<()> {
        let root = tmp_root("cycle");
        let store = SessionStore::new(&root);
        std::fs::create_dir_all(&root)?;
        // A session whose fork parent is itself: must error, not recurse forever.
        std::fs::write(
            root.join("loop.jsonl"),
            "{\"kind\":\"started\",\"forked_from\":{\"parent\":\"loop\",\"at\":0}}\n",
        )?;
        assert!(store.load("loop").is_err());
        let _ = std::fs::remove_dir_all(&root);
        Ok(())
    }

    #[test]
    fn load_tolerates_torn_trailing_line() -> Result<()> {
        use std::io::Write;
        let root = tmp_root("torn");
        let store = SessionStore::new(&root);
        store.save(&Session::new("t"))?;
        store.append_message("t", &Message::user("intact"))?;
        // Simulate a crash mid-append: a partial trailing record (no newline).
        let mut f = std::fs::OpenOptions::new()
            .append(true)
            .open(root.join("t.jsonl"))?;
        f.write_all(b"{\"kind\":\"message\",\"role\":\"assi")?;
        drop(f);
        // load must succeed, dropping only the torn line.
        let loaded = store.load("t")?;
        assert_eq!(contents(&loaded), ["intact"]);
        let _ = std::fs::remove_dir_all(&root);
        Ok(())
    }

    #[test]
    fn load_rejects_traversal_via_fork_parent() -> Result<()> {
        let root = tmp_root("forktrav");
        let store = SessionStore::new(&root);
        std::fs::create_dir_all(&root)?;
        // Clean entry id, but the header points the fork parent OUTSIDE root —
        // the recursive load must validate it too, not just the entry id.
        std::fs::write(
            root.join("evil.jsonl"),
            "{\"kind\":\"started\",\"forked_from\":{\"parent\":\"../escape\",\"at\":0}}\n",
        )?;
        assert!(
            store.load("evil").is_err(),
            "a fork parent that escapes root must be rejected"
        );
        let _ = std::fs::remove_dir_all(&root);
        Ok(())
    }
}
