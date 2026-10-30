//! What glave emails (`specs/model.md`, "Identity"): invites, sign-ups' links, links that
//! get someone back in and the notice of a passkey made by one, each to one address as glave
//! reads it, never a name around it, and at most one a minute to an address, counted in the
//! ledger that records what was sent, so every server on the store counts the same. Each one
//! someone asks for over the network counts against their network too, in the sign-ups
//! ledger: [`PER_NETWORK`] an hour. A link is sent after the answer, from a task the server
//! owns, so how long the answer takes says nothing of whether the address has an account.

use std::net::IpAddr;
use std::sync::Arc;

use glave_event::MailAsked;
use glave_identity::signin::kept;
use glave_ports::{BlobStore, Caller, EventLog, Mail, Mailer, ModelProvider};
use jiff::{SignedDuration, Timestamp};

use crate::protocol::{ApiError, code};
use crate::server::Server;
use crate::words::Sentence;

/// How long after a message to an address the next may go there.
pub(crate) const BETWEEN: SignedDuration = SignedDuration::from_secs(60);

/// How many emails one network may ask for within [`ASKED_WITHIN`]: a household or an office
/// signing up together, or an admin inviting a team by email; not a stranger's way to mail
/// the world from glave.
const PER_NETWORK: usize = 20;

/// The while [`PER_NETWORK`] counts over.
pub(crate) const ASKED_WITHIN: SignedDuration = SignedDuration::from_hours(1);

/// How many emails the networks of one IPv6 /48 together may ask for within
/// [`ASKED_WITHIN`]: one person may be given a whole /48, and with it 65,536 /64s.
const PER_BLOCK: usize = 100;

/// The networks `from` is in, as glave counts what it asks for: an IPv4 address alone; for an
/// IPv6 one, the /64 it is in, which one household or machine is given whole, and the /48
/// that /64 is in.
fn networks(from: IpAddr) -> (String, Option<String>) {
    match from.to_canonical() {
        IpAddr::V4(v4) => (v4.to_string(), None),
        IpAddr::V6(v6) => {
            let [a, b, c, d, ..] = v6.segments();
            (
                format!("{a:x}:{b:x}:{c:x}:{d:x}::/64"),
                Some(format!("{a:x}:{b:x}:{c:x}::/48")),
            )
        }
    }
}

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
    pub(crate) fn sender(&self, to: &str) -> Result<(Arc<dyn Mailer>, String, String), ApiError> {
        let (Some(mailer), Some(origin)) = (&self.inner.mail, &self.inner.origin) else {
            return Err(ApiError::new(code::NOT_AVAILABLE, Sentence::NoEmail));
        };
        let address = read_address(to)
            .filter(|address| mailer.reaches(address))
            .ok_or_else(|| ApiError::new(code::INVALID_PARAMS, Sentence::NotAnEmail))?;
        let origin = origin.to_string();
        let mailer = Arc::clone(mailer);
        Ok((mailer, origin.trim_end_matches('/').to_owned(), address))
    }

    /// Sends `mail` through `mailer`, saying in the log only that it was not sent, since what a
    /// mail server says of a refusal often names the address, and to the person the same.
    pub(crate) async fn send(&self, mailer: &dyn Mailer, mail: &Mail) -> Result<(), ApiError> {
        mailer.send(mail).await.map_err(|_| {
            tracing::warn!("an email was not sent");
            ApiError::new(code::NOT_AVAILABLE, Sentence::MailNotSent)
        })
    }

    /// Sends `mail`, if there is any, through `mailer` after the answer, from a task the
    /// server owns and waits for as it stops: started either way, so an answer that sends
    /// nothing takes as long as one that does. One that is not sent is said in the log alone,
    /// as [`Server::send`] says it.
    pub(crate) fn send_later(&self, mailer: Arc<dyn Mailer>, mail: Option<Mail>) {
        self.own(tokio::spawn(async move {
            if let Some(mail) = mail
                && mailer.send(&mail).await.is_err()
            {
                tracing::warn!("an email was not sent");
            }
        }));
    }

    /// Counts an email `caller` asks for against their network: refused once it asked for
    /// [`PER_NETWORK`] within the hour, or an IPv6 address's /48 for [`PER_BLOCK`], from any
    /// server on the store. One asked for on this computer, in process or over stdio, counts
    /// against nothing.
    pub(crate) async fn count_asked(&self, caller: &Caller) -> Result<(), ApiError> {
        let Some(from) = caller.from else {
            return Ok(());
        };
        let (network, block) = networks(from);
        let asked = MailAsked {
            from: kept(&network),
            within: block.as_deref().map(kept),
        };
        let now = self.inner.harness.clock().now();
        self.record_signups(|signups| {
            let (theirs, their_block) = signups.asked(&asked, now);
            if theirs >= PER_NETWORK || their_block >= PER_BLOCK {
                return Err(ApiError::new(code::LIMIT, Sentence::AskedTooOften));
            }
            Ok(Some(asked.clone()))
        })
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_network_is_an_ipv4_address_or_an_ipv6_one_s_64_within_its_48() {
        let one = |text: &str| networks(text.parse().unwrap());
        assert_eq!(one("203.0.113.7"), ("203.0.113.7".into(), None));
        let carried = one("::ffff:203.0.113.7").0;
        assert_eq!(carried, "203.0.113.7", "as the IPv4 it carries");
        assert_eq!(
            one("2001:db8:1:2:3:4:5:6"),
            ("2001:db8:1:2::/64".into(), Some("2001:db8:1::/48".into()))
        );
        assert_eq!(one("2001:db8:1:2::9"), one("2001:db8:1:2:ffff::1"));
        assert_ne!(one("2001:db8:1:2::9").0, one("2001:db8:1:3::9").0);
        assert_eq!(one("2001:db8:1:2::9").1, one("2001:db8:1:3::9").1);
    }

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
