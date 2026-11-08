//! Sending one push (RFC 8030): the message encrypted to the subscription (RFC 8291), posted
//! to its endpoint with the server's VAPID token, connected within [`CONNECT_WITHIN`] and
//! answered within [`SEND_WITHIN`], so a push service that hangs holds up no one for long. The
//! push service answers whether it took it, or that the subscription is gone (404 or 410), when
//! the browser unsubscribed or its person blocked notifications. Names are looked up keeping
//! only public addresses (glave's one [`Lookup`]), with no proxy and no redirect, so a push never
//! reaches the server's own network. Nothing here says an endpoint: it is the browser's address.

use std::fmt;
use std::sync::Arc;
use std::time::Duration;

use glave_ports::host::Lookup;
use jiff::Timestamp;
use reqwest::{Client, StatusCode, redirect};
use thiserror::Error;
use web_push_native::WebPushBuilder;

use crate::key::VapidKey;
use crate::subscription::{Refused, Subscription};

/// The longest a push service may take to accept the connection.
const CONNECT_WITHIN: Duration = Duration::from_secs(5);

/// The longest a push may take, from the name looked up to the push service's answer.
const SEND_WITHIN: Duration = Duration::from_secs(10);

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

impl PushError {
    /// Whether the push service put the failure down to the subscription itself: a 4xx answer
    /// but 408 (it timed out) and 429 (too many pushes). This server's own network, a timeout,
    /// the service's limits and its own troubles (5xx) never are, so an outage forgets no one.
    pub fn of_the_subscription(&self) -> bool {
        match self {
            Self::Refused(status) => {
                status.is_client_error()
                    && !matches!(
                        *status,
                        StatusCode::REQUEST_TIMEOUT | StatusCode::TOO_MANY_REQUESTS
                    )
            }
            _ => false,
        }
    }
}

/// A push service's name that leads only to addresses a push never reaches.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Private {
    host: String,
}

impl fmt::Display for Private {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} leads only to private addresses (this computer, a local network or cloud \
             metadata), which glave never pushes to",
            self.host
        )
    }
}

impl std::error::Error for Private {}

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
    /// Sends through the browsers' push services, saying `contact` (a `mailto:` or `https:`
    /// URL) is who to reach about them.
    pub fn new(contact: &str) -> Result<Self, PushError> {
        Self::build(contact, false)
    }

    /// Sends to a push service on this computer too, which tests serve their own.
    #[cfg(feature = "test-service")]
    pub fn reaching_this_computer(contact: &str) -> Result<Self, PushError> {
        Self::build(contact, true)
    }

    fn build(contact: &str, here: bool) -> Result<Self, PushError> {
        if !(contact.starts_with("mailto:") || contact.starts_with("https://")) {
            return Err(PushError::Contact(contact.to_owned()));
        }
        let client = Client::builder()
            .redirect(redirect::Policy::none())
            .connect_timeout(CONNECT_WITHIN)
            .timeout(SEND_WITHIN);
        let client = if here {
            client
        } else {
            let lookup = Lookup::new(
                |_| false,
                |host| Private {
                    host: host.to_owned(),
                },
            );
            client.no_proxy().dns_resolver(Arc::new(lookup))
        };
        Ok(Self {
            client: client.build().map_err(|error| unreached(&error))?,
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
            .map_err(|_| PushError::Encrypt("the endpoint is not a URI".to_owned()))?;
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
        let request = reqwest::Request::try_from(request).map_err(|error| unreached(&error))?;
        let sent = tokio::time::timeout(
            SEND_WITHIN,
            self.client
                .execute(with_headers(request, &authorization, urgency)?),
        )
        .await
        .map_err(|_| PushError::TimedOut)?
        .map_err(|error| unreached(&error))?;
        outcome(sent.status())
    }
}

/// `error` as a push that did not reach its service, without the endpoint it was sent to.
fn unreached(error: &reqwest::Error) -> PushError {
    if error.is_timeout() {
        return PushError::TimedOut;
    }
    let mut said = error.to_string();
    if let Some(url) = error.url() {
        said = said.replace(url.as_str(), "the push service");
    }
    PushError::Unreached(said)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_service_s_answer_says_whether_the_subscription_is_gone() {
        assert_eq!(outcome(StatusCode::CREATED).unwrap(), Sent::Taken);
        assert_eq!(outcome(StatusCode::GONE).unwrap(), Sent::Gone);
        assert_eq!(outcome(StatusCode::NOT_FOUND).unwrap(), Sent::Gone);
        assert!(matches!(
            outcome(StatusCode::FORBIDDEN),
            Err(PushError::Refused(StatusCode::FORBIDDEN))
        ));
        assert!(matches!(
            outcome(StatusCode::TOO_MANY_REQUESTS),
            Err(PushError::Refused(_))
        ));
    }

    #[test]
    fn only_a_4xx_but_408_and_429_is_the_subscription_s_own_failure() {
        let refused = |code: u16| PushError::Refused(StatusCode::from_u16(code).unwrap());
        for theirs in [400, 401, 403, 413] {
            assert!(refused(theirs).of_the_subscription(), "{theirs}");
        }
        for not in [408, 429, 500, 502, 503] {
            assert!(!refused(not).of_the_subscription(), "{not}");
        }
        assert!(!PushError::TimedOut.of_the_subscription());
        assert!(!PushError::Unreached("no route".into()).of_the_subscription());
    }

    #[test]
    fn a_name_that_leads_nowhere_public_is_never_pushed_to() {
        let refusal = Private {
            host: "evil.example".into(),
        };
        assert!(refusal.to_string().contains("never pushes to"));
    }
}
