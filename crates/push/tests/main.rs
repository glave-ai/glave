//! glave-push's tests, one binary: pushes encrypted as RFC 8291's example says and read back by
//! its browser, signed with the server's key, sent to a push service on this computer (the
//! `test-service` feature) that takes them, says a subscription is gone or refuses them; and
//! the subscriptions a push may go to.

#![cfg(test)]

mod example;
mod sending;
mod subscriptions;
