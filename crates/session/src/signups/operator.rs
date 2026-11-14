//! Sign-up links whoever runs the server makes (`specs/model.md`, "Identity"): from the
//! server's computer, to hand to one person, where it keeps glave accounts at a public origin,
//! whether or not it takes sign-ups or sends email. Each is kept in the sign-ups ledger as an
//! emailed one is, as the SHA-256 of its code, and is opened by the same passkey step, which
//! makes the account and its organization the same way; but it goes to no address and carries
//! none, as an invite's link need not, so the account has no email until its person gives one.
//! Handed over rather than mailed, it lasts as an invite does, and it opens while the server
//! takes no sign-ups.

use glave_event::{At, Secret, SignupStarted};
use glave_identity::signin::{kept, one_time_code};
use glave_ports::{BlobStore, Caller, EventLog, ModelProvider};
use jiff::SignedDuration;

use super::{MOST_WAITING, link};
use crate::protocol::{ApiError, Reply, code};
use crate::providers::local;
use crate::server::Server;
use crate::words::Sentence;

/// How long a link whoever runs the server made works: as long as an invite, since it goes to
/// a person through a chat or an email, who may not open it until after a weekend.
const LIFETIME: SignedDuration = SignedDuration::from_hours(7 * 24);

impl<P, L, B> Server<P, L, B>
where
    P: ModelProvider + 'static,
    L: EventLog + 'static,
    B: BlobStore + 'static,
{
    /// Whether this server makes organizations by sign-up links of whoever runs it: it keeps
    /// glave accounts at a public origin.
    pub(crate) fn links_signups(&self) -> bool {
        self.inner.accounts.is_some() && self.inner.origin.is_some()
    }

    /// Answers `signup/link` for `caller`: a new sign-up link, good once and for [`LIFETIME`],
    /// answered this once. Only for whoever runs the server, from its computer.
    pub(crate) async fn make_signup_link(&self, caller: &Caller) -> Result<Reply, ApiError> {
        let (true, Some(origin)) = (self.links_signups(), &self.inner.origin) else {
            return Err(ApiError::new(code::NOT_AVAILABLE, Sentence::NoSignupLinks));
        };
        if !local(caller) {
            return Err(ApiError::new(
                code::NOT_ALLOWED,
                Sentence::OperatorLinksSignups,
            ));
        }
        let secret = one_time_code()?;
        let code = secret.expose();
        let now = self.inner.harness.clock().now();
        let expires = At::from(
            now.checked_add(LIFETIME)
                .map_err(|e| ApiError::new(code::INTERNAL, e.to_string()))?,
        );
        let started = SignupStarted {
            hash: kept(code),
            to: None,
            expires,
            recovery: false,
        };
        self.record_signups(|signups| {
            if signups.waiting.len() >= MOST_WAITING {
                return Err(ApiError::new(code::NOT_AVAILABLE, Sentence::SignupsFull));
            }
            Ok(Some(started.clone()))
        })
        .await?;
        let origin = origin.as_str().trim_end_matches('/');
        Ok(Reply::SignupLinked {
            link: Secret::new(link(origin, "signup", code)),
            expires,
        })
    }
}
