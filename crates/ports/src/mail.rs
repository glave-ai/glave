//! The email plug (`specs/model.md`, "Identity"): glave sends a few plain messages, an invite's
//! link among them, through whatever the deployment's configuration names (SMTP), and sends
//! none where it names nothing. A message is plain text to one address.

use thiserror::Error;

use crate::tools::BoxFuture;

/// One message: to one address, a subject and plain text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mail {
    pub to: String,
    pub subject: String,
    pub text: String,
}

/// Why a message was not sent.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum MailError {
    /// The address is not one a message can go to.
    #[error("{0:?} is not an email address")]
    Address(String),
    /// The mail server refused it, or could not be reached in time.
    #[error("the message could not be sent: {0}")]
    Send(String),
}

/// Where glave's messages go out.
pub trait Mailer: Send + Sync {
    /// Whether a message can go to `to` at all, checked before anything else is done.
    fn reaches(&self, to: &str) -> bool;

    /// Sends `mail`, answered once the mail server took it.
    fn send<'a>(&'a self, mail: &'a Mail) -> BoxFuture<'a, Result<(), MailError>>;
}
