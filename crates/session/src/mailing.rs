//! What glave emails (`specs/model.md`, "Identity"): invites, sign-ups' links, and later the
//! links that get someone back in, each sent at most once a minute to one address, from any
//! server on the store, so no one can make glave flood a mailbox. The limit is a lease in the
//! store on the address's SHA-256, which lapses after the minute; the address is never kept.

use std::time::Duration;

use glave_identity::signin::kept;
use glave_ports::{BlobStore, EventLog, Holder, Leased, Mail, MailError, Mailer, ModelProvider};

use crate::protocol::{ApiError, code};
use crate::server::Server;
use crate::words::Sentence;

/// How long after a message to an address the next may go there.
const BETWEEN: Duration = Duration::from_secs(60);

impl<P, L, B> Server<P, L, B>
where
    P: ModelProvider + 'static,
    L: EventLog + 'static,
    B: BlobStore + 'static,
{
    /// Whether a message may go to `to` now: none went there in the last minute, from any
    /// server on the store. Taking the lease is what counts this one.
    pub(crate) async fn may_mail(&self, to: &str) -> Result<bool, ApiError> {
        let hash = kept(&to.trim().to_lowercase());
        let address = hash
            .get(..32)
            .and_then(|start| u128::from_str_radix(start, 16).ok())
            .unwrap_or_default();
        // A holder of its own each time, so this server's last message counts as another's.
        let harness = &self.inner.harness;
        let holder = Holder::new(harness.clock(), harness.entropy())?;
        let lease = self
            .inner
            .leases
            .acquire(self.inner.own, Leased::Mail(address), holder, BETWEEN)
            .await
            .map_err(|e| ApiError::new(code::INTERNAL, e.to_string()))?;
        Ok(lease.is_some())
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
