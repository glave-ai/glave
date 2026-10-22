//! glave's side of its own memory server: an MCP client over an in-process connection, as the
//! memory port ([`Memory`]). The model's tools and the current memories go through MCP, each
//! request carrying who and where in its `_meta`; the person's own controls (listing, moving,
//! restoring) are their actions, not the model's, and go to the store directly. A result that
//! showed the text of a memory or a past conversation is kept by reference (`reply.rs`), and
//! read again from the store when its thread is rebuilt.
//!
//! With a tidier ([`GlaveMemory::tidy_with`]), tidy-up runs in the background on a task this
//! owns: asked after each save or edit, and by the host when it starts, it tidies each ledger
//! that is due (a week since the last time, or past 90% of its budget).

use std::sync::{Mutex, MutexGuard, PoisonError};

use glave_event::{Arguments, MemoryId, Part, Principal};
use glave_mcp::{Limits, McpServer, tool_output};
use glave_ports::memories::ProjectMemory;
use glave_ports::memories::{
    Access, Control, Kept, MEMORY_SERVER, Memory, MemoryChange, MemoryContext, MemoryError,
    MemoryOutput, Remembered, Shown,
};
use glave_ports::{
    BoxFuture, EventLog, ModelProvider, NeverCancelled, Scope, ToolError, ToolInfo, ToolSource,
};
use rmcp::ServiceExt as _;
use rmcp::model::ResourceContents;
use serde::Deserialize;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use crate::memories::{Memories, Remembering, Tidier, Writer};
use crate::reply::{Reply, words};
use crate::server::{MemoryServer, REPLY, RESOURCE, UNSHOWN, meta};

/// Bytes the in-process connection buffers each way.
const BUFFER: usize = 64 * 1024;
/// Ledgers waiting to be looked at by tidy-up; more are dropped, as a run is due anyway.
const TIDY_QUEUE: usize = 16;

/// Memory as glave runs it: its own server, in-process, over the data store.
pub struct GlaveMemory<L: EventLog + 'static> {
    memories: Memories<L>,
    client: McpServer,
    /// The server, serving until it is closed or this is dropped.
    server: Mutex<Option<JoinHandle<()>>>,
    /// Tidy-up, when a tidier was given: where to ask for a ledger, and the task that tidies.
    tidying: Mutex<Option<(mpsc::Sender<Scope>, JoinHandle<()>)>>,
}

#[derive(Deserialize)]
struct Listed {
    memories: Vec<Remembered>,
    #[serde(default)]
    questions: Vec<[MemoryId; 2]>,
}

