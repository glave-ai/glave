//! glave's identity: who is calling. The verified [`Caller`], OIDC sign-in checked against an
//! identity provider's keys, local sign-in from this computer, and the person an issuer's
//! subject is. One crate, so the session API and the gateway service trust callers the same
//! way without depending on each other.

mod caller;
pub mod signin;

pub use caller::Caller;
pub use signin::{SignIn, SignInError, Trust, person};
