//! glave's Postgres storage: the event log, the blob store and their keys for an
//! organization's one data store, the counterpart of the local SQLite file.
//!
//! Every call borrows a connection from a bounded pool and never blocks the async runtime.
//! Appends are transactions that enforce the log's gap-free numbering; the schema migrates
//! forward on open and refuses a newer one. Every query is bound to a scope. Event payloads
//! and blobs are sealed on write and opened on read (`specs/storage.md`); rows written before
//! sealing still read as they were stored.

mod blobs;
mod keys;
mod log;
mod schema;
mod spaces;
mod store;

pub use blobs::PostgresBlobs;
pub use keys::PostgresKeys;
pub use log::PostgresLog;
pub use spaces::PostgresSpaces;
pub use store::{PostgresKeyring, Store, StoreError};
pub use tokio_postgres::Config;