impl<L: EventLog + 'static> GlaveMemory<L> {
    /// Starts the server on `memories` and connects to it.
    pub async fn start(memories: Memories<L>) -> Result<Self, MemoryError> {
        let (ours, theirs) = tokio::io::duplex(BUFFER);
        // The server settles on a lifecycle only at the first request after `discover`, so it
        // serves on its own task while the client connects.
        let handler = MemoryServer::new(memories.clone());
        let server = tokio::spawn(async move {
            if let Ok(running) = handler.serve(theirs).await {
                let _ = running.waiting().await;
            }
        });
        let client = McpServer::connect(MEMORY_SERVER, ours, Limits::default());
        client
            .started()
            .await
            .map_err(|error| MemoryError::Storage(error.to_string()))?;
        Ok(Self {
            memories,
            client,
            server: Mutex::new(Some(server)),
            tidying: Mutex::new(None),
        })
    }

    /// Tidies in the background with `tidier` from now on: each ledger asked for with
    /// [`GlaveMemory::tidy_soon`], and those a save or an edit changes, when they are due.
    pub fn tidy_with<P: ModelProvider + 'static>(&self, tidier: Tidier<P>) {
        let (sender, mut asked) = mpsc::channel::<Scope>(TIDY_QUEUE);
        let memories = self.memories.clone();
        let task = tokio::spawn(async move {
            while let Some(ledger) = asked.recv().await {
                // A failed tidy-up changes nothing it did not record; the next one tries again.
                if memories.tidy_due(ledger).await.unwrap_or(false) {
                    let _ = memories.tidy(ledger, &tidier).await;
                }
            }
        });
        if let Some((_, task)) = lock(&self.tidying).replace((sender, task)) {
            task.abort();
        }
    }

    /// Asks for `ledgers` to be tidied, when they are due and a tidier was given.
    pub fn tidy_soon(&self, ledgers: &[Scope]) {
        if let Some((sender, _)) = lock(&self.tidying).as_ref() {
            for ledger in ledgers {
                let _ = sender.try_send(*ledger);
            }
        }
    }

    /// The store behind the server.
    pub fn memories(&self) -> &Memories<L> {
        &self.memories
    }

    /// Closes the connection and stops the server and any tidy-up. Tidy-up changes one memory
    /// at a time, merged text first, so one stopped halfway leaves a duplicate, never a loss.
    pub async fn close(&self) {
        let (tidying, server) = self.take_tasks();
        if let Some(task) = tidying {
            task.abort();
            let _ = task.await;
        }
        self.client.close().await;
        if let Some(server) = server {
            server.abort();
            let _ = server.await;
        }
    }

    /// Takes the tidy-up task and the server's, to stop them.
    fn take_tasks(&self) -> (Option<JoinHandle<()>>, Option<JoinHandle<()>>) {
        let tidying = lock(&self.tidying).take().map(|(_, task)| task);
        (tidying, lock(&self.server).take())
    }

    async fn read(&self, on: &MemoryContext, uri: &str) -> Result<Vec<Remembered>, MemoryError> {
        Ok(self.listing(on, uri).await?.memories)
    }

    async fn listing(&self, on: &MemoryContext, uri: &str) -> Result<Listed, MemoryError> {
        let contents = self
            .client
            .read_with(uri, meta(on), &NeverCancelled)
            .await
            .map_err(MemoryError::Storage)?;
        let mut all = Listed {
            memories: Vec::new(),
            questions: Vec::new(),
        };
        for content in contents {
            if let ResourceContents::TextResourceContents { text, .. } = content {
                let listed: Listed = serde_json::from_str(&text)
                    .map_err(|error| MemoryError::Storage(error.to_string()))?;
                all.memories.extend(listed.memories);
                all.questions.extend(listed.questions);
            }
        }
        Ok(all)
    }

    async fn call_tool(
        &self,
        on: &MemoryContext,
        name: &str,
        arguments: &Arguments,
    ) -> MemoryOutput {
        let Arguments::Json(arguments) = arguments else {
            return MemoryOutput::failed("The arguments were not valid JSON.");
        };
        let tool = name
            .trim()
            .to_ascii_lowercase()
            .rsplit("__")
            .next()
            .unwrap_or_default()
            .to_owned();
        let meta = meta(on);
        let result = match self
            .client
            .forward_with(&tool, Some(arguments.clone()), meta)
            .await
        {
            Ok(result) => result,
            Err(why) => return MemoryOutput::failed(why),
        };
        let mut output = tool_output(&result);
        let structured = result
            .structured_content
            .as_ref()
            .filter(|_| !output.is_error);
        // A recall that found untrusted output reads as untrusted content.
        output.untrusted = tool == "recall"
            && structured
                .and_then(|structured| structured.get("untrusted"))
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(false);
        let change = structured.and_then(|structured| change(&tool, structured));
        let kept = structured.and_then(kept_by_reference);
        if matches!(
            change,
            Some(MemoryChange::Saved { .. } | MemoryChange::Edited(_))
        ) {
            let ledgers: Vec<Scope> = self
                .memories
                .rules()
                .readable(on.person, on.scope)
                .into_iter()
                .map(|(_, ledger)| ledger)
                .collect();
            self.tidy_soon(&ledgers);
        }
        // Only `forget` shows no text: any other result that comes without what it showed, by
        // reference, shows none either rather than be kept with the text. What it did stands.
        if kept.is_none() && !output.is_error && tool != "forget" {
            output.parts = vec![Part::Text {
                text: UNSHOWN.to_owned(),
            }];
        }
        MemoryOutput {
            output,
            kept,
            change,
        }
    }

    /// The words of the results `shown` keeps by reference, with their texts as they read now;
    /// `None` for one that is not a result glave keeps.
    async fn reread_now(
        &self,
        on: &MemoryContext,
        shown: &[serde_json::Value],
    ) -> Result<Vec<Option<String>>, MemoryError> {
        let replies: Vec<Option<Reply>> = (shown.iter())
            .map(|shown| Reply::deserialize(shown).ok())
            .collect();
        let known: Vec<&Reply> = replies.iter().flatten().collect();
        let mut texts = self.memories.texts(on, &known).await?.into_iter();
        Ok(replies
            .iter()
            .map(|reply| Some(words(reply.as_ref()?, Some(&texts.next()?))))
            .collect())
    }

    async fn controlled(
        &self,
        by: &Access,
        control: Control,
    ) -> Result<Vec<Remembered>, MemoryError> {
        let person = by
            .person
            .or_else(|| self.memories.rules().personal.map(|p| p.person));
        let who = Writer {
            person: by.person,
            scope: by.scope,
            by: person.map_or_else(Principal::glave, Principal::person),
            source: None,
            untrusted_read: false,
        };
        let memories = &self.memories;
        match control {
            Control::List { scope } => memories.list_as(by.person, by.scope, scope).await,
            Control::Save { text, scope } => {
                match memories.remember(&who, &text, scope, None, true).await? {
                    Remembering::Saved { memory, .. } => Ok(vec![memory]),
                    Remembering::Similar(similar) => Ok(similar),
                }
            }
            Control::Edit { memory, text } => Ok(vec![memories.edit(&who, memory, &text).await?]),
            Control::Move { memory, scope } => {
                Ok(vec![memories.move_to(&who, memory, scope).await?])
            }
            Control::Forget { memory } => {
                let (_, entry) = memories.locate(by.person, by.scope, memory).await?;
                memories.forget(&who, memory).await?;
                Ok(entry.live().into_iter().collect())
            }
            Control::ForgetAll { scope } => memories.forget_all(&who, scope).await,
            Control::Restore { memory } => Ok(vec![memories.restore(&who, memory).await?]),
        }
    }
}

