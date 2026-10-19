//! glave's local storage on one SQLite file: the event log, the blob store, their keys, and
//! the projects, with the folders that lead to them.
//!
//! Every call runs off the async runtime. Appends are transactions that enforce the log's
//! gap-free numbering; a new file gets the one schema, and a file of another schema is refused.
//! Event payloads and blobs are sealed on write and opened on read (`specs/storage.md`).

mod blobs;
mod keys;
mod log;
mod projects;
mod schema;
mod store;
mod views;

pub use blobs::SqliteBlobs;
pub use keys::SqliteKeys;
pub use log::SqliteLog;
pub use projects::SqliteProjects;
pub use store::{SqliteKeyring, Store, StoreError};

#[cfg(test)]
mod tests;
