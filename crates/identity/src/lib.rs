//! glave's identity: who is calling. The verified caller (`glave_ports::Caller`), OIDC sign-in
//! checked against an identity provider's keys, local sign-in from this computer, and the
//! person an issuer's subject is. One crate, so the session API and the gateway service trust callers the same
//! way without depending on each other.

pub mod signin;
#[cfg(any(test, feature = "fake-authenticator"))]
pub mod testing;

pub use signin::{Door, SignIn, SignInError, Trust, person};
