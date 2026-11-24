//! The messages of the session API (`specs/session.md`): requests by method (`request.rs`, its one
//! table of methods), their replies, the notifications a turn sends, and errors. Every transport
//! carries these same types; in-process they are passed as they are, elsewhere as JSON-RPC 2.0.

mod acts_on;
mod appearance;
mod changes;
pub mod code;
mod connections;
mod connectors;
mod credentials;
mod credits;
mod error;
mod exports;
mod files;
mod library;
mod live;
mod made;
mod meta;
mod methods;
mod models;
mod notification;
mod params;
mod passkeys;
mod people;
mod plans;
mod projects;
mod providers;
mod push;
mod reply;
mod request;
mod secrets;
mod settings;
mod shares;
mod sources;
mod steps;
mod tasks;
mod told;
mod trust;

pub use exports::*;
pub(crate) use request::RESULTS;
pub use request::Request;

/// The protocol version this server speaks.
pub const PROTOCOL: &str = "2026-10";
