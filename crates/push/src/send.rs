//! Sending one push (RFC 8030): the message encrypted to the subscription (RFC 8291), posted
//! to its endpoint with the server's VAPID token, within [`SEND_WITHIN`]. The push service
//! answers whether it took it, or that the subscription is gone (404 or 410), when the browser
//! unsubscribed or its person blocked notifications. Names are looked up keeping only public
//! addresses, no proxy and no redirect, so an endpoint never reaches the server's own network.

use std::fmt;
use std::sync::Arc;
use std::time::Duration;

use glave_ports::host::reachable;
use jiff::Timestamp;
use reqwest::dns::{Addrs, Name, Resolve, Resolving};
use reqwest::{Client, StatusCode, redirect};
use thiserror::Error;
use web_push_native::WebPushBuilder;

use crate::key::VapidKey;
use crate::subscription::{Refused, Subscription};

/// The longest a push may take, from the name looked up to the push service's answer.
const SEND_WITHIN: Duration = Duration::from_secs(30);

/// How long a token stays good: RFC 8292 allows a day at most.
const TOKEN_FOR: i64 = 12 * 60 * 60;

/// The most a message may say: what fits in one record of the 4,096 bytes a push service takes.
const MOST: usize = 3_000;

/// How soon a push should reach its device (RFC 8030, 5.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Urgency {
    Normal,
    /// For what someone waits on.
    High,
}

/// One push: what it says, how long the push service keeps it for a device that is away, and
/// how soon it should reach it.
#[derive(Debug, Clone, Copy)]
pub struct Message<'a> {
    pub text: &'a [u8],
    pub keep: Duration,
    pub urgency: Urgency,
}

/// What the push service said of a push.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sent {
    /// It took it, to deliver.
    Taken,
    /// The subscription is no more: forget it.
    Gone,
}

/// Why a push was not sent.
#[derive(Debug, Error)]
pub enum PushError {
    #[error("the message is {0} bytes, more than a push carries")]
    TooLong(usize),
    #[error("the message could not be encrypted: {0}")]
    Encrypt(String),
    #[error("the push service could not be reached: {0}")]
    Unreached(String),
    #[error("the push service did not answer within {}s", SEND_WITHIN.as_secs())]
    TimedOut,
    #[error("the push service refused the push ({0})")]
    Refused(StatusCode),
    #[error("{0} is not a way to contact the server's operator: give mailto: or https:")]
    Contact(String),
}

/// What sends pushes: an HTTP client that reaches only push services, and whom the push
/// services may contact about them (VAPID's `sub`).
pub struct Pusher {
    client: Client,
    contact: String,
    /// Whether a test's own push service on this computer may be reached.
    here: bool,
}

impl fmt::Debug for Pusher {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Pusher")
            .field("contact", &self.contact)
            .finish_non_exhaustive()
    }
}

impl Pusher {
    /// Sends through push services on the internet, saying `contact` (a `mailto:` or `https:`
    /// URL) is who to reach about them.
    pub fn new(contact: &str) -> Result<Self, PushError> {
        Self::build(contact, false)
    }

    /// Sends to a push service on this computer too, for tests, which serve their own.
    pub fn reaching_this_computer(contact: &str) -> Result<Self, PushError> {
        Self::build(contact, true)
    }

    fn build(contact: &str, here: bool) -> Result<Self, PushError> {
        if !(contact.starts_with("mailto:") || contact.starts_with("https://")) {
            return Err(PushError::Contact(contact.to_owned()));
        }
        let client = Client::builder()
            .redirect(redirect::Policy::none())
            .timeout(SEND_WITHIN);
        let client = if here {
            client
        } else {
            client.no_proxy().dns_resolver(Arc::new(PublicOnly))
        };
        Ok(Self {
            client: client
                .build()
                .map_err(|error| PushError::Unreached(error.to_string()))?,
            contact: contact.to_owned(),
            here,
        })
    }

    /// The subscription a browser gave, when a push can go to it.
    pub fn subscription(
        &self,
        endpoint: &str,
        p256dh: &str,
        auth: &str,
    ) -> Result<Subscription, Refused> {
        Subscription::read(endpoint, p256dh, auth, self.here)
    }

    /// Sends `message` to `to`, signed with `key` as of `now`.
    pub async fn send(
        &self,
        key: &VapidKey,
        to: &Subscription,
        message: Message<'_>,
        now: Timestamp,
    ) -> Result<Sent, PushError> {
        if message.text.len() > MOST {
            return Err(PushError::TooLong(message.text.len()));
        }
        let endpoint = to
            .endpoint
            .as_str()
            .parse()
            .map_err(|_| PushError::Encrypt(format!("{} is not a URI", to.endpoint)))?;
        let request = WebPushBuilder::new(endpoint, to.key, to.secret)
            .with_valid_duration(message.keep)
            .build(message.text)
            .map_err(|error| PushError::Encrypt(error.to_string()))?;
        let audience = to.endpoint.origin().ascii_serialization();
        let authorization = key.authorization(
            &audience,
            now.as_second().saturating_add(TOKEN_FOR),
            &self.contact,
        );
        let urgency = match message.urgency {
            Urgency::Normal => "normal",
            Urgency::High => "high",
        };
        let request = reqwest::Request::try_from(request)
            .map_err(|error| PushError::Encrypt(error.to_string()))?;
        let sent = tokio::time::timeout(
            SEND_WITHIN,
            self.client
                .execute(with_headers(request, &authorization, urgency)?),
        )
        .await
        .map_err(|_| PushError::TimedOut)?
        .map_err(|error| PushError::Unreached(error.to_string()))?;
        outcome(sent.status())
    }
}

/// `request` with the token that signs it and how soon it should arrive.
fn with_headers(
    mut request: reqwest::Request,
    authorization: &str,
    urgency: &'static str,
) -> Result<reqwest::Request, PushError> {
    let authorization = authorization
        .parse()
        .map_err(|_| PushError::Encrypt("the token is not a header's value".to_owned()))?;
    let headers = request.headers_mut();
    headers.insert(reqwest::header::AUTHORIZATION, authorization);
    headers.insert(
        "urgency",
        reqwest::header::HeaderValue::from_static(urgency),
    );
    Ok(request)
}

/// What a push service's answer means.
fn outcome(status: StatusCode) -> Result<Sent, PushError> {
    if status.is_success() {
        Ok(Sent::Taken)
    } else if matches!(status, StatusCode::NOT_FOUND | StatusCode::GONE) {
        Ok(Sent::Gone)
    } else {
        Err(PushError::Refused(status))
    }
}

/// Looks a push service's name up with the system's resolver, keeping its public addresses.
struct PublicOnly;

impl Resolve for PublicOnly {
    fn resolve(&self, name: Name) -> Resolving {
        let host = name.as_str().to_owned();
        Box::pin(async move {
            let public: Vec<_> = tokio::net::lookup_host((host.as_str(), 0))
                .await?
                .filter(|found| reachable(found.ip(), false))
                .collect();
            if public.is_empty() {
                let said = format!("{host} has no public address a push service would have");
                return Err(said.into());
            }
            Ok(Box::new(public.into_iter()) as Addrs)
        })
    }
}

#[cfg(test)]
mod tests;
