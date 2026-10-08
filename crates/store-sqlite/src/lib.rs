//! glave's local storage on one SQLite file: the event log, the blob store, and which space
//! each project folder belongs to.
//!
//! Every call runs off the async runtime. Appends are transactions that enforce the log's
//! gap-free numbering; the schema migrates forward on open and refuses a newer one.

mod blobs;
mod log;
mod schema;
mod store;

pub use blobs::SqliteBlobs;
pub use log::SqliteLog;
pub use store::{Store, StoreError};

#[cfg(test)]
mod tests;
