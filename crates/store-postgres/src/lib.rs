//! glave's Postgres storage: the event log, the blob store, their keys, the leases work in
//! flight runs under and the directory sign-in finds people in, for an organization's one data store, the counterpart of the local SQLite
//! file.
//!
//! Every call borrows a connection from a bounded pool and never blocks the async runtime.
//! Appends are transactions that enforce the log's gap-free numbering; a new database gets the
//! one schema, and a database of another schema is refused. Every query is bound to a scope.
//! Event payloads and blobs are sealed on write and opened on read (`specs/storage.md`).

mod blobs;
mod directory;
mod keys;
mod leases;
mod log;
mod projects;
mod schema;
mod store;
mod tls;
mod views;

pub use blobs::PostgresBlobs;
pub use directory::PostgresDirectory;
pub use keys::PostgresKeys;
pub use leases::PostgresLeases;
pub use log::PostgresLog;
pub use projects::PostgresProjects;
pub use store::{PostgresKeyring, Store, StoreError};
pub use tokio_postgres::Config;
