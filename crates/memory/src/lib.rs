//! glave's memory (`specs/memory.md`): what it remembers from one thread for the next. An MCP
//! server, in-process and first-party, offers the model `remember`, `edit`, `forget` and
//! `recall` and the resource `glave://memory`; the store behind it keeps each space's memories as events in
//! the space's memory ledger, each memory sealed under a data key of its own, so forgetting it
//! destroys the key and every version with it. Who and where come from the session, never from
//! the model, and every read is scoped by the rules: a shared space reads the person's memory
//! and its own, an isolated one only its own, one that is off nothing.

mod book;
mod client;
#[cfg(any(test, feature = "conformance"))]
pub mod conformance;
mod memories;
mod rules;
mod server;
mod shred;
mod similar;

pub use client::GlaveMemory;
pub use memories::{
    DEFAULT_HITS, Housekept, MOST_HITS, Memories, ModelTidier, Recalled, Remembering, Source,
    Tidied, Tidier, Writer,
};
pub use rules::{Personal, Rules, SpaceMemory};
pub use server::{CONTEXT, MemoryServer, RESOURCE};
pub use shred::{NoKeys, Shredder};
