//! glave's computers (`specs/computers.md`): this machine as a computer, the environments
//! computers are made from, and the JSON-RPC protocol that serves a computer to a harness
//! elsewhere and reaches one from here.
//!
//! [`LocalComputers`] runs work on the machine it is on. [`serve`] offers any computers over
//! one connection, and [`RemoteComputers`] is the other end of it: both pass the same
//! conformance suite, so a harness cannot tell a computer here from one across the wire.

mod cancel;
pub mod environment;
mod local;
mod remote;
mod serve;
mod wire;

pub use cancel::Switch;
pub use environment::{EnvironmentError, Found, read_environment};
pub use local::LocalComputers;
pub use remote::RemoteComputers;
pub use serve::{serve, serve_tcp};
