//! gateway-service's integration tests, one program: each file is a module of it.

#![cfg(test)]

#[path = "../../session/tests/support/issuer.rs"]
#[expect(
    dead_code,
    reason = "the session's test issuer; these tests use some of it"
)]
mod issuer;
#[path = "support/world.rs"]
mod world;

mod catalog;
mod surfaces;
