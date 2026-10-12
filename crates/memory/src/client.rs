//! glave's side of its own memory server: an MCP client over an in-process connection, as the
//! harness's [`Memory`]. The model's tools and the current memories go through MCP, each
//! request carrying who and where in its `_meta`; the person's own controls (listing, moving,
//! restoring) are their actions, not the model's, and go to the store directly.
//!
//! With a tidier ([`GlaveMemory::tidy_with`]), tidy-up runs in the background on a task this
//! owns: asked after each save or edit, and by the host when it starts, it tidies each ledger
//! that is due (a week since the last time, or past 90% of its budget).

use std::sync::{Arc, Mutex};

use glave_event::{Arguments, MemoryId, Principal};
use glave_harness::{
    Access, Control, MEMORY_SERVER, Memory, MemoryChange, MemoryContext, MemoryError, MemoryOutput,
    Remembered, Shown,
};
use glave_mcp::{Limits, McpServer, tool_output};
use glave_ports::{BoxFuture, EventLog, NeverCancelled, Scope, ToolError, ToolInfo, ToolSource};
use rmcp::ServiceExt as _;
use rmcp::model::ResourceContents;
use serde::Deserialize;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use crate::memories::{Memories, Remembering, Tidier, Writer};
use crate::rules::SpaceMemory;
use crate::server::{Context, MemoryServer, RESOURCE};

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
    pub fn tidy_with(&self, tidier: Arc<dyn Tidier>) {
        let (sender, mut asked) = mpsc::channel::<Scope>(TIDY_QUEUE);
        let memories = self.memories.clone();
        let task = tokio::spawn(async move {
            while let Some(ledger) = asked.recv().await {
                // A failed tidy-up changes nothing it did not record; the next one tries again.
                if memories.tidy_due(ledger).await.unwrap_or(false) {
                    let _ = memories.tidy(ledger, tidier.as_ref()).await;
                }
            }
        });
        let previous = self
            .tidying
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .replace((sender, task));
        if let Some((_, task)) = previous {
            task.abort();
        }
    }

    /// Asks for `ledgers` to be tidied, when they are due and a tidier was given.
    pub fn tidy_soon(&self, ledgers: &[Scope]) {
        let tidying = self
            .tidying
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some((sender, _)) = tidying.as_ref() {
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
        let tidying = self
            .tidying
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take();
        if let Some((_, task)) = tidying {
            task.abort();
            let _ = task.await;
        }
        self.client.close().await;
        let server = self
            .server
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take();
        if let Some(server) = server {
            server.abort();
            let _ = server.await;
        }
    }

    async fn read(&self, on: &MemoryContext, uri: &str) -> Result<Vec<Remembered>, MemoryError> {
        Ok(self.listing(on, uri).await?.memories)
    }

    async fn listing(&self, on: &MemoryContext, uri: &str) -> Result<Listed, MemoryError> {
        let contents = self
            .client
            .read_with(uri, Context::of(on).meta(), &NeverCancelled)
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
        let meta = Context::of(on).meta();
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
        MemoryOutput { output, change }
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
        if let Some(server) = self
            .server
            .get_mut()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take()
        {
            server.abort();
        }
        if let Some((_, task)) = self
            .tidying
            .get_mut()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take()
        {
            task.abort();
        }
    }
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
        self.memories.rules().setting(on.scope.space) != SpaceMemory::Off
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
