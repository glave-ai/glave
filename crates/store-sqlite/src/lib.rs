//! glave's local storage on one SQLite file: the event log, the blob store, their keys, and
//! the spaces, with the folders that lead to them.
//!
//! Every call runs off the async runtime. Appends are transactions that enforce the log's
//! gap-free numbering; the schema migrates forward on open and refuses a newer one. Event
//! payloads and blobs are sealed on write and opened on read (`specs/storage.md`); rows
//! written before sealing still read as they were stored.

mod blobs;
mod keys;
mod log;
mod schema;
mod spaces;
mod store;
mod views;

pub use blobs::SqliteBlobs;
pub use keys::SqliteKeys;
pub use log::SqliteLog;
pub use spaces::SqliteSpaces;
pub use store::{SqliteKeyring, Store, StoreError};
pub use views::Rebuilt;

#[cfg(test)]
mod tests;