impl<L: EventLog + 'static> Drop for GlaveMemory<L> {
    fn drop(&mut self) {
        let (tidying, server) = self.take_tasks();
        for task in [tidying, server].into_iter().flatten() {
            task.abort();
        }
    }
}

/// `mutex`'s value, also after a panic elsewhere: what it guards is only ever replaced whole.
fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// What a thread's log keeps of a result that showed texts: its words without them, and what
/// it showed, by reference.
fn kept_by_reference(structured: &serde_json::Value) -> Option<Kept> {
    let shown = structured.get(REPLY)?;
    let reply = Reply::deserialize(shown).ok()?;
    Some(Kept {
        text: words(&reply, None),
        shown: shown.clone(),
    })
}

/// What a tool's structured result says it changed.
fn change(tool: &str, structured: &serde_json::Value) -> Option<MemoryChange> {
    #[derive(Deserialize)]
    struct Saved {
        memory: Remembered,
        #[serde(default)]
        replaced: Option<MemoryId>,
    }
    #[derive(Deserialize)]
    struct Forgotten {
        forgotten: MemoryId,
    }
    match tool {
        "remember" => serde_json::from_value::<Saved>(structured.clone())
            .ok()
            .map(|saved| MemoryChange::Saved {
                memory: saved.memory,
                replaced: saved.replaced,
            }),
        "edit" => serde_json::from_value::<Saved>(structured.clone())
            .ok()
            .map(|saved| MemoryChange::Edited(saved.memory)),
        "forget" => serde_json::from_value::<Forgotten>(structured.clone())
            .ok()
            .map(|forgotten| MemoryChange::Forgotten(forgotten.forgotten)),
        _ => None,
    }
}

impl<L: EventLog + 'static> Memory for GlaveMemory<L> {
    fn active(&self, on: &MemoryContext) -> bool {
        self.memories.rules().setting(on.scope.project()) != ProjectMemory::Off
    }

    fn tools(&self) -> BoxFuture<'_, Result<Vec<ToolInfo>, ToolError>> {
        self.client.tools()
    }

    fn call<'a>(
        &'a self,
        on: &'a MemoryContext,
        name: &'a str,
        arguments: &'a Arguments,
    ) -> BoxFuture<'a, MemoryOutput> {
        Box::pin(self.call_tool(on, name, arguments))
    }

    fn list<'a>(
        &'a self,
        on: &'a MemoryContext,
    ) -> BoxFuture<'a, Result<Vec<Remembered>, MemoryError>> {
        Box::pin(self.read(on, RESOURCE))
    }

    fn versions<'a>(
        &'a self,
        on: &'a MemoryContext,
        wanted: &'a [Shown],
    ) -> BoxFuture<'a, Result<Vec<Remembered>, MemoryError>> {
        Box::pin(async move {
            let mut found = Vec::new();
            for shown in wanted {
                let uri = format!("glave://memory/{}/{}", shown.memory, shown.version);
                // A forgotten memory can no longer be opened: it drops out.
                if let Ok(mut memories) = self.read(on, &uri).await {
                    found.append(&mut memories);
                }
            }
            Ok(found)
        })
    }

    fn reread<'a>(
        &'a self,
        on: &'a MemoryContext,
        shown: &'a [serde_json::Value],
    ) -> BoxFuture<'a, Result<Vec<Option<String>>, MemoryError>> {
        Box::pin(self.reread_now(on, shown))
    }

    fn questions<'a>(
        &'a self,
        on: &'a MemoryContext,
    ) -> BoxFuture<'a, Result<Vec<[MemoryId; 2]>, MemoryError>> {
        Box::pin(async move { Ok(self.listing(on, RESOURCE).await?.questions) })
    }

    fn control<'a>(
        &'a self,
        by: &'a Access,
        control: Control,
    ) -> BoxFuture<'a, Result<Vec<Remembered>, MemoryError>> {
        Box::pin(self.controlled(by, control))
    }
}
