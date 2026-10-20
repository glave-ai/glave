//! glave's HTTPS of its own (`specs/one-server.md`, "HTTPS of its own"). Given the name people
//! reach it at, glave gets a certificate for it from an ACME authority, Let's Encrypt unless it
//! is told another, and proves the name with TLS-ALPN-01 on the port it serves HTTPS on, so
//! nothing else has to listen. It renews the certificate with a third of its life left, and
//! keeps the account and the certificate in the certificates ledger of the store, sealed, so a
//! restart serves at once without asking the authority. Until a certificate is in hand, the
//! only handshake it completes is the authority's challenge.

mod https;
mod kept;
mod order;
mod resolver;

pub use https::{Attempt, Https, HttpsError, Issued};
