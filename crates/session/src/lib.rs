//! glave's session API: the one boundary every surface uses to run threads and turns. The
//! server keeps the methods and notifications of `specs/session.md`; each transport, from
//! in-process to the network, carries the same messages. Network transports need a sign-in.

mod approvals;
mod background;
mod browsers;
pub mod client;
mod credentials;
mod cron;
mod dispatch;
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
pub mod protocol;
mod providers;
mod reread;
pub mod rpc;
mod schedule;
pub mod schema;
mod server;
mod spaces;
mod standing;
mod steer;
mod stopping;
mod tasks;
mod titles;
mod trace;
mod trust;
mod turn;

pub use approvals::Answering;
pub use credentials::{GitHosts, NewCredential};
pub use dispatch::Answered;
pub use glave_identity::signin;
pub use ledger::{accounts_ledger, is_ledger, space_ledger};
pub use providers::{Models, NewProvider, Providers};
pub use reread::{Apply, Changed, Rereads, Runs};
pub use server::{OnWorkers, Server, Wiring};
pub use spaces::PERSONAL_NAME;
pub use tasks::TaskLimits;
pub use trust::ProjectTrust;
pub use turn::{TurnItem, TurnStream};
