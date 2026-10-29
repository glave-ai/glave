//! What glave emails (`specs/model.md`, "Identity"): invites, sign-ups' links, and later the
//! links that get someone back in, each to one address as glave reads it, never a name around
//! it, and at most one a minute to an address, counted in the ledger that records what was
//! sent, so every server on the store counts the same.

use glave_ports::{BlobStore, EventLog, Mail, MailError, Mailer, ModelProvider};
use jiff::{SignedDuration, Timestamp};

use crate::protocol::{ApiError, code};
use crate::server::Server;
use crate::words::Sentence;

/// How long after a message to an address the next may go there.
pub(crate) const BETWEEN: SignedDuration = SignedDuration::from_secs(60);

/// Whether a message sent at `sent` is within [`BETWEEN`] of `now`, so no other may follow yet.
pub(crate) fn just_now(sent: Timestamp, now: Timestamp) -> bool {
    now.duration_since(sent) < BETWEEN
}

/// `to` as glave reads an address: one address, with no name, quotes, comment or space in it,
/// lowercase, else none. So every way of writing one mailbox's address counts as one.
fn read_address(to: &str) -> Option<String> {
    let to = to.trim();
    let plain = !to
        .chars()
        .any(|c| c.is_whitespace() || "<>\"(),;:[]\\".contains(c));
    let (local, domain) = to.rsplit_once('@')?;
    let well_formed = plain && !local.is_empty() && !local.contains('@') && domain.contains('.');
    well_formed.then(|| to.to_lowercase())
}

impl<P, L, B> Server<P, L, B>
where
    P: ModelProvider + 'static,
    L: EventLog + 'static,
    B: BlobStore + 'static,
{
    /// What sends a link to `to`, where the link leads, and the address as glave reads it:
    /// refused where this server sends no email, or is reached from nowhere a link could lead
    /// to, and for anything but one address its mail server takes.
    pub(crate) fn sender(&self, to: &str) -> Result<(&dyn Mailer, String, String), ApiError> {
        let (Some(mailer), Some(origin)) = (self.inner.mail.as_deref(), &self.inner.origin) else {
            return Err(ApiError::new(code::NOT_AVAILABLE, Sentence::NoEmail));
        };
        let address = read_address(to)
            .filter(|address| mailer.reaches(address))
            .ok_or_else(|| ApiError::new(code::INVALID_PARAMS, Sentence::NotAnEmail))?;
        let origin = origin.to_string();
        Ok((mailer, origin.trim_end_matches('/').to_owned(), address))
    }

    /// Sends `mail` through `mailer`, saying what the mail server said in the log, never the
    /// address, and to the person only that it was not sent.
    pub(crate) async fn send(&self, mailer: &dyn Mailer, mail: &Mail) -> Result<(), ApiError> {
        mailer.send(mail).await.map_err(|error| {
            if let MailError::Send(reason) = &error {
                tracing::warn!(%reason, "an email was not sent");
            }
            ApiError::new(code::NOT_AVAILABLE, Sentence::MailNotSent)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_address_is_read_one_way_and_nothing_but_an_address_is_one() {
        assert_eq!(
            read_address(" Ada@Example.COM "),
            Some("ada@example.com".into())
        );
        for refused in [
            "Ada <ada@example.com>",
            "<ada@example.com>",
            "\"ada\"@example.com",
            "ada @example.com",
            "ada@example",
            "@example.com",
            "ada@@example.com",
            "ada@example.com, bea@example.com",
        ] {
            assert_eq!(read_address(refused), None, "{refused}");
        }
    }
}
