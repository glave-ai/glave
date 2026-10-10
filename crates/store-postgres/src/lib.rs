//! glave's Postgres storage: the event log and the blob store for an organization's one data
//! store, the counterpart of the local SQLite file.
//!
//! Every call borrows a connection from a bounded pool and never blocks the async runtime.
//! Appends are transactions that enforce the log's gap-free numbering; the schema migrates
//! forward on open and refuses a newer one. Every query is bound to a scope.

mod blobs;
mod log;
mod schema;
mod store;

pub use blobs::PostgresBlobs;
pub use log::PostgresLog;
pub use store::{Store, StoreError};
pub use tokio_postgres::Config;
