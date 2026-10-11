//! glave's side of its own memory server: an MCP client over an in-process connection, as the
//! harness's [`Memory`]. The model's tools and the current memories go through MCP, each
//! request carrying who and where in its `_meta`; the person's own controls (listing, moving,
//! restoring) are their actions, not the model's, and go to the store directly.

use std::sync::Mutex;

use glave_event::{Arguments, MemoryId, Principal};
use glave_harness::{
    Access, Control, MEMORY_SERVER, Memory, MemoryChange, MemoryContext, MemoryError, MemoryOutput,
    Remembered, Shown,
};
use glave_mcp::{Limits, McpServer, tool_output};
use glave_ports::{BoxFuture, Cancellation, EventLog, ToolError, ToolInfo, ToolSource};
use rmcp::ServiceExt as _;
use rmcp::model::ResourceContents;
use serde::Deserialize;
use tokio::task::JoinHandle;

use crate::memories::{Memories, Remembering, Writer};
use crate::rules::SpaceMemory;
use crate::server::{Context, MemoryServer, RESOURCE};

/// Bytes the in-process connection buffers each way.
const BUFFER: usize = 64 * 1024;

/// Memory as glave runs it: its own server, in-process, over the data store.
pub struct GlaveMemory<L: EventLog + 'static> {
    memories: Memories<L>,
    client: McpServer,
    /// The server, serving until it is closed or this is dropped.
    server: Mutex<Option<JoinHandle<()>>>,
}

#[derive(Deserialize)]
struct Listed {
    memories: Vec<Remembered>,
}

/// A read that is never cancelled: memory reads are short and bounded by the call limit.
struct Never;

impl Cancellation for Never {
    fn is_cancelled(&self) -> bool {
        false
    }

    fn cancelled(&self) -> BoxFuture<'_, ()> {
        Box::pin(std::future::pending())
    }
}

impl<L: EventLog + 'static> GlaveMemory<L> {
    /// Starts the server on `memories` and connects to it.
    pub async fn start(memories: Memories<L>) -> Result<Self, MemoryError> {
        let (ours, theirs) = tokio::io::duplex(BUFFER);
        // The server settles on a lifecycle only at the first request after `discover`, so it
        // serves on its own task while the client connects.
        let served = MemoryServer::new(memories.clone());
        let server = tokio::spawn(async move {
            if let Ok(running) = served.serve(theirs).await {
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
        })
    }

    /// The store behind the server.
    pub fn memories(&self) -> &Memories<L> {
        &self.memories
    }

    /// Closes the connection and stops the server.
    pub async fn close(&self) {
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
        let contents = self
            .client
            .read_with(uri, Context::of(on).meta(), &Never)
            .await
            .map_err(MemoryError::Storage)?;
        let mut memories = Vec::new();
        for content in contents {
            if let ResourceContents::TextResourceContents { text, .. } = content {
                let listed: Listed = serde_json::from_str(&text)
                    .map_err(|error| MemoryError::Storage(error.to_string()))?;
                memories.extend(listed.memories);
            }
        }
        Ok(memories)
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
        let output = tool_output(&result);
        let change = if output.is_error {
            None
        } else {
            result
                .structured_content
                .as_ref()
                .and_then(|structured| change(&tool, structured))
        };
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

    fn control<'a>(
        &'a self,
        by: &'a Access,
        control: Control,
    ) -> BoxFuture<'a, Result<Vec<Remembered>, MemoryError>> {
        Box::pin(self.controlled(by, control))
    }
}
