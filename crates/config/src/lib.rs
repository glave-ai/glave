//! glave's config and policy (`specs/policy.md`): the choices people and projects make in
//! `config.toml`, the constraints organizations set in `policy.toml`, the trust a project needs
//! before what it brings runs, and `glave policy explain`. A library, so the CLI and the session
//! server read them the same way.

mod error;
pub mod policy;
pub mod trust;

pub use error::{Error, Result};
