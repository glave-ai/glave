//! What the session tests share: scripted answers, a server built from a `Setup`, the requests
//! most tests make, a fake identity provider, and a client of the HTTP transport.

pub(crate) mod following;
mod git;
pub(crate) mod issuer;
mod requests;
mod script;
mod setup;
pub(crate) mod web;

pub(crate) use git::*;
pub(crate) use glave_ports::block_on;
pub(crate) use requests::*;
pub(crate) use script::*;
pub(crate) use setup::*;
