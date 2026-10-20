//! glave's session API: the one boundary every surface uses to run threads and turns. The
//! server keeps the methods and notifications of `specs/session.md`; each transport, from
//! in-process to the network, carries the same messages. Network transports need a sign-in.

mod accounts;
mod approvals;
mod background;
mod browsers;
pub mod client;
mod credentials;
mod cron;
mod cursor;
mod dispatch;
mod errors;
mod folders;
mod goal;
mod gone;
mod held;
pub mod http;
mod hub;
mod keyed;
mod ledger;
mod memory;
mod models;
mod modes;
mod projects;
pub mod protocol;
mod providers;
mod reread;
pub mod rpc;
mod schedule;
pub mod schema;
mod server;
mod standing;
mod steer;
mod stopping;
mod tasks;
mod titles;
mod trace;
mod trust;
mod turn;

pub use accounts::{AccountLedger, Accounts, AccountsError};
pub use approvals::Answering;
pub use credentials::{GitHosts, NewCredential};
pub use dispatch::Answered;
pub use glave_identity::signin;
pub use projects::PERSONAL_NAME;
pub use providers::{Models, NewProvider, Providers, Refreshed};
pub use reread::{Apply, Changed, Rereads, Runs};
pub use server::{OnWorkers, Server, Wiring};
pub use tasks::TaskLimits;
pub use trust::ProjectTrust;
pub use turn::{TurnItem, TurnStream};
