//! Memory in the harness (`specs/memory.md`), through the memory port
//! (`glave_ports::memories`): the model's calls of its tools, and what each turn is shown.
//!
//! What the model is shown is never stored in the thread: the thread records which memories,
//! at which version, each turn was shown (in the turn's configuration), and the start of the
//! conversation is rebuilt from those versions, so it is never rewritten and forgetting a
//! memory really removes it.

mod call;
mod rebuild;
mod render;
mod state;

pub use call::{MemoryVerb, REMEMBER_CAUTION};
pub(crate) use state::{ConfigMemory, MemoryState};
